//! SIM-41: one meeting of the hub's list, or of this Mac's own, as a row.
use super::*;
use orbit_core::notes::{LocalPhase, MeetingRow, NoteState, TranscriptState};

/// The list's line shows how the short summary begins: 140 characters fit
/// one line of the list at its narrowest. A display length, not a bound on
/// the summary.
const PREVIEW_CHARS: usize = 140;

/// Calendar days on Mars's clock, not periods of 24 hours: a note from 23:00
/// is "Ayer" at 07:00 the next morning.
pub(crate) fn note_day_group(started_at_ms: i64, now_ms: i64, zone: &LocalZone) -> NoteDayGroup {
    let days = zone
        .day(now_ms)
        .saturating_sub(zone.day(started_at_ms))
        .max(0);
    match days {
        0 => NoteDayGroup::Today,
        1 => NoteDayGroup::Yesterday,
        2..=6 => NoteDayGroup::ThisWeek,
        7..=29 => NoteDayGroup::Last30Days,
        _ => NoteDayGroup::Before,
    }
}

pub(crate) fn project_row(row: MeetingRow, now_ms: i64, zone: &LocalZone) -> NoteListItem {
    let state = match (row.local.map(|local| local.phase), row.transcript) {
        (Some(LocalPhase::Recording), _) => NoteListState::Recording,
        (Some(LocalPhase::Uploading), _) => NoteListState::Uploading,
        (None, TranscriptState::Failed) => NoteListState::Failed,
        (None, TranscriptState::Ready) => NoteListState::Ready,
        (None, TranscriptState::None | TranscriptState::Pending) => NoteListState::Transcribing,
    };
    let headline = row.note.and_then(|note| match note.state {
        NoteState::WaitingTranscript => Some(NoteHeadlineSnapshot::Transcribing {}),
        NoteState::Queued | NoteState::Generating => Some(NoteHeadlineSnapshot::Writing {}),
        NoteState::Ready => Some(NoteHeadlineSnapshot::Ready {
            preview: note
                .short_summary
                .unwrap_or_default()
                .chars()
                .take(PREVIEW_CHARS)
                .collect(),
        }),
        NoteState::Failed => Some(NoteHeadlineSnapshot::Failed {}),
        NoteState::NotConfigured => Some(NoteHeadlineSnapshot::NotConfigured {}),
        NoteState::Unknown => None,
    });
    NoteListItem {
        group: note_day_group(row.started_at_ms, now_ms, zone),
        recording_id: row.recording_id,
        title: row.title,
        started_at_ms: row.started_at_ms,
        duration_ms: row.duration_ms,
        state,
        headline,
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests;
