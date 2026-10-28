use super::*;

#[test]
fn recovery_absence_is_unknown_and_ready_is_explicit() -> Result<(), serde_json::Error> {
    let older = serde_json::to_value(State::default())?;
    assert!(older.get("recovery").is_none());
    let restored: State = serde_json::from_value(older)?;
    assert_eq!(restored.views.recovery, RecoveryStatus::Unknown);

    let ready = State {
        views: crate::Views {
            recovery: RecoveryStatus::Ready,
            ..crate::Views::default()
        },
        ..State::default()
    };
    let served = serde_json::to_value(ready)?;
    assert_eq!(served["recovery"]["state"], "ready");
    Ok(())
}

#[test]
fn ephemeral_views_do_not_inflate_every_actor_frame() {
    // A resource budget, not a stable ABI. Large new presentation payloads
    // belong off-stack; raising the test-thread stack hides this regression.
    assert!(std::mem::size_of::<State>() <= 4096);
}

#[test]
fn a_snapshot_that_still_names_the_daemon_version_loads() -> Result<(), serde_json::Error> {
    // Up to 0.5.x the daemon stamped its build version on the state it
    // served, and a snapshot could keep it. One executable has one
    // version since the daemon runs inside the app: the field is gone,
    // and a snapshot that still carries it loads as if it never had.
    let mut saved = serde_json::to_value(DurableState::default())?;
    if let Some(fields) = saved.as_object_mut() {
        fields.insert("daemon_version".into(), "0.5.1".into());
    }
    let restored: DurableState = serde_json::from_value(saved)?;
    assert_eq!(restored, DurableState::default());
    assert!(
        serde_json::to_value(&restored)?
            .get("daemon_version")
            .is_none()
    );
    Ok(())
}

#[test]
fn the_ipc_state_says_whether_a_helper_is_connected() {
    // "Nobody has asked yet" and "there is no helper to ask" are only
    // distinguishable while the state says whether a
    // helper is connected at all.
    let connected = State {
        durable: crate::DurableState {
            capabilities: Capabilities {
                native_helper: true,
                ..Capabilities::default()
            },
            ..crate::DurableState::default()
        },
        ..State::default()
    };
    assert!(connected.helper_connected());
    let absent = State::default();
    assert!(!absent.helper_connected());
}

#[test]
fn snapshot_without_finalizing_since_ms_deserializes() -> Result<(), serde_json::Error> {
    let state = State {
        durable: crate::DurableState {
            capture: crate::transition::CaptureState {
                recording: Some(Recording {
                    id: "r".into(),
                    title: "Sesión".into(),
                    started_at_ms: 1_000,
                    ended_at_ms: Some(2_000),
                    expires_at_ms: 1_000_000,
                    application_pid: None,
                    audio: crate::protocol::MeetingAudio::MicrophoneOnly,
                    phase: CapturePhase::Finalizing,
                    sealed_chunks: 1,
                    gaps: 0,
                    tracks: Vec::new(),
                    finalizing_since_ms: Some(2_000),
                    ceiling_notice_given: false,
                    muted_spans: Vec::new(),
                }),
                ..crate::transition::CaptureState::default()
            },
            ..crate::DurableState::default()
        },
        ..State::default()
    };
    let mut value = serde_json::to_value(&state)?;
    // A snapshot written before the field existed must keep loading.
    let removed = value
        .pointer_mut("/recording")
        .and_then(serde_json::Value::as_object_mut)
        .map(|recording| recording.remove("finalizing_since_ms").is_some());
    assert_eq!(removed, Some(true));
    let restored: State = serde_json::from_value(value)?;
    assert_eq!(
        restored
            .durable
            .capture
            .recording
            .and_then(|recording| recording.finalizing_since_ms),
        None
    );
    Ok(())
}
