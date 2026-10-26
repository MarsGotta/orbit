//! One meeting of the hub's list, or of this Mac's own, as a row.
use super::*;
use orbit_core::notes::{LocalPhase, MeetingRow, NoteState, TranscriptState};

/// The list's line shows how the short summary begins: 140 characters fit
/// one line of the list at its narrowest. A display length, not a bound on
/// the summary.
const PREVIEW_CHARS: usize = 140;

/// Calendar days on the user's clock, not periods of 24 hours: a note from 23:00
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
        recorded_on: row.recorded_on,
        group: note_day_group(row.started_at_ms, now_ms, zone),
        recording_id: row.recording_id,
        title: row.title,
        started_at_ms: row.started_at_ms,
        duration_ms: row.duration_ms,
        state,
        headline,
    }
}

/// Follows the notes being written across list reads: `writing` holds those
/// seen in progress (recorded, transcribed or summarized, the summary not
/// written yet).
/// Returns the newest one of them `list` shows ready, which is news; a note
/// read ready without having been seen in progress — every one when the
/// app starts — is not. A note seen failed or without a model leaves
/// `writing` (written again later, it is news again); one the read did not
/// list stays, so a read the hub could not answer loses none. It holds the
/// notes in progress, a handful; one deleted while in progress stays until
/// the app quits.
pub(crate) fn follow_writing<'a>(
    writing: &mut std::collections::BTreeSet<String>,
    list: &'a [NoteListItem],
) -> Option<&'a NoteListItem> {
    let mut ready = None;
    for item in list {
        let written = matches!(item.headline, Some(NoteHeadlineSnapshot::Ready { .. }));
        let in_progress = !written
            && (matches!(
                item.state,
                NoteListState::Recording | NoteListState::Uploading | NoteListState::Transcribing
            ) || matches!(
                item.headline,
                Some(NoteHeadlineSnapshot::Transcribing {} | NoteHeadlineSnapshot::Writing {})
            ));
        if in_progress {
            writing.insert(item.recording_id.clone());
        } else if writing.remove(&item.recording_id) && written {
            ready = ready.or(Some(item));
        }
    }
    ready
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests;
