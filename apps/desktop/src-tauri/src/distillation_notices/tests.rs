use super::*;
use std::error::Error;

fn proposal(state: &str) -> Result<Distillation, serde_json::Error> {
    serde_json::from_value(serde_json::json!({
        "id":"cedar", "meeting_id":"meeting", "source_kind":"meeting",
        "created_at":"2026-10-30T11:52:00Z", "revision":0, "status":state,
        "changes":[],"conflicts":[],"quotes":[],"new_source":null,"error":null,
        "changed_paths":[],"skipped":[],"can_prepare_again":true,
    }))
}

#[test]
fn startup_reads_once_without_replaying_old_ready_proposals() -> Result<(), Box<dyn Error>> {
    let mut watcher = Watcher::default();
    assert!(watcher.take_poll(false));
    assert_eq!(watcher.observe(&[proposal("proposed")?]), 0);
    assert!(!watcher.take_poll(false));
    Ok(())
}

#[test]
fn an_empty_startup_page_stops_all_proposal_reads() {
    let mut watcher = Watcher::default();
    assert!(watcher.take_poll(false));
    watcher.observe(&[]);
    assert!(!watcher.take_poll(false));
}

#[test]
fn a_failed_startup_read_does_not_invent_preparation() {
    let mut watcher = Watcher::default();
    assert!(watcher.take_poll(false));
    // No page arrives: no hub, no credentials or a transport failure.
    assert!(!watcher.take_poll(false));
}

#[test]
fn polling_lasts_only_while_preparation_is_in_progress() -> Result<(), Box<dyn Error>> {
    let mut watcher = Watcher::default();
    let mut old = proposal("proposed")?;
    old.id = "cedar-old".into();
    assert!(watcher.take_poll(false));
    assert_eq!(watcher.observe(&[old.clone(), proposal("queued")?]), 0);
    assert!(watcher.take_poll(false));
    assert!(
        watcher.take_poll(false),
        "a failed read does not erase known queued work"
    );
    assert_eq!(watcher.observe(&[old.clone(), proposal("preparing")?]), 0);
    assert!(watcher.take_poll(false));
    assert_eq!(watcher.observe(&[old, proposal("proposed")?]), 1);
    assert!(!watcher.take_poll(false));
    Ok(())
}

#[test]
fn a_new_ready_note_frame_wakes_once_even_without_a_window() -> Result<(), Box<dyn Error>> {
    let handle = Handle::new(3);
    assert!(handle.change(|w| w.take_poll(false)));
    handle.change(|w| w.observe(&[]));
    let app = tauri::test::mock_builder()
        .manage(handle.clone())
        .build(tauri::test::mock_context(tauri::test::noop_assets()))?;
    let mut notes = orbit_app_core::AppSnapshot::default()
        .notes
        .ok_or("notes absent")?;
    notes.ready = Some(orbit_app_core::NoteReadySnapshot {
        sequence: 3,
        recording_id: "cedar-meeting".into(),
        title: "Cedar review".into(),
    });
    follow_notes(app.handle(), &Some(notes.clone()));
    assert!(
        !handle.change(|w| w.take_poll(false)),
        "startup sequence is old news"
    );
    let ready = notes.ready.as_mut().ok_or("ready absent")?;
    ready.sequence = 4;
    follow_notes(app.handle(), &Some(notes.clone()));
    assert!(handle.change(|w| w.take_poll(false)));
    let mut listed = proposal("proposed")?;
    listed.meeting_id = "cedar-meeting".into();
    handle.change(|w| w.observe(&[listed]));
    follow_notes(app.handle(), &None);
    follow_notes(app.handle(), &Some(notes));
    assert!(
        !handle.change(|w| w.take_poll(false)),
        "absent frames do not reset the sequence"
    );
    Ok(())
}

