//! Call detection, through the daemon's own paths: a synthetic helper
//! (`microphone_activity`, the agenda read, the audio starts) and the
//! detection's clock driven by the test. Nothing native, no real call.
use super::*;
use crate::daemon::{
    audio::tests::fixture,
    calendar::tests::{ack, host_state, next_command, page, seed, status},
};
use orbit_core::topics::recording::CallPanel as TopicPanel;
use tokio::sync::mpsc;

/// 1 October 2026, 10:00 UTC: a call in the morning.
const NOW: i64 = 1_790_848_800_000;
const MINUTE: i64 = 60_000;

fn zoom_holding(holding: bool) -> Value {
    let applications = if holding {
        json!([{"application_pid": 4242, "bundle_id": "us.zoom.xos", "name": "zoom.us"}])
    } else {
        json!([])
    };
    json!({"event": "microphone_activity", "v": 1, "applications": applications})
}

fn invitee(name: &str, me: bool) -> Value {
    json!({"name": name, "participant_url": format!("mailto:{name}@example.com"),
        "email": format!("{name}@example.com"), "role": "required",
        "response_status": "accepted", "participant_type": "person",
        "is_current_user": me})
}

/// The committee on now, with its Zoom link and two guests.
fn committee() -> Value {
    json!({
        "observation_id": "55555555-5555-4555-8555-000000000001",
        "calendar": {"calendar_id": "calendar-0000", "source_id": "account"},
        "calendar_item_id": "committee-series",
        "start_unix_ms": NOW - 5 * MINUTE,
        "end_unix_ms": NOW + 25 * MINUTE,
        "interpreted_timezone": "UTC",
        "is_floating": true,
        "is_all_day": false,
        "is_detached": false,
        "title": "Comité de producto",
        "status": "confirmed",
        "invitees": [invitee("Alina", true), invitee("Lucía", false)],
        "invitees_status": "provided",
        "event_url": "https://acme.zoom.us/j/42",
        "event_url_status": "provided",
    })
}

async fn at(runtime: &mut Runtime, now_ms: i64) -> Result<()> {
    runtime.features.call.clock_ms = Some(now_ms);
    runtime.maintain_call_detection().await
}

fn panel(runtime: &Runtime) -> Option<TopicPanel> {
    runtime.call_topic().panel
}

/// Acknowledges the capture starts the daemon sent, and returns them.
async fn ack_starts(
    runtime: &mut Runtime,
    helper: &mut mpsc::Receiver<Value>,
) -> Result<Vec<Value>> {
    let mut starts = Vec::new();
    while let Ok(command) = helper.try_recv() {
        if command["command"] == "audio_start" {
            runtime
                .native_event(json!({"event":"response","id":command["id"],"ok":true,
                    "result":{"stream_id":command["stream_id"]}}))
                .await?;
            starts.push(command);
        }
    }
    Ok(starts)
}

