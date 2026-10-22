//! The hub's document of one meeting, in the screen's words. The
//! times, speakers' names, turns and language are the hub's; nothing here
//! recomputes them.
use super::*;
use orbit_core::{
    notes::{LocalPhase, MeetingDocument, NoteDocument, NoteState, TranscriptState, Turn},
    refusal::Refusal,
};

/// How the note names the user when no speaker is their microphone.
const DEFAULT_OWNER_NAME: &str = orbit_core::notes::DEFAULT_OWNER_NAME;

fn step(state: NoteStepState, cause: Option<NoteStepCause>) -> NoteStepSnapshot {
    NoteStepSnapshot { state, cause }
}

/// Upload → Transcript → Summary, from what the hub and this Mac know.
pub(crate) fn project_progress(document: &MeetingDocument) -> NoteProgressSnapshot {
    let local = document.local.as_ref().map(|local| local.phase);
    let recording = local == Some(LocalPhase::Recording);
    let upload = match document.local {
        Some(local) if local.phase == LocalPhase::Recording => {
            step(NoteStepState::Active, Some(NoteStepCause::Recording {}))
        }
        Some(local) => step(
            NoteStepState::Active,
            Some(NoteStepCause::Uploading {
                chunks: local.pending_chunks,
            }),
        ),
        None => step(NoteStepState::Done, None),
    };
    let transcript = &document.transcript;
    let failed = transcript.failed_groups.len() as u64;
    let transcript_step = if failed > 0 || transcript.state == TranscriptState::Failed {
        step(
            NoteStepState::Failed,
            Some(NoteStepCause::TranscriptFailed { groups: failed }),
        )
    } else {
        match transcript.state {
            TranscriptState::Ready => step(NoteStepState::Done, None),
            TranscriptState::Pending => step(
                NoteStepState::Active,
                Some(NoteStepCause::Transcribing {
                    chunks: transcript.pending_chunks,
                }),
            ),
            TranscriptState::None | TranscriptState::Failed => step(
                NoteStepState::Waiting,
                document
                    .hub_refusal
                    .as_ref()
                    .filter(|refusal| !matches!(refusal, Refusal::NotFound { .. }))
                    .map(|_| NoteStepCause::HubUnavailable {}),
            ),
        }
    };
    let summary = match &document.note {
        None if recording => step(
            NoteStepState::Waiting,
            Some(NoteStepCause::AfterRecording {}),
        ),
        None if transcript.state != TranscriptState::Ready => step(
            NoteStepState::Waiting,
            Some(NoteStepCause::WaitingTranscript {}),
        ),
        None => step(NoteStepState::Waiting, Some(NoteStepCause::NotRequested {})),
        Some(note) => summary_step(note),
    };
    NoteProgressSnapshot {
        upload,
        transcript: transcript_step,
        summary,
    }
}

fn summary_step(note: &NoteDocument) -> NoteStepSnapshot {
    match note.state {
        NoteState::NotConfigured => {
            step(NoteStepState::Failed, Some(NoteStepCause::NotConfigured {}))
        }
        NoteState::WaitingTranscript => step(
            NoteStepState::Waiting,
            Some(NoteStepCause::WaitingTranscript {}),
        ),
        // A failed attempt the hub will repeat on its own.
        NoteState::Queued | NoteState::Generating if note.error.is_some() => {
            step(NoteStepState::Active, Some(NoteStepCause::Retrying {}))
        }
        NoteState::Queued | NoteState::Generating => {
            step(NoteStepState::Active, Some(NoteStepCause::Writing {}))
        }
        NoteState::Ready => step(NoteStepState::Done, None),
        NoteState::Failed => step(NoteStepState::Failed, Some(NoteStepCause::Failed {})),
        NoteState::Unknown => step(NoteStepState::Waiting, None),
    }
}

/// A colour that stays with a speaker's key across notes: one of the four
/// calendar colours of `tokens.css`. Red is left out: on the note screen it
/// means recording and failure.
pub(crate) fn stable_speaker_color(key: &str) -> String {
    const COLORS: [&str; 4] = ["var(--o-c1)", "var(--o-c2)", "var(--o-c3)", "var(--o-c4)"];
    let index = key.bytes().fold(0_usize, |hash, byte| {
        hash.wrapping_mul(31).wrapping_add(byte as usize)
    }) % COLORS.len();
    COLORS[index].into()
}