#[test]
fn only_the_two_explicit_preparation_calls_restart_the_watcher() -> Result<(), Box<dyn Error>> {
    let handle = Handle::new(0);
    assert!(handle.change(|w| w.take_poll(false)));
    handle.change(|w| w.observe(&[]));
    let app = tauri::test::mock_builder()
        .manage(handle.clone())
        .build(tauri::test::mock_context(tauri::test::noop_assets()))?;
    for (wire, expected) in [
        (
            serde_json::json!({"call":"prepare_distillation","meeting_id":"cedar-meeting"}),
            true,
        ),
        (
            serde_json::json!({"call":"distillation_action","id":"cedar","action":"prepare"}),
            true,
        ),
        (
            serde_json::json!({"call":"distillation_action","id":"cedar","action":"apply"}),
            false,
        ),
        (
            serde_json::json!({"call":"distillation_action","id":"cedar","action":"undo"}),
            false,
        ),
        (
            serde_json::json!({"call":"distillation_action","id":"cedar","action":"discard"}),
            false,
        ),
        (
            serde_json::json!({"call":"distillation","id":"cedar"}),
            false,
        ),
        (
            serde_json::json!({"call":"list_distillations","meeting_id":null,"page":1}),
            false,
        ),
        (serde_json::json!({"call":"config"}), false),
    ] {
        preparation_requested(app.handle(), &serde_json::from_value(wire)?);
        assert_eq!(handle.change(|w| w.take_poll(false)), expected);
        handle.change(|w| w.observe(&[]));
        assert!(!handle.change(|w| w.take_poll(false)));
    }
    Ok(())
}

#[test]
fn privacy_stop_skips_the_read_and_leaves_no_timer_until_a_new_reason() -> Result<(), Box<dyn Error>>
{
    let mut watcher = Watcher::default();
    assert!(!watcher.take_poll(true));
    assert!(!watcher.take_poll(false));
    assert!(watcher.note_ready(1, "meeting"));
    assert!(watcher.take_poll(false));
    watcher.observe(&[proposal("queued")?]);
    assert!(!watcher.take_poll(true));
    assert!(!watcher.take_poll(false));
    assert!(watcher.note_ready(2, "meeting"));
    assert!(watcher.take_poll(false));
    Ok(())
}

#[test]
fn a_proposal_notifies_once_across_later_triggers() -> Result<(), Box<dyn Error>> {
    let mut watcher = Watcher::default();
    assert!(watcher.take_poll(false));
    assert_eq!(watcher.observe(&[proposal("proposed")?]), 0);
    let mut new = proposal("proposed")?;
    new.id = "cedar-new".into();
    assert!(watcher.note_ready(1, "meeting"));
    assert!(watcher.take_poll(false));
    assert_eq!(watcher.observe(&[new.clone(), proposal("proposed")?]), 1);
    assert!(!watcher.take_poll(false));
    assert!(watcher.note_ready(2, "meeting"));
    assert!(watcher.take_poll(false));
    assert_eq!(watcher.observe(&[new, proposal("proposed")?]), 0);
    assert!(!watcher.take_poll(false));
    Ok(())
}

#[test]
fn a_trigger_while_reading_survives_an_empty_response() -> Result<(), Box<dyn Error>> {
    let mut watcher = Watcher::default();
    assert!(watcher.take_poll(false));
    assert!(watcher.note_ready(1, "meeting"));
    watcher.observe(&[]);
    assert!(
        watcher.take_poll(false),
        "the newer reason was not answered by this read"
    );
    watcher.observe(&[proposal("failed")?]);
    assert!(!watcher.take_poll(false));
    Ok(())
}

#[test]
fn all_terminal_states_stop_polling() -> Result<(), Box<dyn Error>> {
    for status in [
        "failed",
        "stale",
        "applying",
        "applied",
        "discarded",
        "undoing",
        "undone",
        "recovery_required",
    ] {
        let mut watcher = Watcher::default();
        assert!(watcher.take_poll(false));
        watcher.observe(&[proposal(status)?]);
        assert!(!watcher.take_poll(false), "{status}");
    }
    Ok(())
}

#[test]
fn a_note_ready_before_its_proposal_reads_until_the_proposal_is_there() -> Result<(), Box<dyn Error>>
{
    let mut watcher = Watcher::default();
    assert!(watcher.take_poll(false));
    watcher.observe(&[]);
    assert!(watcher.note_ready(1, "meeting"));
    assert!(watcher.take_poll(false));
    // The hub stored the note and has not made the meeting's proposal yet.
    assert_eq!(watcher.observe(&[]), 0);
    assert!(
        watcher.take_poll(false),
        "the ready note's proposal is still to come"
    );
    assert_eq!(watcher.observe(&[proposal("proposed")?]), 1);
    assert!(!watcher.take_poll(false));
    Ok(())
}

#[test]
fn a_proposal_the_hub_never_lists_stops_the_reads_at_their_bound() {
    let mut watcher = Watcher::default();
    assert!(watcher.take_poll(false));
    watcher.observe(&[]);
    assert!(watcher.note_ready(1, "meeting"));
    for _ in 0..PROPOSAL_AWAIT_READS {
        assert!(watcher.take_poll(false));
        watcher.observe(&[]);
    }
    assert!(!watcher.take_poll(false));
}
