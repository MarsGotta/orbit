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

/// REF-04: the list's days are Mars's calendar days, recomputed from the
/// clock when the view is projected (no new read at midnight).
#[test]
fn ref04_a1_notes_are_grouped_by_the_local_day() {
    use crate::{
        calendar::LocalClock,
        features::{FeatureInput, notes},
        projection::test_support::*,
        topics::TopicCache,
    };
    let mut local = crate::AppSnapshot::empty(12, crate::DaemonConnection::Ready);
    if let Some(notes) = local.notes.as_mut() {
        notes.list = vec![
            recording_summary("rec-tuesday-morning", "Daily", REF04_2026_09_15_T0630Z),
            recording_summary("rec-wednesday-night", "Guardia", REF04_2026_09_15_T2215Z),
        ];
    }
    let view = notes::project(&FeatureInput::with_clock(
        &local,
        &TopicCache::default(),
        LocalClock {
            now_ms: REF04_2026_09_16_T0500Z,
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