/// The open note: the hub's document with its turns, and the template
/// assigned to the event's series when the library knows it.
pub(crate) fn project_open_note(
    document: MeetingDocument,
    turns: Result<Vec<Turn>, String>,
    library: Option<&NoteLibrarySnapshot>,
) -> OpenNoteSnapshot {
    let progress = project_progress(&document);
    let (turns, error) = match turns {
        Ok(turns) => (turns, None),
        Err(cause) => (Vec::new(), Some(cause)),
    };
    let total = turns.len();
    let speakers = document
        .speakers
        .iter()
        .map(|speaker| {
            let spoken = turns
                .iter()
                .filter(|turn| turn.speaker_key == speaker.key)
                .count();
            NoteSpeakerSnapshot {
                key: speaker.key.clone(),
                track: speaker.track.clone(),
                name: speaker.name.clone(),
                color: stable_speaker_color(&speaker.key),
                share_pct: u8::try_from((spoken * 100).checked_div(total).unwrap_or(0))
                    .unwrap_or(100),
            }
        })
        .collect::<Vec<_>>();
    let owner_name = document
        .speakers
        .iter()
        .find(|speaker| speaker.track.as_deref() == Some("mic_local"))
        .map_or_else(
            || DEFAULT_OWNER_NAME.to_owned(),
            |speaker| speaker.name.clone(),
        );
    let series = document.event.as_ref().map(|event| NoteSeriesLinkSnapshot {
        calendar_item_id: event.calendar_item_id.clone(),
        assigned: match library {
            Some(library) if library.links.state == NoteTemplatesState::Available => {
                NoteSeriesAssignedSnapshot::Known {
                    template_id: library
                        .assignments
                        .iter()
                        .find(|link| link.calendar_item_id == event.calendar_item_id)
                        .map(|link| link.template_id.clone()),
                }
            }
            Some(library) if library.links.state == NoteTemplatesState::Failed => {
                NoteSeriesAssignedSnapshot::Failed {
                    cause: library.links.detail.clone().unwrap_or_default(),
                }
            }
            Some(_) | None => NoteSeriesAssignedSnapshot::Unknown {},
        },
    });
    let note_revision = document.note.as_ref().map(|note| note.revision);
    let edited_sections = document
        .note
        .as_ref()
        .is_some_and(|note| note.edited.sections);
    let superseded = document
        .note
        .as_ref()
        .is_some_and(|note| note.superseded_available);
    let seen = document.note.and_then(|note| note.note);
    let (summary, actions, decisions, open_topics) = match seen {
        Some(seen) => (
            Some(NoteSummarySnapshot {
                short_summary: seen.short_summary,
                sections: seen
                    .sections
                    .into_iter()
                    .map(|section| NoteSectionSnapshot {
                        name: section.name,
                        markdown: section.markdown,
                        edited: edited_sections,
                    })
                    .collect(),
                template_id: seen.template_id,
                template_name: seen.template_name,
                generated_at_ms: seen.generated_at_ms,
                model: seen.model,
                superseded_available: superseded,
                template_source: seen.template_source,
            }),
            seen.action_items
                .into_iter()
                .map(|action| NoteActionSnapshot {
                    id: action.id,
                    owner: action.owner,
                    text: action.text,
                    due: action.due,
                    done: action.done,
                })
                .collect(),
            seen.decisions,
            seen.open_topics,
        ),
        None => (None, Vec::new(), Vec::new(), Vec::new()),
    };
    OpenNoteSnapshot {
        dictionary: document.dictionary,
        capture: Some(document.capture),
        recording_id: document.recording_id,
        title: document.title,
        owner_name,
        started_at_ms: document.started_at_ms,
        ended_at_ms: document
            .started_at_ms
            .saturating_add(document.duration_ms.max(0)),
        event: document.event.map(|event| NoteEventSnapshot {
            attendees: event.attendees,
            title: event.title,
            start_ms: event.starts_at_ms,
            end_ms: event.ends_at_ms,
        }),
        summary,
        note_revision,
        actions,
        decisions,
        open_topics,
        private_notes: document.private_notes.text,
        private_notes_revision: document.private_notes.revision,
        progress,
        transcript: NoteTranscriptSnapshot {
            final_pass: document
                .transcript
                .final_pass
                .map(|final_pass| NoteFinalPassSnapshot {
                    state: final_pass.state,
                    timeout_ms: final_pass
                        .error
                        .as_ref()
                        .and_then(|cause| cause.details.get("timeout_ms"))
                        .and_then(serde_json::Value::as_u64),
                    error: final_pass.error.map(|cause| cause.code),
                }),
            unmatched_speaker_names: (document.transcript.unmatched_speaker_names > 0)
                .then_some(document.transcript.unmatched_speaker_names),
            language: document.language,
            turns: turns
                .into_iter()
                .enumerate()
                .map(|(index, turn)| NoteTurnSnapshot {
                    index,
                    at_ms: turn.at_ms,
                    speaker_key: turn.speaker_key,
                    text: turn.text,
                })
                .collect(),
            pending_chunks: document.transcript.pending_chunks,
            failed_groups: document.transcript.failed_groups.len() as u64,
            error,
            retranscribing: document.transcript.retranscribing.map(|repeating| {
                NoteRetranscribingSnapshot {
                    language: repeating.language,
                    done: repeating.done,
                    total: repeating.total,
                }
            }),
        },
        speakers,
        series,
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests;
