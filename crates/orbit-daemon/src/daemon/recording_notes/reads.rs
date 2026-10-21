//! SIM-41: the notes reads, served from the hub. The facts the Mac owns — the
//! recording in progress, the ones still uploading, the private notes still
//! on their way and the event a recording was started from — are taken on
//! the daemon loop; the hub is asked off it, and its answer is passed through
//! with those facts laid over it. Nothing is kept.
use super::super::{
    Runtime,
    reads::{bounded_page, cursor_value},
};
use crate::hub_notes::NoteHub;
use orbit_core::{
    CapturePhase,
    notes::{
        Capture, ListItem, LocalPhase, LocalRecording, MeetingDocument, MeetingEvent, MeetingRow,
        PrivateNotes, TranscriptProgress, TranscriptState,
    },
    reads::{ReadId, ReadRequest},
    refusal::{Operation, Refusal},
    topics::{Cursor, Knowledge, Page},
};
use serde_json::Value;
use std::{future::Future, pin::Pin};

/// Recordings of this Mac still uploading that the list adds to the hub's.
/// A recording's closure waits only for its own chunks, so more than a
/// hundred unfinished at once means the Mac has not reached the hub for
/// weeks; past it the oldest are not listed until the newer ones upload. It
/// bounds one read of the queue, not recordings.
const MAX_LOCAL_UPLOADING: usize = 100;

pub(in crate::daemon) type RemoteRead =
    Pin<Box<dyn Future<Output = Knowledge<Page<Value>>> + Send + 'static>>;

fn absent(refusal: Refusal) -> Knowledge<Page<Value>> {
    Knowledge::Absent { refusal }
}

fn storage(error: impl std::fmt::Display) -> Refusal {
    Refusal::StorageFailed {
        operation: Operation::Storage,
        cause: Some(format!("No se pudo leer la grabación en este Mac: {error}").into()),
    }
}

impl Runtime {
    /// The hub, or why it cannot be asked now.
    fn hub_for_read(&self) -> Result<NoteHub, Refusal> {
        self.hub_access()?;
        self.note_hub().ok_or(Refusal::NotPaired)
    }

    fn pending_chunks(&self, recording: &str) -> Option<u64> {
        self.reader
            .finalize_counts(recording)
            .ok()
            .map(|(chunks, uploaded)| chunks.saturating_sub(uploaded))
    }

    /// A voice call closed on this Mac and not yet on the hub. The hub
    /// leaves closed calls out of its list (`kind: voice_call`); these rows
    /// are the Mac's own, which the hub has not seen. A voice note is never
    /// closed, so it has no row here.
    fn voice_call(&self, recording: &str) -> bool {
        self.reader
            .get_job(&format!("recording-{recording}"))
            .ok()
            .flatten()
            .is_some_and(|job| job.payload["recording_kind"] == "voice_call")
    }

    /// The recordings of this Mac the hub does not have whole yet: the one
    /// being recorded and those still uploading, newest first.
    fn local_rows(&self) -> Result<Vec<MeetingRow>, Refusal> {
        let now_ms = self.notes_now();
        let mut rows = Vec::new();
        let active = self.state.durable.recording.as_ref().filter(|recording| {
            matches!(
                recording.phase,
                CapturePhase::Starting
                    | CapturePhase::Recording
                    | CapturePhase::Paused
                    | CapturePhase::Finalizing
            )
        });
        if let Some(recording) = active {
            rows.push(MeetingRow {
                recording_id: recording.id.clone(),
                title: recording.title.clone(),
                started_at_ms: recording.started_at_ms,
                duration_ms: now_ms.saturating_sub(recording.started_at_ms).max(0),
                capture: Capture::Open,
                transcript: TranscriptState::None,
                note: None,
                local: Some(LocalRecording {
                    phase: LocalPhase::Recording,
                    pending_chunks: self.pending_chunks(&recording.id),
                }),
            });
        }
        let closures = self
            .reader
            .pending_jobs_of_kind("recording_finalize", None, MAX_LOCAL_UPLOADING)
            .map_err(storage)?;
        for job in closures {
            let Some(recording_id) = job.payload["recording_id"].as_str() else {
                continue;
            };
            if active.is_some_and(|active| active.id == recording_id)
                || self.voice_call(recording_id)
            {
                continue;
            }
            let manifest = &job.payload["manifest"];
            let Some(started_at_ms) = manifest["started_at_ms"].as_i64() else {
                continue;
            };
            let finalized_at_ms = manifest["finalized_at_ms"]
                .as_i64()
                .unwrap_or(started_at_ms);
            rows.push(MeetingRow {
                recording_id: recording_id.to_owned(),
                title: manifest["title"]
                    .as_str()
                    .map_or_else(|| "Grabación".to_owned(), str::to_owned),
                started_at_ms,
                duration_ms: finalized_at_ms.saturating_sub(started_at_ms).max(0),
                capture: Capture::Closed,
                transcript: TranscriptState::None,
                note: None,
                local: Some(LocalRecording {
                    phase: LocalPhase::Uploading,
                    pending_chunks: self.pending_chunks(recording_id),
                }),
            });
        }
        rows.sort_by(newest_first);
        Ok(rows)
    }

