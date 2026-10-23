//! One request of the notes outbox to the hub, and what came back. Nothing
//! here retries or keeps anything: the outbox decides what an answer means.
use super::{MAX_KEPT_CONFLICTS, Outcome};
use crate::hub_notes::{Answer, NoteHub};
use orbit_core::{
    note_outbox::NoteOrder,
    notes::{self as core, LibraryRequest, Section},
};
use std::collections::BTreeMap;

/// The `conflicts` of a dictionary order's answer. A hub up to 0.10.1 sends
/// none, and so does an order that is not a correction. One the Mac cannot
/// read is said in the log, by its shape only, and shows no warning: the
/// correction itself was saved.
fn conflicts_of(answer: &serde_json::Value) -> Vec<core::DictionaryConflict> {
    let Some(listed) = answer.get("conflicts") else {
        return Vec::new();
    };
    match serde_json::from_value::<Vec<core::DictionaryConflict>>(listed.clone()) {
        Ok(mut conflicts) => {
            conflicts.truncate(MAX_KEPT_CONFLICTS);
            conflicts
        }
        Err(error) => {
            eprintln!(
                "dictionary_conflicts_unreadable line={} column={}",
                error.line(),
                error.column()
            );
            Vec::new()
        }
    }
}

/// The hub's answer to a note edit on a revision that moved (409
/// `note_revision_changed`, which carries no revision): the document is read
/// again for the current one, and the order goes once more on it. The Mac is
/// the only writer of this device's private notes and edits, so the revision
/// moves under it only when the hub itself rewrote the note or an earlier
/// order of the user's had not been answered yet.
fn conflict<T>(answer: &Answer<T>) -> bool {
    matches!(answer, Answer::Refused { status: 409, .. })
}

async fn current_document(
    hub: &NoteHub,
    recording: &str,
) -> Result<core::MeetingDocument, Outcome> {
    match hub.document(recording).await {
        Answer::Value(document) => Ok(document),
        other @ (Answer::Refused { .. } | Answer::Unreachable(_)) => Err(other.into()),
    }
}

/// Private notes go on the revision the hub's 409 names
/// (`stale_revision {revision, base_revision}`); a 409 without it is
/// answered as the hub gave it.
pub(super) async fn send_private_notes(
    hub: &NoteHub,
    pending: &orbit_store::PendingPrivateNotes,
) -> Outcome {
    let recording = &pending.recording_id;
    let first = hub
        .private_notes(recording, &pending.markdown, pending.base_revision)
        .await;
    match first {
        Answer::Refused {
            status: 409,
            revision: Some(current),
            ..
        } if current != pending.base_revision => hub
            .private_notes(recording, &pending.markdown, current)
            .await
            .into(),
        other @ (Answer::Value(_) | Answer::Refused { .. } | Answer::Unreachable(_)) => {
            other.into()
        }
    }
}

pub(super) async fn send_note_order(hub: &NoteHub, recording: &str, order: &NoteOrder) -> Outcome {
    match order {
        NoteOrder::Edit {
            sections,
            done,
            base_revision,
        } => {
            let first = hub
                .edit_note(recording, sections.as_deref(), done, *base_revision)
                .await;
            if !conflict(&first) {
                return first.into();
            }
            match current_document(hub, recording).await {
                Ok(document) => {
                    let revision = document.note.map_or(0, |note| note.revision);
                    hub.edit_note(recording, sections.as_deref(), done, revision)
                        .await
                        .into()
                }
                Err(outcome) => outcome,
            }
        }
        NoteOrder::Write { template_id } => hub.write_note(recording, template_id).await.into(),
        NoteOrder::RestoreSuperseded => restore_superseded(hub, recording).await,
        NoteOrder::Rename { key, name, scope } => hub
            .rename(recording, key, name.as_deref(), *scope)
            .await
            .into(),
        NoteOrder::Language { language } => hub.language(recording, language).await.into(),
        NoteOrder::Title { title } => hub.title(recording, title).await.into(),
        NoteOrder::Retranscribe => hub.final_pass(recording).await.into(),
        NoteOrder::Delete { .. } => super::deletion::send_delete(hub, recording).await,
        NoteOrder::RetryTranscript => match hub.retry_transcript(recording).await {
            Answer::Value(value) => Outcome::Done {
                requeued: value.get("requeued").and_then(serde_json::Value::as_u64),
                conflicts: Vec::new(),
            },
            other @ (Answer::Refused { .. } | Answer::Unreachable(_)) => other.into(),
        },
    }
}

/// The edits a regeneration set aside come back as an edit of the note: the
/// meeting's document says they exist, the note's own route carries them.
async fn restore_superseded(hub: &NoteHub, recording: &str) -> Outcome {
    let note = match hub.note_with_superseded(recording).await {
        Answer::Value(note) => note,
        other @ (Answer::Refused { .. } | Answer::Unreachable(_)) => return other.into(),
    };
    let Some(sections) = note
        .get("superseded_edits")
        .and_then(|edits| edits.get("sections"))
        .and_then(|sections| serde_json::from_value::<Vec<Section>>(sections.clone()).ok())
    else {
        return Outcome::Refused {
            status: 404,
            code: Some("nothing_superseded".into()),
            numbers: BTreeMap::new(),
        };
    };
    let revision = note
        .get("revision")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    hub.edit_note(recording, Some(&sections), &BTreeMap::new(), revision)
        .await
        .into()
}

pub(super) async fn send_library_order(hub: &NoteHub, request: &LibraryRequest) -> Outcome {
    match request {
        LibraryRequest::Dictionary { request } => match hub.dictionary_order(request).await {
            Answer::Value(value) => Outcome::Done {
                requeued: None,
                conflicts: conflicts_of(&value),
            },
            other @ (Answer::Refused { .. } | Answer::Unreachable(_)) => other.into(),
        },
        LibraryRequest::SaveTemplate { template_id, draft } => {
            hub.save_template(template_id, draft).await.into()
        }
        LibraryRequest::DeleteTemplate { template_id } => {
            hub.delete_template(template_id).await.into()
        }
        LibraryRequest::Assign {
            calendar_item_id,
            template_id: Some(template_id),
        } => hub.link(calendar_item_id, template_id).await.into(),
        LibraryRequest::Assign {
            calendar_item_id,
            template_id: None,
        } => hub.unlink(calendar_item_id).await.into(),
        LibraryRequest::UseForSeries {
            recording_id,
            template_id,
            dictionary_id,
            dictionary_ids,
        } => hub
            .use_for_series(
                recording_id,
                template_id,
                dictionary_id.as_deref(),
                dictionary_ids.as_deref(),
            )
            .await
            .into(),
        LibraryRequest::RestoreTemplate {
            template_id,
            draft,
            series,
            ..
        } => {
            let saved: Outcome = hub.save_template(template_id, draft).await.into();
            if !matches!(saved, Outcome::Done { .. }) {
                return saved;
            }
            for calendar_item_id in series.iter().flatten() {
                let linked: Outcome = hub.link(calendar_item_id, template_id).await.into();
                if !matches!(linked, Outcome::Done { .. }) {
                    return linked;
                }
            }
            saved
        }
    }
}
