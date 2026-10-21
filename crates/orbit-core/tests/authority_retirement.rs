use orbit_core::{
    Connection, Event, State,
    authority::{AuthorityChange, RetireSlot, retire, retire_stopped_decision},
    decide,
};

#[test]
fn stop_and_close_set_the_connection_offline_in_the_decision()
-> Result<(), Box<dyn std::error::Error>> {
    for event in [
        Event::PrivacyStop,
        Event::Close {
            cause: "cierre".into(),
        },
    ] {
        let state = decide(&State::default(), Event::Connect(Connection::Online), 0)?.state;
        let decision = decide(&state, event, 1)?;
        assert_eq!(decision.state.connection(), Connection::Offline);
    }
    Ok(())
}

#[test]
fn a_lost_hub_closes_the_turn_in_the_same_retirement_decision()
-> Result<(), Box<dyn std::error::Error>> {
    let mut state = decide(&State::default(), Event::Connect(Connection::Online), 0)?.state;
    state.durable.active_request = Some("pending".into());
    let decision = decide(&state, Event::HubDisconnected, 1)?;
    assert_eq!(decision.state.connection(), Connection::Offline);
    assert!(decision.state.durable.active_request.is_none());
    assert_eq!(
        decision.state.notice_code(),
        Some(orbit_core::notice::NoticeCode::AssistantTurnLostByDisconnect)
    );
    assert_eq!(
        decision.authority_change,
        Some(AuthorityChange::HubDisconnected)
    );
    Ok(())
}

#[test]
fn every_authority_change_has_the_agreed_slots() {
    use RetireSlot::*;
    let full = [
        Calendar,
        Bridge,
        CallDetection,
        Computer,
        Sources,
        VoiceTranscription,
        CallTurn,
        Playback,
        Upload,
        RecordingNotes,
        InFlightReads,
        HubLink,
    ];
    let table: &[(AuthorityChange, &[RetireSlot])] = &[
        (AuthorityChange::MarsStop, &full),
        (AuthorityChange::Revoked, &full),
        (AuthorityChange::Closing, &full),
        (AuthorityChange::Rearm, &[InFlightReads, Computer, Bridge]),
        (
            AuthorityChange::RearmWithoutStop,
            &[InFlightReads, Computer],
        ),
        (AuthorityChange::HubLink, &[]),
        (AuthorityChange::HubTransportReady, &[]),
        (AuthorityChange::HubLinkOffline, &[]),
        (AuthorityChange::HubDisconnected, &[Bridge]),
        (
            AuthorityChange::Conversation,
            &[VoiceTranscription, Sources],
        ),
        (
            AuthorityChange::Interrupt,
            &[Computer, Sources, VoiceTranscription, CallTurn],
        ),
        (
            AuthorityChange::Device,
            &[InFlightReads, VoiceTranscription, RecordingNotes, HubLink],
        ),
    ];
    for (change, expected) in table {
        assert_eq!(retire(*change).slots, *expected, "{change:?}");
    }
    assert_eq!(
        retire(AuthorityChange::Revoked),
        retire(AuthorityChange::MarsStop)
    );
    for change in [
        AuthorityChange::MarsStop,
        AuthorityChange::Revoked,
        AuthorityChange::Closing,
    ] {
        assert_eq!(retire(change).durable_slots, &[Computer, Sources]);
    }
    for change in [
        AuthorityChange::Rearm,
        AuthorityChange::RearmWithoutStop,
        AuthorityChange::HubLink,
        AuthorityChange::HubTransportReady,
        AuthorityChange::HubLinkOffline,
        AuthorityChange::HubDisconnected,
        AuthorityChange::Conversation,
        AuthorityChange::Interrupt,
        AuthorityChange::Device,
    ] {
        assert!(retire(change).durable_slots.is_empty(), "{change:?}");
    }
    for change in [AuthorityChange::HubLink, AuthorityChange::Conversation] {
        assert!(!retire(change).slots.contains(&Computer), "{change:?}");
    }
    assert_eq!(retire_stopped_decision().slots, &[CallDetection, Computer]);
    // REF-303b A5: the calendar refusal reaches Deimos before the bridge
    // release can consume the ask it answers.
    for change in [
        AuthorityChange::MarsStop,
        AuthorityChange::Revoked,
        AuthorityChange::Closing,
    ] {
        let slots = retire(change).slots;
        let Some(calendar) = slots.iter().position(|slot| *slot == Calendar) else {
            panic!("{change:?}: calendar slot missing");
        };
        let Some(bridge) = slots.iter().position(|slot| *slot == Bridge) else {
            panic!("{change:?}: bridge slot missing");
        };
        assert!(calendar < bridge, "{change:?}");
    }
}