    /// P9-11: the event the recording was started from, as its manifest
    /// keeps it.
    fn recording_event(&self, recording: &str) -> Option<MeetingEvent> {
        let job = self
            .reader
            .get_job(&format!("recording-{recording}"))
            .ok()
            .flatten()?;
        let linked = serde_json::from_value::<orbit_core::call_detection::LinkedEvent>(
            job.payload.get("event")?.clone(),
        )
        .ok()?;
        Some(MeetingEvent {
            attendees: orbit_core::notes::event_context(&linked).attendees,
            title: linked.title,
            starts_at_ms: linked.starts_at_ms,
            ends_at_ms: linked.ends_at_ms,
            calendar_item_id: linked.calendar_item_id,
        })
    }

    /// The notes reads, prepared on the loop and answered off it.
    pub(in crate::daemon) fn notes_read(&self, read: &ReadRequest) -> Option<RemoteRead> {
        let hub = self.hub_for_read();
        let head = &self.features.notes.head;
        let answer: RemoteRead = match read {
            ReadRequest::NotesPage { cursor } => {
                let first = cursor.is_none();
                let before = match cursor_value(cursor.as_ref(), |cursor| {
                    if let Cursor::DescendingDateAndId { date_ms, id } = cursor {
                        Some((*date_ms, id.clone()))
                    } else {
                        None
                    }
                }) {
                    Ok(before) => before,
                    Err(refusal) => return Some(Box::pin(async move { absent(refusal) })),
                };
                let local = match self.local_rows() {
                    Ok(local) => local,
                    Err(refusal) => return Some(Box::pin(async { absent(refusal) })),
                };
                let revision = head.revision;
                // The call in progress is on the hub as an open recording
                // until it closes as a `voice_call`.
                let call = self.state.durable.call.as_ref().map(|call| call.id.clone());
                Box::pin(list_page(hub, before, first, local, call, revision))
            }
            ReadRequest::NoteDetail { id, .. } => {
                let local = self
                    .local_rows()
                    .ok()
                    .and_then(|rows| rows.into_iter().find(|row| row.recording_id == *id));
                let private = match self.reader.private_notes(id) {
                    Ok(private) => private,
                    Err(error) => {
                        let refusal = storage(error);
                        return Some(Box::pin(async { absent(refusal) }));
                    }
                };
                let local = LocalNote {
                    row: local,
                    event: self.recording_event(id),
                    private,
                };
                let revision = head.changed_at(id);
                Box::pin(detail(hub, id.clone(), local, revision))
            }
            ReadRequest::NoteTranscript { id, cursor } => {
                let offset = match cursor_value(cursor.as_ref(), |cursor| {
                    if let Cursor::Index { index } = cursor {
                        Some(*index)
                    } else {
                        None
                    }
                }) {
                    Ok(offset) => offset.unwrap_or(0),
                    Err(refusal) => return Some(Box::pin(async move { absent(refusal) })),
                };
                let local = self
                    .active_recording_id()
                    .is_some_and(|active| active == id)
                    || self
                        .local_rows()
                        .is_ok_and(|rows| rows.iter().any(|row| row.recording_id == *id));
                let revision = head.changed_at(id);
                Box::pin(transcript(hub, id.clone(), offset, local, revision))
            }
            ReadRequest::NoteLibrary { cursor } => {
                let skip = match cursor_value(cursor.as_ref(), |cursor| {
                    if let Cursor::Index { index } = cursor {
                        Some(usize::try_from(*index).unwrap_or(usize::MAX))
                    } else {
                        None
                    }
                }) {
                    Ok(skip) => skip.unwrap_or(0),
                    Err(refusal) => return Some(Box::pin(async move { absent(refusal) })),
                };
                let state = self.library_state();
                let revision = head.library_revision;
                Box::pin(library(hub, skip, state, revision))
            }
            ReadRequest::AgendaRange { .. }
            | ReadRequest::AgendaCalendars { .. }
            | ReadRequest::Sessions { .. }
            | ReadRequest::SessionMessages { .. }
            | ReadRequest::ControlHistory { .. }
            | ReadRequest::EngineTools { .. } => return None,
        };
        Some(answer)
    }
}

