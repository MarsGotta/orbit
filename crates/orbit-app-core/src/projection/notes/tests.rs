use super::*;
use orbit_core::notes::{Capture, LocalRecording, NoteHeadline};

fn row(local: Option<LocalPhase>, transcript: TranscriptState) -> MeetingRow {
    MeetingRow {
        recording_id: "6f2b7d5a-9c41-4b7e-8d3f-1a2b3c4d5e6f".into(),
        title: "Comité de dirección".into(),
        started_at_ms: 1_800_000_000_000,
        duration_ms: 60_000,
        capture: Capture::Closed,
        transcript,
        note: None,
        local: local.map(|phase| LocalRecording {
            phase,
            pending_chunks: Some(3),
        }),
    }
}

#[test]
fn a_row_says_where_the_recording_stands_from_the_mac_and_the_hub() {
    let zone = LocalZone::utc();
    let now = 1_800_000_000_000;
    let state = |local, transcript| project_row(row(local, transcript), now, &zone).state;
    assert_eq!(
        state(Some(LocalPhase::Recording), TranscriptState::Pending),
        NoteListState::Recording
    );
    assert_eq!(
        state(Some(LocalPhase::Uploading), TranscriptState::None),
        NoteListState::Uploading
    );
    assert_eq!(
        state(None, TranscriptState::Pending),
        NoteListState::Transcribing
    );
    assert_eq!(state(None, TranscriptState::Ready), NoteListState::Ready);
    assert_eq!(state(None, TranscriptState::Failed), NoteListState::Failed);
}

#[test]
fn the_title_is_the_meetings_and_the_line_begins_the_short_summary() {
    let mut meeting = row(None, TranscriptState::Ready);
    meeting.note = Some(NoteHeadline {
        state: NoteState::Ready,
        short_summary: Some("á".repeat(200)),
    });
    let item = project_row(meeting, 1_800_000_000_000, &LocalZone::utc());
    assert_eq!(item.title, "Comité de dirección");
    let Some(NoteHeadlineSnapshot::Ready { preview }) = item.headline else {
        panic!("no preview: {:?}", item.headline);
    };
    assert_eq!(preview.chars().count(), PREVIEW_CHARS);
}

#[test]
fn days_follow_the_local_calendar() {
    let zone = LocalZone::utc();
    let now = 1_800_000_000_000;
    assert_eq!(note_day_group(now, now, &zone), NoteDayGroup::Today);
    assert_eq!(
        note_day_group(now - 86_400_000, now, &zone),
        NoteDayGroup::Yesterday
    );
    assert_eq!(
        note_day_group(now - 40 * 86_400_000, now, &zone),
        NoteDayGroup::Before
    );
}

/// The list's days are the user's calendar days, recomputed from the
/// clock when the view is projected (no new read at midnight).
#[test]
fn notes_are_grouped_by_the_local_day() {
    use crate::{
        calendar::LocalClock,
        features::{FeatureInput, notes},
        projection::test_support::*,
        topics::TopicCache,
    };
    let mut local = crate::AppSnapshot::empty(12, crate::DaemonConnection::Ready);
    if let Some(notes) = local.notes.as_mut() {
        notes.list = vec![
            recording_summary("rec-tuesday-morning", "Daily", UTC_2026_09_15_T0630Z),
            recording_summary("rec-wednesday-night", "Guardia", UTC_2026_09_15_T2215Z),
        ];
    }
    let view = notes::project(&FeatureInput::with_clock(
        &local,
        &TopicCache::default(),
        LocalClock {
            now_ms: UTC_2026_09_16_T0500Z,
            zone: madrid(),
            timezone_name: Ok("Europe/Madrid".into()),
        },
    ))
    .expect("notes view");
    let groups = view
        .list
        .iter()
        .map(|note| (note.recording_id.as_str(), note.group.clone()))
        .collect::<Vec<_>>();
    assert_eq!(
        groups,
        vec![
            ("rec-tuesday-morning", NoteDayGroup::Yesterday),
            ("rec-wednesday-night", NoteDayGroup::Today),
        ],
        "22.5 hours ago is still yesterday when it was Tuesday in Madrid"
    );
}

/// A note is news when it was seen being written and is now read ready;
/// one read ready the first time (the app just started) is not, nor is a
/// failure, and a read that does not list a note in progress keeps it.
#[test]
fn a_note_seen_being_written_and_then_ready_is_news_once() {
    let zone = LocalZone::utc();
    let now = 1_800_000_000_000;
    let item = |state: NoteState| {
        let mut row = row(None, TranscriptState::Ready);
        row.note = Some(NoteHeadline {
            state,
            short_summary: Some("Se aprobó el plan.".into()),
        });
        project_row(row, now, &zone)
    };
    let mut writing = std::collections::BTreeSet::new();
    assert_eq!(
        follow_writing(&mut writing, &[item(NoteState::Ready)]),
        None,
        "already written when first read"
    );
    assert_eq!(
        follow_writing(&mut writing, &[item(NoteState::Generating)]),
        None
    );
    assert_eq!(follow_writing(&mut writing, &[]), None, "a read without it");
    let ready = [item(NoteState::Ready)];
    assert_eq!(
        follow_writing(&mut writing, &ready).map(|item| item.title.as_str()),
        Some("Comité de dirección")
    );
    assert_eq!(follow_writing(&mut writing, &ready), None, "said once");
    assert_eq!(
        follow_writing(&mut writing, &[item(NoteState::Queued)]),
        None
    );
    assert_eq!(
        follow_writing(&mut writing, &[item(NoteState::Failed)]),
        None
    );
    assert!(
        writing.is_empty(),
        "a failed note is no longer being written"
    );
    assert_eq!(follow_writing(&mut writing, &ready), None);
}