#[test]
fn rearm_after_a_stop_retires_every_generation_bound_slot() -> Result<(), Box<dyn std::error::Error>>
{
    let state = decide(&State::default(), Event::PrivacyStop, 0)?.state;
    let decision = decide(&state, Event::LocalRearm, 1)?;
    assert!(decision.state.generation() > state.generation());
    let slots = retire(decision.authority_change.ok_or("missing rearm plan")?).slots;
    assert!(
        slots.contains(&RetireSlot::Bridge),
        "rearm left the bridge alive: {slots:?}"
    );
    Ok(())
}

#[test]
fn redundant_rearm_keeps_local_verticals_and_cached_reads() -> Result<(), Box<dyn std::error::Error>>
{
    let state = State::default();
    let decision = decide(&state, Event::LocalRearm, 1)?;
    assert_eq!(
        decision.authority_change,
        Some(AuthorityChange::RearmWithoutStop)
    );
    assert_eq!(
        retire(AuthorityChange::RearmWithoutStop).slots,
        &[RetireSlot::InFlightReads, RetireSlot::Computer]
    );
    Ok(())
}

#[test]
fn link_ready_loss_and_offline_have_distinct_retirements() -> Result<(), Box<dyn std::error::Error>>
{
    let online = decide(&State::default(), Event::Connect(Connection::Online), 0)?.state;
    let lost = decide(&online, Event::HubDisconnected, 1)?;
    assert_eq!(lost.state.connection(), Connection::Offline);
    assert_eq!(
        lost.authority_change,
        Some(AuthorityChange::HubDisconnected)
    );
    let ready = decide(&lost.state, Event::Connect(Connection::Online), 2)?;
    assert_eq!(ready.authority_change, Some(AuthorityChange::HubLink));
    let offline = decide(&online, Event::Connect(Connection::Offline), 3)?;
    assert_eq!(
        offline.authority_change,
        Some(AuthorityChange::HubLinkOffline)
    );
    Ok(())
}

#[test]
fn naming_a_first_session_is_not_a_conversation_retirement()
-> Result<(), Box<dyn std::error::Error>> {
    let state = State::default();
    let named = decide(&state, Event::Session("first".into()), 1)?;
    assert_eq!(named.authority_change, None);
    let changed = decide(&named.state, Event::Session("second".into()), 2)?;
    assert_eq!(
        changed.authority_change,
        Some(AuthorityChange::Conversation)
    );
    Ok(())
}

#[test]
fn closing_does_not_latch_privacy() -> Result<(), Box<dyn std::error::Error>> {
    let state = decide(&State::default(), Event::Connect(Connection::Online), 0)?.state;
    let closing = decide(
        &state,
        Event::Close {
            cause: "cierre".into(),
        },
        1,
    )?;
    assert_eq!(closing.authority_change, Some(AuthorityChange::Closing));
    assert!(!closing.state.privacy_stopped());
    Ok(())
}

#[test]
fn explicit_conversation_events_clear_the_turn_and_pending_approval()
-> Result<(), Box<dyn std::error::Error>> {
    let mut state = decide(&State::default(), Event::Session("before".into()), 0)?.state;
    state.durable.turn = Some(orbit_core::Turn {
        user: orbit_core::Message {
            id: "message".into(),
            role: "user".into(),
            text: "fixture".into(),
            complete: true,
            at_ms: 1,
            voice: None,
        },
        answer: None,
    });
    state.durable.pending_approval = Some(orbit_core::Approval {
        request_id: "approval".into(),
        description: "fixture".into(),
        clarification: false,
    });
    for (event, expected) in [
        (Event::NewSession, None),
        (Event::OpenSession("saved".into()), Some("saved")),
    ] {
        let decision = decide(&state, event, 2)?;
        assert_eq!(
            decision.authority_change,
            Some(AuthorityChange::Conversation)
        );
        assert_eq!(decision.state.session_id().as_deref(), expected);
        assert!(decision.state.durable.turn.is_none());
        assert!(decision.state.durable.pending_approval.is_none());
    }
    Ok(())
}
