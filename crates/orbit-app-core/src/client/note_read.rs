//! One note read from the daemon, which reads it from the hub: its
//! document and its turns, page after page.
use super::*;
use orbit_core::{
    notes::{MeetingDocument, Turn},
    reads::ReadRequest,
    topics::{Cursor, Knowledge, Page},
};
use serde_json::Value;

/// Pages of turns one note may take. The hub pages its turns by bytes, 256
/// KiB a page (`notes::TURN_PAGE_BYTES`), about two hours of speech; 64 pages
/// are far more than any meeting. It protects the app from a hub whose cursor never
/// ends, not a meeting's length: past it the transcript says so.
const MAX_TURN_PAGES: usize = 64;

pub(super) fn checked_page(
    response: orbit_core::protocol::TopicResponse,
    request: &ReadRequest,
) -> Result<Page<Value>, String> {
    let id = request.id();
    if response.read != Some(id) {
        return Err("La respuesta pertenece a otra lectura".into());
    }
    match response.answer {
        Some(Knowledge::Present { value, .. }) => Ok(value),
        Some(Knowledge::Absent { refusal }) => Err(refusal.to_string()),
        Some(Knowledge::Unknown { .. }) | None => Err("El daemon no entregó la lectura".into()),
    }
}

pub(super) async fn read_page(
    daemon: &LocalClient,
    request: ReadRequest,
) -> Result<Page<Value>, String> {
    let response = daemon
        .read_topic(request.clone())
        .await
        .map_err(|error| error.to_string())?;
    checked_page(response, &request)
}

async fn read_document(
    daemon: &LocalClient,
    recording_id: &str,
) -> Result<MeetingDocument, String> {
    let page = read_page(
        daemon,
        ReadRequest::NoteDetail {
            id: recording_id.into(),
            cursor: None,
        },
    )
    .await?;
    let item = page
        .items
        .into_iter()
        .next()
        .ok_or_else(|| "El detalle de la nota llegó vacío".to_owned())?;
    let document = serde_json::from_value::<MeetingDocument>(item)
        .map_err(|error| format!("Documento de la nota no válido: {error}"))?;
    if document.recording_id != recording_id {
        return Err("La respuesta pertenece a otra nota".into());
    }
    Ok(document)
}

async fn read_turns(daemon: &LocalClient, recording_id: &str) -> Result<Vec<Turn>, String> {
    let mut turns = Vec::new();
    let mut cursor: Option<Cursor> = None;
    for _ in 0..MAX_TURN_PAGES {
        let page = read_page(
            daemon,
            ReadRequest::NoteTranscript {
                id: recording_id.into(),
                cursor: cursor.clone(),
            },
        )
        .await?;
        for item in page.items {
            turns.push(
                serde_json::from_value::<Turn>(item)
                    .map_err(|error| format!("Intervención no válida: {error}"))?,
            );
        }
        match page.next {
            Some(next) if Some(&next) != cursor.as_ref() => cursor = Some(next),
            Some(_) => {
                return Err(format!(
                    "Cursor repetido tras {} de {} intervenciones",
                    turns.len(),
                    page.total
                ));
            }
            None => return Ok(turns),
        }
    }
    Err(format!(
        "La transcripción no terminó en {MAX_TURN_PAGES} páginas ({} intervenciones)",
        turns.len()
    ))
}

/// The turns of a note whose transcript is complete: the capture closed,
/// nothing pending, and the same state and failed stretches as when they were
/// read, and the hub confirms that no dictionary applies. Its turns then
/// change only with a new group, so a change of anything
/// else — a speaker's name, which comes with the document, the note, the
/// private notes — reads the document alone. A retried stretch moves the
/// state or the failed count, and the turns are read again.
pub(super) struct SettledTurns {
    recording_id: String,
    shape: (orbit_core::notes::TranscriptState, usize),
    turns: Vec<Turn>,
}

fn settled(document: &MeetingDocument) -> Option<(orbit_core::notes::TranscriptState, usize)> {
    use orbit_core::notes::{Capture, TranscriptState};
    let transcript = &document.transcript;
    // A manual final pass can finish between reads, with the same ready
    // state and failed count. Its old turns cannot certify the replacement.
    (document
        .dictionary
        .as_ref()
        .is_some_and(|dictionary| dictionary.dictionary_id.is_none())
        && transcript.final_pass.is_none()
        && document.capture == Capture::Closed
        && transcript.pending_chunks == 0
        && document.local.is_none()
        && matches!(
            transcript.state,
            TranscriptState::Ready | TranscriptState::Failed
        ))
    .then_some((transcript.state, transcript.failed_groups.len()))
}