#[tokio::test]
async fn a_call_is_offered_recorded_with_its_event_and_ends_after_the_countdown() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut helper) = fixture(directory.path())?;
    // The detection's clock is the test's from the start: it never runs
    // backwards, so a wall-clock reading taken first would hold it there.
    runtime.features.call.clock_ms = Some(NOW);
    seed(&mut runtime, &mut helper).await?;
    runtime.recover_call_preference();

    // Zoom takes the microphone: the agenda around now is read at once.
    runtime.native_event(zoom_holding(true)).await?;
    let read = next_command(&mut helper).await?;
    assert_eq!(read["command"], "calendar_snapshot");
    assert_eq!(read["calendar_query"]["start_unix_ms"], NOW);
    assert_eq!(
        read["calendar_query"]["end_unix_ms"],
        NOW + orbit_core::call_detection::EVENT_LOOKAHEAD_MS + 1
    );
    ack(
        &mut runtime,
        &read,
        page(
            &read["calendar_query"],
            vec![committee()],
            1,
            "44444444-4444-4444-8444-444444444444",
        ),
    )
    .await?;
    assert!(
        runtime.features.call.events.is_some(),
        "the agenda read for the call arrived"
    );
    at(&mut runtime, NOW + 2_999).await?;
    assert_eq!(panel(&runtime), None, "not a call before three seconds");

    // Three seconds later: the offer, with the event's title.
    at(&mut runtime, NOW + 3_000).await?;
    let topic = runtime.call_topic();
    assert_eq!(topic.preference, Some(CallPreference::Notify));
    let Some(TopicPanel::Offer {
        call_id,
        application,
        event_title,
    }) = topic.panel
    else {
        bail!("no offer after three seconds: {:?}", topic.panel);
    };
    assert_eq!(application, "zoom.us");
    assert_eq!(event_title.as_deref(), Some("Comité de producto"));
    assert!(
        runtime.state.durable.recording.is_none(),
        "«Avisarme» never records alone"
    );

    // «Grabar»: a recording linked to the event, capturing Zoom's audio.
    runtime.command(Command::CallRecord { call_id }).await?;
    let starts = ack_starts(&mut runtime, &mut helper).await?;
    let remote = starts
        .iter()
        .find(|start| start["source"] == "remote_app")
        .context("the call's audio was not asked for")?;
    assert_eq!(remote["application_pid"], 4242);
    assert_eq!(remote["capture_scope"], "application");
    let recording = runtime
        .state
        .durable
        .recording
        .clone()
        .context("no recording started")?;
    assert_eq!(recording.title, "Comité de producto");
    assert_eq!(recording.phase, CapturePhase::Recording);
    let manifest = runtime
        .reader
        .get_job(&format!("recording-{}", recording.id))?
        .context("no manifest")?;
    let event = &manifest.payload["event"];
    assert_eq!(event["calendar_item_id"], "committee-series");
    assert_eq!(event["occurrence_start_ms"], NOW - 5 * MINUTE);
    assert_eq!(event["attendees"][1]["name"], "Lucía");
    assert!(
        event.get("observation_id").is_none(),
        "the link is the stable reference, never one read's observation"
    );
    assert_eq!(manifest.payload["origin"], "call_detection");
    assert_eq!(
        panel(&runtime),
        None,
        "the pill shows an accepted recording"
    );

    // Zoom leaves the microphone at one minute: three seconds later the
    // countdown starts, thirteen seconds after that the recording ends.
    runtime.features.call.clock_ms = Some(NOW + MINUTE);
    runtime.native_event(zoom_holding(false)).await?;
    at(&mut runtime, NOW + MINUTE + 2_999).await?;
    assert_eq!(panel(&runtime), None);
    at(&mut runtime, NOW + MINUTE + 3_000).await?;
    assert_eq!(
        panel(&runtime),
        Some(TopicPanel::Ending {
            recording_id: recording.id.clone(),
            application: "zoom.us".into(),
            ends_at_ms: NOW + MINUTE + 16_000,
        })
    );
    at(&mut runtime, NOW + MINUTE + 15_999).await?;
    assert_eq!(
        runtime
            .state
            .durable
            .recording
            .as_ref()
            .map(|r| r.phase.clone()),
        Some(CapturePhase::Recording)
    );
    at(&mut runtime, NOW + MINUTE + 16_000).await?;
    assert_eq!(
        runtime
            .state
            .durable
            .recording
            .as_ref()
            .map(|r| r.phase.clone()),
        Some(CapturePhase::Finalizing),
        "ended as «Terminar» would"
    );
    let mut stops = 0;
    while let Ok(command) = helper.try_recv() {
        if command["command"] == "audio_stop" {
            stops += 1;
        }
    }
    assert_eq!(stops, 2, "both tracks are stopped and drained");
    assert_eq!(panel(&runtime), None);
    Ok(())
}

