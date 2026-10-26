//! Keyed reads. Agenda pages come from the daemon's complete native read;
//! control pages from the engine record; the conversations and their
//! messages from one HTTP hub page each. The notes reads are the
//! hub's (`recording_notes::reads`).
use orbit_core::{
    calendar::{CalendarWindow, MAX_UNIX_MS, MIN_UNIX_MS},
    reads::{ReadId, ReadRequest},
    refusal::{EngineCause, Expected, Operation, Parameter, Refusal, Resource},
    topics::{
        Cursor, Knowledge, Page, Pending,
        control::{Engine, engine_knowledge_with_permissions},
    },
};

use serde::Serialize;
use std::time::{Duration, Instant};

use super::Runtime;

mod hub;
pub(super) use hub::{HubReadContext, bounded_page};

mod agenda;
use agenda::{agenda_calendars, agenda_range};

/// The IPC keyed-read window is 10 s. Three synthetic four-page sweeps at
/// load 19.24–19.73 took at most 106 ms per page. Eight seconds lets a slow hub
/// use most of the existing window while leaving two for IPC scheduling and
/// a typed timeout reply. This protects the reply, not a latency target,
/// conversation retention, or a user's action.
const SESSION_READ_DEADLINE: Duration = Duration::from_secs(8);

/// The store reader that tells a conversation's voice notes whose
/// audio this Mac still holds. One of its own, so a page read off the actor
/// never waits on the actor's reader; a page asks it once per voice note.
pub(super) type VoiceAudioReader = std::sync::Arc<std::sync::Mutex<orbit_store::ReadOnlyStore>>;

pub(super) fn dispatch(
    runtime: &mut Runtime,
    read: ReadRequest,
    revision: u64,
) -> Knowledge<Page<serde_json::Value>> {
    match read {
        ReadRequest::AgendaRange {
            start_ms,
            end_ms,
            cursor,
        } => agenda_range(runtime, start_ms, end_ms, cursor, revision),
        ReadRequest::ControlHistory { cursor } => control_history(runtime, cursor),
        ReadRequest::EngineTools { cursor } => engine_tools(runtime, cursor),
        // The IPC loop answers these with remote reads (`notes_read`).
        ReadRequest::NotesPage { .. }
        | ReadRequest::NoteDetail { .. }
        | ReadRequest::NoteTranscript { .. }
        | ReadRequest::NoteLibrary { .. }
        | ReadRequest::SeriesPrevious { .. }
        | ReadRequest::PersonalReminder { .. } => Knowledge::Unknown {
            why: Pending::NotObservedYet,
        },
        // The IPC loop answers these with one asynchronous hub page, so a
        // slow hub cannot block audio or other daemon events.
        ReadRequest::Sessions { .. } | ReadRequest::SessionMessages { .. } => Knowledge::Unknown {
            why: Pending::NotObservedYet,
        },
        ReadRequest::AgendaCalendars { cursor } => agenda_calendars(runtime, cursor, revision),
    }
}

fn candidate_page_bytes(
    base_bytes: usize,
    accepted_item_bytes: usize,
    accepted_items: usize,
    new_item_bytes: usize,
    next_bytes: usize,
) -> usize {
    base_bytes
        .saturating_add(accepted_item_bytes)
        .saturating_add(usize::from(accepted_items > 0))
        .saturating_add(new_item_bytes)
        .saturating_add(next_bytes.saturating_sub(4))
}

/// The common 203 B entries fit in one page. Valid maximal entries reach
/// about 690 B each, so the 96 KiB response ceiling needs a byte cursor.
/// This bounds one IPC response, not the durable 256-entry record. The page has no
/// revision of its own — the local record has no counter — so readers
/// validate it against the control topic's revision, which is what the
/// app-core cache already does.
fn control_history(
    runtime: &Runtime,
    cursor: Option<Cursor>,
) -> Knowledge<Page<serde_json::Value>> {
    let history = match runtime.control_history() {
        Ok(history) => history,
        Err(error) => {
            return Knowledge::Absent {
                refusal: Refusal::StorageFailed {
                    operation: Operation::Storage,
                    cause: Some(error.to_string().into()),
                },
            };
        }
    };
    let total = history.len();
    let skip = match cursor_value(cursor.as_ref(), |cursor| {
        if let Cursor::Id { id } = cursor {
            history
                .iter()
                .position(|entry| entry.operation_id == *id)
                .map(|index| index + 1)
        } else {
            None
        }
    }) {
        Ok(skip) => skip.unwrap_or(0),
        Err(refusal) => return Knowledge::Absent { refusal },
    };
    if skip > 0 && skip >= total {
        return invalid_cursor();
    }
    paginate(
        history
            .iter()
            .enumerate()
            .skip(skip)
            .map(|(offset, entry)| {
                let next = (offset + 1 < total).then(|| Cursor::Id {
                    id: entry.operation_id.clone(),
                });
                (entry, next)
            }),
        total as u64,
        0,
        None,
        ReadId::ControlHistory,
    )
}