impl DaemonClient {
    /// The change counter the Notes topic gives this note now.
    pub(super) async fn note_changed_at(&self, recording_id: &str) -> u64 {
        let topics = self.inner.topics.read().await;
        match topics
            .topic(orbit_core::topics::TopicId::Notes)
            .map(|cell| &cell.knowledge)
        {
            Some(orbit_core::topics::TopicData::Notes(Knowledge::Present { value, .. })) => {
                value.changed_at(recording_id)
            }
            _ => 0,
        }
    }

    /// The user opened a note: read it, showing it loading until it arrives.
    pub(super) async fn read_open_note(
        &self,
        recording_id: String,
    ) -> Result<AppSnapshot, AppError> {
        self.read_note(recording_id, true).await
    }

    /// The open note changed on the hub: read it again, keeping what is shown
    /// until the new one arrives.
    pub(super) async fn reread_open_note(
        &self,
        recording_id: String,
    ) -> Result<AppSnapshot, AppError> {
        self.read_note(recording_id, false).await
    }

    async fn read_note(&self, recording_id: String, opened: bool) -> Result<AppSnapshot, AppError> {
        let ticket = {
            let mut target = self.inner.note_target.lock().await;
            if target.0 != recording_id {
                target.0.clone_from(&recording_id);
            }
            target.1 = target.1.wrapping_add(1);
            target.1
        };
        let revision = self.note_changed_at(&recording_id).await;
        if opened {
            self.update_local(|snapshot| {
                if let Some(notes) = snapshot.notes.as_mut() {
                    if notes
                        .open
                        .as_ref()
                        .is_some_and(|open| open.recording_id != recording_id)
                    {
                        notes.open = None;
                    }
                    notes.open_read = Some(crate::NoteReadSnapshot {
                        recording_id: recording_id.clone(),
                        revision,
                        state: crate::NoteReadState::Loading,
                        detail: None,
                        read_at_ms: None,
                    });
                }
            });
        }
        let mut reading = self.inner.note_io.lock().await;
        let read = match read_document(&self.inner.daemon, &recording_id).await {
            Ok(document) => {
                let shape = settled(&document);
                let kept = reading
                    .as_ref()
                    .filter(|kept| {
                        !opened && kept.recording_id == recording_id && Some(kept.shape) == shape
                    })
                    .map(|kept| kept.turns.clone());
                let turns = match kept {
                    Some(turns) => Ok(turns),
                    None => read_turns(&self.inner.daemon, &recording_id).await,
                };
                *reading = match (&turns, shape) {
                    (Ok(turns), Some(shape)) => Some(SettledTurns {
                        recording_id: recording_id.clone(),
                        shape,
                        turns: turns.clone(),
                    }),
                    _ => None,
                };
                Ok((document, turns))
            }
            Err(cause) => Err(cause),
        };
        drop(reading);
        let library = self
            .local_snapshot()
            .notes
            .and_then(|notes| notes.library.map(|library| *library));
        let now_ms = self.inner.calendar.lock().await.now_ms();
        if !self.note_is_target(&recording_id, ticket).await {
            return Ok(self.local_snapshot());
        }
        match read {
            Ok((document, turns)) => {
                let note =
                    crate::projection::note::project_open_note(document, turns, library.as_ref());
                Ok(self.update_local(|snapshot| {
                    if let Some(notes) = snapshot.notes.as_mut() {
                        notes.open = Some(note);
                        notes.open_read = Some(crate::NoteReadSnapshot {
                            recording_id,
                            revision,
                            state: crate::NoteReadState::Available,
                            detail: None,
                            read_at_ms: Some(now_ms),
                        });
                    }
                }))
            }
            Err(cause) if opened => {
                self.update_local(|snapshot| {
                    if let Some(notes) = snapshot.notes.as_mut() {
                        notes.open = None;
                        notes.open_read = Some(crate::NoteReadSnapshot {
                            recording_id,
                            revision,
                            state: crate::NoteReadState::Failed,
                            detail: Some(cause.clone()),
                            read_at_ms: None,
                        });
                    }
                });
                Err(AppError::daemon(cause))
            }
            // A reread that failed keeps the note shown and says why; the
            // next change reads it again.
            Err(cause) => Ok(self.update_local(|snapshot| {
                if let Some(read) = snapshot
                    .notes
                    .as_mut()
                    .and_then(|notes| notes.open_read.as_mut())
                    .filter(|read| read.recording_id == recording_id)
                {
                    read.detail = Some(cause);
                }
            })),
        }
    }

    async fn note_is_target(&self, recording_id: &str, ticket: u64) -> bool {
        let target = self.inner.note_target.lock().await;
        target.0 == recording_id && target.1 == ticket
    }
}