#[tokio::test]
async fn record_without_asking_starts_alone_and_keep_recording_makes_it_manual() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut helper) = fixture(directory.path())?;
    runtime.recover_call_preference();
    runtime
        .command(Command::CallPreference {
            preference: CallPreference::Record,
        })
        .await?;
    // The choice is kept by the daemon, encrypted, across a restart.
    assert_eq!(
        runtime.reader.load_call_preference()?,
        Some(CallPreference::Record)
    );
    runtime.features.call.clock_ms = Some(NOW);
    runtime.native_event(zoom_holding(true)).await?;
    at(&mut runtime, NOW + 3_000).await?;
    ack_starts(&mut runtime, &mut helper).await?;
    let recording = runtime
        .state
        .durable
        .recording
        .clone()
        .context("«Grabar sin preguntar» did not start")?;
    assert_eq!(recording.title, "Llamada en zoom.us", "no agenda: no event");
    assert!(matches!(
        panel(&runtime),
        Some(TopicPanel::Recording { ref recording_id, .. }) if recording_id == &recording.id
    ));
    runtime.features.call.clock_ms = Some(NOW + MINUTE);
    runtime.native_event(zoom_holding(false)).await?;
    at(&mut runtime, NOW + MINUTE + 3_000).await?;
    assert!(matches!(panel(&runtime), Some(TopicPanel::Ending { .. })));
    runtime
        .command(Command::RecordingKeep {
            recording_id: recording.id.clone(),
        })
        .await?;
    at(&mut runtime, NOW + 10 * MINUTE).await?;
    assert_eq!(
        runtime
            .state
            .durable
            .recording
            .as_ref()
            .map(|r| r.phase.clone()),
        Some(CapturePhase::Recording),
        "a kept recording is manual: it never ends on its own"
    );
    assert_eq!(panel(&runtime), None);
    Ok(())
}

#[tokio::test]
async fn the_call_app_closing_during_the_countdown_ends_with_what_was_recorded() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut helper) = fixture(directory.path())?;
    runtime.recover_call_preference();
    runtime
        .command(Command::CallPreference {
            preference: CallPreference::Record,
        })
        .await?;
    runtime.features.call.clock_ms = Some(NOW);
    runtime.native_event(zoom_holding(true)).await?;
    at(&mut runtime, NOW + 3_000).await?;
    ack_starts(&mut runtime, &mut helper).await?;
    let recording = runtime
        .state
        .durable
        .recording
        .clone()
        .context("not started")?;
    runtime.features.call.clock_ms = Some(NOW + MINUTE);
    runtime.native_event(zoom_holding(false)).await?;
    at(&mut runtime, NOW + MINUTE + 4_000).await?;
    // Zoom quits: ScreenCaptureKit stops its stream and the helper cuts.
    runtime
        .native_event(
            json!({"event":"capture_stopped","reason":"system_audio_stream_stopped",
            "privacy_stopped":false,"drain_complete":true}),
        )
        .await?;
    let closed = runtime
        .state
        .durable
        .recording
        .clone()
        .context("recording gone")?;
    assert_eq!(closed.id, recording.id);
    assert!(
        closed.ended_at_ms.is_some(),
        "the recording ends as «Terminar», not interrupted: {:?}",
        closed.phase
    );
    assert_eq!(
        runtime.state.notice_code(),
        Some(orbit_core::notice::NoticeCode::CallRecordingFinished),
        "{:?}",
        runtime.state.durable.notice
    );
    assert!(crate::daemon::turn::said::mentions(
        &runtime.state,
        "call_app_closed"
    ));
    Ok(())
}

#[tokio::test]
async fn nothing_is_offered_before_the_helper_says_who_holds_the_microphone() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut helper) = fixture(directory.path())?;
    runtime.recover_call_preference();
    runtime
        .command(Command::CallPreference {
            preference: CallPreference::Record,
        })
        .await?;
    for second in 0..30 {
        at(&mut runtime, NOW + second * 1_000).await?;
    }
    assert!(runtime.state.durable.recording.is_none());
    assert_eq!(panel(&runtime), None);
    assert!(ack_starts(&mut runtime, &mut helper).await?.is_empty());
    Ok(())
}