/// The hub's order (`views.rows`): newest first, equal starts by id.
fn newest_first(left: &MeetingRow, right: &MeetingRow) -> std::cmp::Ordering {
    right
        .started_at_ms
        .cmp(&left.started_at_ms)
        .then_with(|| left.recording_id.cmp(&right.recording_id))
}

/// One page of the hub's list; the first also carries this Mac's own
/// recordings, which replace the hub's row of the same recording.
async fn list_page(
    hub: Result<NoteHub, Refusal>,
    before: Option<(i64, String)>,
    first: bool,
    local: Vec<MeetingRow>,
    call: Option<String>,
    revision: u64,
) -> Knowledge<Page<Value>> {
    let listed = match hub {
        Ok(hub) => hub
            .list(before.as_ref(), orbit_core::notes::LIST_PAGE)
            .await
            .into_result(),
        Err(refusal) => Err(refusal),
    };
    let (hub_rows, more) = match listed {
        Ok(list) => (list.meetings, list.more),
        Err(refusal) if first => {
            // The hub's part is missing and says why; the Mac's own
            // recordings are still there.
            let mut items = vec![ListItem::HubRefused {
                hub_refusal: refusal,
            }];
            items.extend(local.into_iter().map(ListItem::Meeting));
            let total = items.len() as u64;
            return bounded_page(&items, None, total, revision, ReadId::NotesPage);
        }
        Err(refusal) => return absent(refusal),
    };
    let next = hub_rows
        .last()
        .filter(|_| more)
        .map(|last| Cursor::DescendingDateAndId {
            date_ms: last.started_at_ms,
            id: last.recording_id.clone(),
        });
    let mut rows = hub_rows
        .into_iter()
        .filter(|row| {
            call.as_deref() != Some(row.recording_id.as_str())
                && !local
                    .iter()
                    .any(|mine| mine.recording_id == row.recording_id)
        })
        .collect::<Vec<_>>();
    if first {
        rows.extend(local);
    }
    rows.sort_by(newest_first);
    let total = rows.len() as u64;
    let items = rows.into_iter().map(ListItem::Meeting).collect::<Vec<_>>();
    bounded_page(&items, next, total, revision, ReadId::NotesPage)
}

/// What this Mac knows of one recording.
struct LocalNote {
    row: Option<MeetingRow>,
    event: Option<MeetingEvent>,
    private: Option<orbit_store::PendingPrivateNotes>,
}