/// The engine's declared tool names, one page: MAX_ENGINE_TOOLS names of at
/// most MAX_TOOL_NAME_BYTES fit the 96 KiB ceiling whole, matching how the
/// catalogue already pages toward Deimos. Without a connected
/// engine there is no catalogue to page, and that is a refusal, not an
/// empty list.
fn engine_tools(runtime: &Runtime, cursor: Option<Cursor>) -> Knowledge<Page<serde_json::Value>> {
    let skip = match index_cursor(cursor) {
        Ok(skip) => skip,
        Err(refusal) => return Knowledge::Absent { refusal },
    };
    match engine_knowledge_with_permissions(
        runtime.engine_status(),
        runtime.computer.engine_failure_cause(),
        runtime.state.capabilities().accessibility_state(),
        runtime.state.capabilities().screen_state(),
    ) {
        Knowledge::Absent { refusal } => return Knowledge::Absent { refusal },
        Knowledge::Unknown { why } => return Knowledge::Unknown { why },
        Knowledge::Present {
            value: Engine::Starting,
            ..
        } => {
            return Knowledge::Absent {
                refusal: Refusal::EngineUnavailable {
                    cause: EngineCause::NotRunning,
                },
            };
        }
        Knowledge::Present {
            value: Engine::Ready { .. },
            ..
        } => {}
    }
    let names: Vec<String> = runtime
        .computer
        .engine_tools()
        .into_iter()
        .map(|tool| tool.name)
        .collect();
    let total = names.len();
    if skip > 0 && skip >= total {
        return invalid_cursor();
    }
    let page: Vec<serde_json::Value> = names
        .into_iter()
        .skip(skip)
        .take(orbit_core::computer_control::MAX_ENGINE_TOOLS)
        .map(serde_json::Value::String)
        .collect();
    Knowledge::Present {
        value: finish_page(page, total),
        as_of_ms: None,
    }
}

/// Fills the shared page tail. One page always holds the whole bounded
/// collection here, so there is no next cursor to compute.
fn finish_page(items: Vec<serde_json::Value>, total: usize) -> Page<serde_json::Value> {
    Page {
        items,
        next: None,
        total: total as u64,
        revision: 0,
        store_version: None,
        handed_over_from: None,
    }
}

pub(super) fn cursor_error() -> Refusal {
    Refusal::InvalidParameter {
        parameter: Parameter::Identifier,
        expected: Expected::ValidIdentifier,
    }
}

/// Absence means the first page; a present cursor must have the read's shape.
pub(super) fn cursor_value<T>(
    cursor: Option<&Cursor>,
    decode: impl FnOnce(&Cursor) -> Option<T>,
) -> Result<Option<T>, Refusal> {
    cursor
        .map(|cursor| decode(cursor).ok_or_else(cursor_error))
        .transpose()
}

fn invalid_cursor<T>() -> Knowledge<T> {
    Knowledge::Absent {
        refusal: cursor_error(),
    }
}

fn index_cursor(cursor: Option<Cursor>) -> Result<usize, Refusal> {
    cursor_value(cursor.as_ref(), |cursor| {
        if let Cursor::Index { index } = cursor {
            usize::try_from(*index).ok()
        } else {
            None
        }
    })
    .map(|index| index.unwrap_or(0))
}

fn serialization_failed<T>() -> Knowledge<T> {
    Knowledge::Absent {
        refusal: Refusal::EncodeFailed {
            resource: Resource::IpcResponse,
        },
    }
}

pub(super) fn page<T: Serialize>(
    items: &[T],
    cursor: Option<Cursor>,
    id: ReadId,
    revision: u64,
) -> Knowledge<Page<serde_json::Value>> {
    let start = match index_cursor(cursor) {
        Ok(index) if index <= items.len() => index,
        Ok(_) | Err(_) => {
            return Knowledge::Absent {
                refusal: Refusal::InvalidParameter {
                    parameter: Parameter::Selection,
                    expected: Expected::ValidSelection,
                },
            };
        }
    };
    paginate(
        items.iter().enumerate().skip(start).map(|(index, item)| {
            let next = (index + 1 < items.len()).then_some(Cursor::Index {
                index: (index + 1) as u64,
            });
            (item, next)
        }),
        items.len() as u64,
        revision,
        None,
        id,
    )
}

/// Each candidate is encoded once, including its continuation. The bound is
/// the serialized IPC knowledge envelope, not the collection or its rows.
fn paginate<T: Serialize>(
    rows: impl IntoIterator<Item = (T, Option<Cursor>)>,
    total: u64,
    revision: u64,
    as_of_ms: Option<i64>,
    read: ReadId,
) -> Knowledge<Page<serde_json::Value>> {
    let mut page = Page {
        items: Vec::new(),
        next: None,
        total,
        revision,
        store_version: None,
        handed_over_from: None,
    };
    let Ok(base_bytes) = serde_json::to_vec(&Knowledge::Present {
        value: &page,
        as_of_ms,
    }) else {
        return serialization_failed();
    };
    let mut accepted_bytes = 0_usize;
    for (item, next) in rows {
        let Ok(value) = serde_json::to_value(item) else {
            return serialization_failed();
        };
        let Ok(item_bytes) = serde_json::to_vec(&value) else {
            return serialization_failed();
        };
        let Ok(next_bytes) = serde_json::to_vec(&next) else {
            return serialization_failed();
        };
        let bytes = candidate_page_bytes(
            base_bytes.len(),
            accepted_bytes,
            page.items.len(),
            item_bytes.len(),
            next_bytes.len(),
        );
        if bytes > read.max_bytes() {
            if page.items.is_empty() {
                return Knowledge::Absent {
                    refusal: Refusal::TooLarge {
                        resource: Resource::IpcResponse,
                        bytes: bytes as u64,
                        max_bytes: read.max_bytes() as u64,
                    },
                };
            }
            break;
        }
        accepted_bytes = accepted_bytes
            .saturating_add(item_bytes.len())
            .saturating_add(usize::from(!page.items.is_empty()));
        page.items.push(value);
        page.next = next;
    }
    Knowledge::Present {
        value: page,
        as_of_ms,
    }
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests;

#[cfg(test)]
mod size_tests;

#[cfg(test)]
mod hub_page_tests;