#[tokio::test]
async fn privacy_stop_hides_the_offer_and_rearming_resumes_nothing() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut helper) = fixture(directory.path())?;
    // The detection's clock is the test's from the start: it never runs
    // backwards, so a wall-clock reading taken first would hold it there.
    runtime.features.call.clock_ms = Some(NOW);
    seed(&mut runtime, &mut helper).await?;
    runtime.recover_call_preference();
    runtime.native_event(zoom_holding(true)).await?;
    at(&mut runtime, NOW + 3_000).await?;
    assert!(matches!(panel(&runtime), Some(TopicPanel::Offer { .. })));
    runtime.command(Command::PrivacyStop).await?;
    assert_eq!(panel(&runtime), None);
    // The helper latches, drains and answers.
    let mut stop = None;
    while let Ok(command) = helper.try_recv() {
        if command["command"] == "privacy_stop" {
            stop = Some(command);
        }
    }
    let stop = stop.context("the helper was not told to stop")?;
    runtime
        .native_event(json!({"event":"response", "id":stop["id"], "ok":true,
            "result":{"privacy_stopped":true,"drain_complete":true}}))
        .await?;
    assert!(
        runtime
            .command(Command::CallRecord {
                call_id: "call-anything".into()
            })
            .await
            .is_err()
    );
    // The helper rearms and sends the whole list again: Zoom was there.
    runtime
        .native_event(json!({"event":"local_rearm","privacy_stopped":false}))
        .await?;
    runtime
        .native_event(host_state(2, false, status("full_access")))
        .await?;
    runtime.features.call.clock_ms = Some(NOW + MINUTE);
    runtime.native_event(zoom_holding(true)).await?;
    at(&mut runtime, NOW + 2 * MINUTE).await?;
    assert_eq!(panel(&runtime), None, "nothing resumes on its own");
    Ok(())
}

/// Daemon side: «Grabar» on a calendar event during a
/// call links the recording to the event and captures the call's audio.
#[tokio::test]
async fn record_on_an_event_during_a_call_captures_the_call_and_links_the_event() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut helper) = fixture(directory.path())?;
    runtime.features.call.clock_ms = Some(NOW);
    runtime.recover_call_preference();
    runtime
        .command(Command::CallPreference {
            preference: CallPreference::Nothing,
        })
        .await?;
    runtime.native_event(zoom_holding(true)).await?;
    let event: orbit_core::call_detection::LinkedEvent = serde_json::from_value(json!({
        "calendar_item_id": "committee-series", "occurrence_start_ms": NOW - 5 * MINUTE,
        "title": "Comité de producto", "starts_at_ms": NOW - 5 * MINUTE,
        "ends_at_ms": NOW + 25 * MINUTE,
        "attendees": [{"name": "Lucía", "email": "lucia@example.com", "is_self": false}]
    }))?;
    runtime
        .command(Command::MeetingStart {
            title: "Comité de producto".into(),
            audio: orbit_core::protocol::MeetingAudio::System,
            duration_minutes: None,
            event: Some(event),
        })
        .await?;
    let starts = ack_starts(&mut runtime, &mut helper).await?;
    let remote = starts
        .iter()
        .find(|start| start["source"] == "remote_app")
        .context("no call audio")?;
    assert_eq!(remote["capture_scope"], "application");
    assert_eq!(remote["application_pid"], 4242);
    let recording = runtime
        .state
        .durable
        .recording
        .clone()
        .context("not started")?;
    let manifest = runtime
        .reader
        .get_job(&format!("recording-{}", recording.id))?
        .context("no manifest")?;
    assert_eq!(
        manifest.payload["event"]["calendar_item_id"],
        "committee-series"
    );
    assert_eq!(manifest.payload["origin"], "explicit");
    // Started by hand: the call ending never ends it.
    runtime.features.call.clock_ms = Some(NOW + MINUTE);
    runtime.native_event(zoom_holding(false)).await?;
    at(&mut runtime, NOW + 10 * MINUTE).await?;
    assert_eq!(
        runtime
            .state
            .durable
            .recording
            .as_ref()
            .map(|r| r.phase.clone()),
        Some(CapturePhase::Recording)
    );
    Ok(())
}