async fn detail(
    hub: Result<NoteHub, Refusal>,
    recording: String,
    local: LocalNote,
    revision: u64,
) -> Knowledge<Page<Value>> {
    let read = match hub {
        Ok(hub) => hub.document(&recording).await.into_result(),
        Err(refusal) => Err(refusal),
    };
    let mut document = match read {
        Ok(document) if document.recording_id == recording => document,
        Ok(_) => {
            return absent(Refusal::HubReplyInvalid {
                field: "recording_id".into(),
                received_bytes: None,
            });
        }
        Err(refusal) => match &local.row {
            // Recorded on this Mac and not readable on the hub (not there
            // yet, or the hub cannot be reached): what the Mac knows.
            Some(row) => MeetingDocument {
                dictionary: None,
                recording_id: row.recording_id.clone(),
                title: row.title.clone(),
                started_at_ms: row.started_at_ms,
                duration_ms: row.duration_ms,
                capture: row.capture,
                language: None,
                speakers: Vec::new(),
                transcript: TranscriptProgress {
                    final_pass: None,
                    unmatched_speaker_names: 0,
                    state: TranscriptState::None,
                    pending_chunks: 0,
                    failed_groups: Vec::new(),
                    retranscribing: None,
                },
                note: None,
                private_notes: PrivateNotes::default(),
                local: None,
                event: None,
                hub_refusal: Some(refusal),
            },
            None => return absent(refusal),
        },
    };
    if let Some(row) = &local.row {
        document.local = row.local;
        // SIM-52: named as the list names it. The hub learns a recording's
        // title only from its closing manifest, so until that reaches it its
        // document says «Reunión del…»; this Mac has the title. A recording
        // the hub does not have whole cannot be renamed yet, so no name of
        // Mars's is hidden by this.
        document.title.clone_from(&row.title);
    }
    document.event = local.event;
    if let Some(private) = local.private {
        // What Mars typed and the hub has not confirmed yet.
        document.private_notes = PrivateNotes {
            text: private.markdown,
            revision: document.private_notes.revision,
            pending: true,
        };
    }
    bounded_page(&[document], None, 1, revision, ReadId::NoteDetail)
}

async fn transcript(
    hub: Result<NoteHub, Refusal>,
    recording: String,
    offset: u64,
    local: bool,
    revision: u64,
) -> Knowledge<Page<Value>> {
    let read = match hub {
        Ok(hub) => hub.turns(&recording, offset).await.into_result(),
        Err(refusal) => Err(refusal),
    };
    match read {
        Ok(page) => bounded_page(
            &page.turns,
            page.next_offset.map(|index| Cursor::Index { index }),
            page.total,
            revision,
            ReadId::NoteTranscript,
        ),
        // Not on the hub yet: nothing transcribed, which is a fact here.
        Err(Refusal::NotFound { .. }) if local && offset == 0 => {
            bounded_page(&[] as &[Value], None, 0, revision, ReadId::NoteTranscript)
        }
        Err(refusal) => absent(refusal),
    }
}

async fn library(
    hub: Result<NoteHub, Refusal>,
    skip: usize,
    state: Vec<orbit_core::notes::LibraryItem>,
    revision: u64,
) -> Knowledge<Page<Value>> {
    use orbit_core::notes::{LibraryItem, LibraryList};
    let mut items = Vec::new();
    match &hub {
        Ok(hub) => {
            let dictionaries = match hub.dictionaries().await {
                crate::hub_notes::Answer::Refused { status, .. } => Err(Refusal::UpstreamFailed {
                    service: orbit_core::refusal::Service::Hub,
                    status: Some(status),
                }),
                answer @ (crate::hub_notes::Answer::Value(_)
                | crate::hub_notes::Answer::Unreachable(_)) => answer.into_result(),
            };
            match dictionaries {
                Ok(list) => {
                    items.extend(list.dictionaries.into_iter().map(LibraryItem::Dictionary))
                }
                Err(refusal) => items.push(LibraryItem::Unavailable {
                    list: LibraryList::Dictionaries,
                    refusal,
                }),
            }
            match hub.templates().await.into_result() {
                Ok(list) => items.extend(list.templates.into_iter().map(LibraryItem::Template)),
                Err(refusal) => items.push(LibraryItem::Unavailable {
                    list: LibraryList::Templates,
                    refusal,
                }),
            }
            match hub.links().await.into_result() {
                Ok(links) => items.extend(links.links.into_iter().map(LibraryItem::Link)),
                Err(refusal) => items.push(LibraryItem::Unavailable {
                    list: LibraryList::Links,
                    refusal,
                }),
            }
        }
        Err(refusal) => {
            for list in [
                LibraryList::Templates,
                LibraryList::Links,
                LibraryList::Dictionaries,
            ] {
                items.push(LibraryItem::Unavailable {
                    list,
                    refusal: refusal.clone(),
                });
            }
        }
    }
    items.extend(state);
    super::super::reads::page(
        &items,
        Some(Cursor::Index { index: skip as u64 }),
        ReadId::NoteLibrary,
        revision,
    )
}