/// A recording already going (started by hand) answers any call:
/// Zoom taking the microphone offers nothing and starts nothing, whatever
/// the preference.
#[tokio::test]
async fn a_call_during_a_manual_recording_offers_and_starts_nothing() -> Result<()> {
    for preference in [CallPreference::Notify, CallPreference::Record] {
        let directory = tempfile::tempdir()?;
        let (mut runtime, mut helper) = fixture(directory.path())?;
        runtime.features.call.clock_ms = Some(NOW);
        runtime.recover_call_preference();
        runtime
            .command(Command::CallPreference { preference })
            .await?;
        runtime
            .command(Command::MeetingStart {
                title: "Reunión a mano".into(),
                audio: orbit_core::protocol::MeetingAudio::MicrophoneOnly,
                duration_minutes: None,
                event: None,
            })
            .await?;
        let manual = ack_starts(&mut runtime, &mut helper).await?;
        assert_eq!(manual.len(), 1, "{preference:?}: the manual microphone");
        runtime.native_event(zoom_holding(true)).await?;
        for second in 0..=10 {
            at(&mut runtime, NOW + second * 1_000).await?;
        }
        assert_eq!(panel(&runtime), None, "{preference:?}: no offer");
        let mut starts = 0;
        while let Ok(command) = helper.try_recv() {
            if command["command"] == "audio_start" {
                starts += 1;
            }
        }
        assert_eq!(starts, 0, "{preference:?}: no second capture");
        assert_eq!(
            runtime
                .state
                .durable
                .recording
                .as_ref()
                .map(|r| r.title.as_str()),
            Some("Reunión a mano")
        );
    }
    Ok(())
}

/// The call's application quits and the helper, which no longer cuts the
/// microphone with it, declares the application's track stopped. The call
/// recording still ends as «Terminar» with what was recorded.
#[tokio::test]
async fn the_call_app_closing_with_its_track_declared_stopped_ends_the_recording() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut helper) = fixture(directory.path())?;
    runtime.recover_call_preference();
    runtime
        .command(Command::CallPreference {
            preference: CallPreference::Record,
        })
        .await?;
    runtime.features.call.clock_ms = Some(NOW);
    runtime.native_event(zoom_holding(true)).await?;
    at(&mut runtime, NOW + 3_000).await?;
    let starts = ack_starts(&mut runtime, &mut helper).await?;
    let remote = starts
        .iter()
        .find(|start| start["source"] == "remote_app")
        .and_then(|start| start["stream_id"].as_str())
        .context("the call records the application's audio")?
        .to_owned();
    let recording = runtime
        .state
        .durable
        .recording
        .clone()
        .context("not started")?;
    runtime.features.call.clock_ms = Some(NOW + MINUTE);
    runtime.native_event(zoom_holding(false)).await?;
    at(&mut runtime, NOW + MINUTE + 4_000).await?;
    runtime
        .native_event(
            json!({"event":"gap","source":"remote_app","stream_id":remote,
            "reason":"track_stopped","frames":0,"duration_unknown":true}),
        )
        .await?;
    let closed = runtime
        .state
        .durable
        .recording
        .clone()
        .context("recording gone")?;
    assert_eq!(closed.id, recording.id);
    // The microphone was never cut: the recording finishes as «Terminar»
    // does, draining what the Mac still holds, never interrupted.
    assert_eq!(closed.phase, CapturePhase::Finalizing);
    assert!(crate::daemon::turn::said::mentions(
        &runtime.state,
        "call_app_closed"
    ));
    Ok(())
}
