//! Keyed reads. Agenda pages come from the daemon's complete native read;
//! control pages from the engine record; the conversations and their
//! messages from one HTTP hub page each (SIM-30). The notes reads are the
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

/// The IPC keyed-read window is 10 s. Three synthetic four-page sweeps at
/// load 19.24–19.73 took at most 106 ms per page. Eight seconds lets a slow hub
/// use most of the existing window while leaving two for IPC scheduling and
/// a typed timeout reply. This protects the reply, not a latency target,
/// conversation retention, or a user's action.
const SESSION_READ_DEADLINE: Duration = Duration::from_secs(8);

/// SIM-30: the store reader that tells a conversation's voice notes whose
/// audio this Mac still holds. One of its own, so a page read off the actor
/// never waits on the actor's reader; a page asks it once per voice note.
pub(super) type VoiceAudioReader = std::sync::Arc<std::sync::Mutex<orbit_store::ReadOnlyStore>>;

/// SIM-30: a read the daemon makes of the hub's conversations, captured on
/// the actor and answered off it, so a slow hub never blocks audio.
pub(super) struct HubReadContext {
    settings: crate::config::Settings,
    token: Option<String>,
    development: bool,
    revision: u64,
    privacy_stopped: bool,
    connection: orbit_core::Connection,
    /// The op of the read's order: its request to the hub carries it.
    op: orbit_core::notice::Op,
}

impl HubReadContext {
    pub(super) fn capture(runtime: &Runtime, revision: u64) -> Self {
        Self {
            settings: runtime.settings.clone(),
            token: runtime.token.clone(),
            development: runtime.development,
            revision,
            privacy_stopped: runtime.state.privacy_stopped(),
            connection: runtime.state.connection(),
            op: runtime.turn.op.clone(),
        }
    }

    /// Why the hub is not asked at all, if it is not. Without the hub there
    /// is no chat (Mars, 2026-10-15): offline, the read says so.
    fn refused_before_asking(&self) -> Result<String, Refusal> {
        crate::hub::access(
            &self.settings,
            self.token.as_deref(),
            self.privacy_stopped,
            self.connection.clone(),
        )
        .map(str::to_owned)
        .map_err(|refusal| {
            // Conversation reads historically name a revoked pairing;
            // note reads name its missing link. Preserve that distinction.
            if self.connection == orbit_core::Connection::Revoked
                && matches!(refusal, Refusal::HubUnreachable { .. })
            {
                Refusal::NotPaired
            } else {
                refusal
            }
        })
    }

    /// One page of the conversations, as the hub lists them.
    pub(super) async fn sessions(
        self,
        cursor: Option<Cursor>,
    ) -> Knowledge<Page<serde_json::Value>> {
        let token = match self.refused_before_asking() {
            Ok(token) => token,
            Err(refusal) => return Knowledge::Absent { refusal },
        };
        let before = match cursor_value(cursor.as_ref(), |cursor| {
            if let Cursor::Id { id } = cursor {
                orbit_core::wire::canonical_uuid(id).then(|| id.clone())
            } else {
                None
            }
        }) {
            Ok(before) => before,
            Err(refusal) => return Knowledge::Absent { refusal },
        };
        let started = Instant::now();
        let listing = match tokio::time::timeout(
            SESSION_READ_DEADLINE,
            crate::hub::sessions_page(
                &self.settings,
                &token,
                self.development,
                before.as_deref(),
                &self.op,
            ),
        )
        .await
        {
            Ok(Ok(listing)) => listing,
            Ok(Err(error)) => {
                return Knowledge::Absent {
                    refusal: hub_refusal(&error, started, Operation::SessionList),
                };
            }
            Err(_) => {
                return Knowledge::Absent {
                    refusal: timed_out(started, Operation::SessionList),
                };
            }
        };
        let delivered = listing.sessions.len() as u64;
        // A cursor that repeats would walk the list forever.
        if before.as_ref().is_some_and(|id| {
            listing.sessions.iter().any(|session| &session.id == id)
                || listing.next.as_ref() == Some(id)
        }) {
            return Knowledge::Absent {
                refusal: Refusal::HubSessionsUnpaged {
                    delivered,
                    total: listing.total,
                },
            };
        }
        let rows = listing
            .sessions
            .into_iter()
            .map(|session| orbit_core::SessionSummary {
                id: session.id,
                created_at_ms: session.created_at_ms,
                last_at_ms: session.last_at_ms,
                message_count: session.message_count,
            })
            .collect::<Vec<_>>();
        bounded_page(
            &rows,
            listing.next.map(|id| Cursor::Id { id }),
            listing.total,
            self.revision,
            ReadId::Sessions,
        )
    }

    /// SIM-30: one page of a conversation, newest first without a cursor.
    /// Each voice note says whether this Mac still holds its audio.
    pub(super) async fn messages(
        self,
        session_id: String,
        cursor: Option<Cursor>,
        voice_audio: VoiceAudioReader,
    ) -> Knowledge<Page<serde_json::Value>> {
        let token = match self.refused_before_asking() {
            Ok(token) => token,
            Err(refusal) => return Knowledge::Absent { refusal },
        };
        if !orbit_core::wire::canonical_uuid(&session_id) {
            return Knowledge::Absent {
                refusal: Refusal::InvalidParameter {
                    parameter: Parameter::Identifier,
                    expected: Expected::ValidIdentifier,
                },
            };
        }
        let before = match cursor_value(cursor.as_ref(), |cursor| {
            if let Cursor::Id { id } = cursor {
                id.parse::<i64>().ok()
            } else {
                None
            }
        }) {
            Ok(before) => before,
            Err(refusal) => return Knowledge::Absent { refusal },
        };
        let started = Instant::now();
        let page = match tokio::time::timeout(
            SESSION_READ_DEADLINE,
            crate::hub::session_messages_page(
                &self.settings,
                &token,
                self.development,
                &session_id,
                before,
                &self.op,
            ),
        )
        .await
        {
            Ok(Ok(page)) => page,
            Ok(Err(error)) => {
                return Knowledge::Absent {
                    refusal: hub_refusal(&error, started, Operation::Chat),
                };
            }
            Err(_) => {
                return Knowledge::Absent {
                    refusal: timed_out(started, Operation::Chat),
                };
            }
        };
        let voice_ids = page
            .messages
            .iter()
            .filter(|message| message.kind == "voice")
            .filter_map(|message| message.client_id.clone())
            .collect::<Vec<_>>();
        let held = match tokio::task::spawn_blocking(move || {
            let store = voice_audio
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            voice_ids
                .into_iter()
                .filter(|id| {
                    store
                        .list_chunks(id, None, 2)
                        .is_ok_and(|chunks| super::audio::voice_note_chunk(chunks).is_some())
                })
                .collect::<std::collections::BTreeSet<_>>()
        })
        .await
        {
            Ok(held) => held,
            Err(error) => {
                return Knowledge::Absent {
                    refusal: Refusal::StorageFailed {
                        operation: Operation::Storage,
                        cause: Some(
                            format!("La lectura local terminó sin respuesta: {error}").into(),
                        ),
                    },
                };
            }
        };
        let total = page.total;
        let next = page.next.map(|id| Cursor::Id { id: id.to_string() });
        let messages = page
            .messages
            .into_iter()
            .map(|message| {
                let voice = (message.kind == "voice").then(|| orbit_core::SessionVoice {
                    duration_ms: message.duration_ms.unwrap_or(0),
                    audio_available: message
                        .client_id
                        .as_ref()
                        .is_some_and(|id| held.contains(id)),
                });
                orbit_core::SessionMessage {
                    id: message.client_id.unwrap_or_else(|| message.id.to_string()),
                    role: message.role,
                    text: message.text,
                    at_ms: message.at_ms,
                    voice,
                }
            })
            .collect::<Vec<_>>();
        bounded_page(
            &messages,
            next,
            total,
            self.revision,
            ReadId::SessionMessages,
        )
    }
}

/// The page the hub gave, whole, under the read's ceiling, or refused with
/// its size.
pub(super) fn bounded_page<T: Serialize>(
    items: &[T],
    next: Option<Cursor>,
    total: u64,
    revision: u64,
    read: ReadId,
) -> Knowledge<Page<serde_json::Value>> {
    let items = match items
        .iter()
        .map(serde_json::to_value)
        .collect::<Result<Vec<_>, _>>()
    {
        Ok(items) => items,
        Err(_) => return serialization_failed(),
    };
    let answer = Knowledge::Present {
        value: Page {
            items,
            next,
            total,
            revision,
            store_version: None,
        },
        as_of_ms: None,
    };
    match serde_json::to_vec(&answer) {
        Ok(bytes) if bytes.len() <= read.max_bytes() => answer,
        Ok(bytes) => Knowledge::Absent {
            refusal: Refusal::TooLarge {
                resource: Resource::IpcResponse,
                bytes: u64::try_from(bytes.len()).unwrap_or(u64::MAX),
                max_bytes: read.max_bytes() as u64,
            },
        },
        Err(_) => serialization_failed(),
    }
}

fn timed_out(started: Instant, operation: Operation) -> Refusal {
    Refusal::Timeout {
        operation,
        waited_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
        deadline_ms: SESSION_READ_DEADLINE.as_millis() as u64,
    }
}

/// What the hub's refusal of a read was, with its numbers.
fn hub_refusal(error: &anyhow::Error, started: Instant, operation: Operation) -> Refusal {
    if let Some(failure) = error.downcast_ref::<crate::hub::HubBodyTooLarge>() {
        Refusal::TooLarge {
            resource: Resource::HubResponse,
            bytes: failure.bytes,
            max_bytes: failure.max_bytes,
        }
    } else if let Some(failure) = error.downcast_ref::<crate::hub::HubSessionInvalidRow>() {
        Refusal::HubSessionInvalidRow {
            position: failure.position,
            rows: failure.rows,
            field: failure.field,
            bytes: failure.bytes,
            max_bytes: failure.max_bytes,
        }
    } else if let Some(failure) = error.downcast_ref::<crate::hub::HubSessionsCursorMismatch>() {
        Refusal::HubSessionsCursorMismatch { rows: failure.rows }
    } else if let Some(failure) = error.downcast_ref::<crate::hub::HubReadStatus>() {
        match (failure.status, failure.bytes, failure.max_bytes) {
            (404, ..) => Refusal::NotFound {
                resource: Resource::Session,
            },
            (413, Some(bytes), Some(max_bytes)) => Refusal::TooLarge {
                resource: Resource::Message,
                bytes,
                max_bytes,
            },
            (status, ..) => Refusal::UpstreamFailed {
                service: orbit_core::refusal::Service::Hub,
                status: Some(status),
            },
        }
    } else if let Some(failure) = error.downcast_ref::<reqwest::Error>() {
        match crate::hub::TransportCause::of(failure) {
            crate::hub::TransportCause::ConnectionTimeout | crate::hub::TransportCause::Timeout => {
                timed_out(started, operation)
            }
            crate::hub::TransportCause::Connection => Refusal::HubUnreachable { since_ms: None },
            crate::hub::TransportCause::Cut => Refusal::UpstreamFailed {
                service: orbit_core::refusal::Service::Hub,
                status: None,
            },
        }
    } else {
        Refusal::UpstreamFailed {
            service: orbit_core::refusal::Service::Hub,
            status: None,
        }
    }
}

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
        | ReadRequest::NoteLibrary { .. } => Knowledge::Unknown {
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

fn agenda_calendars(
    runtime: &Runtime,
    cursor: Option<Cursor>,
    revision: u64,
) -> Knowledge<Page<serde_json::Value>> {
    if let Some(refusal) = runtime.features.calendar.inventory_refusal() {
        return Knowledge::Absent { refusal };
    }
    match runtime.calendar_access() {
        Knowledge::Unknown { why } => return Knowledge::Unknown { why },
        Knowledge::Absent { refusal } => return Knowledge::Absent { refusal },
        Knowledge::Present { .. } => {}
    }
    if !runtime.features.calendar.inventory_valid {
        return Knowledge::Unknown {
            why: Pending::NotObservedYet,
        };
    }
    let Some(calendars) = runtime.features.calendar.view.calendars.as_ref() else {
        return Knowledge::Unknown {
            why: Pending::NotObservedYet,
        };
    };
    let mut sorted: Vec<_> = calendars.iter().collect();
    sorted.sort_by(|left, right| left.reference.calendar_id.cmp(&right.reference.calendar_id));
    let start = match cursor_value(cursor.as_ref(), |cursor| {
        if let Cursor::Id { id } = cursor {
            orbit_core::calendar::CalendarReference::valid_id(id)
                .then(|| sorted.partition_point(|entry| entry.reference.calendar_id <= *id))
        } else {
            None
        }
    }) {
        Ok(start) => start.unwrap_or(0),
        Err(refusal) => return Knowledge::Absent { refusal },
    };
    let total = sorted.len();
    paginate(
        sorted.iter().enumerate().skip(start).map(|(index, entry)| {
            let next = (index + 1 < total).then(|| Cursor::Id {
                id: entry.reference.calendar_id.clone(),
            });
            (entry, next)
        }),
        total as u64,
        revision,
        None,
        ReadId::AgendaCalendars,
    )
}

fn agenda_range(
    runtime: &Runtime,
    start_ms: i64,
    end_ms: i64,
    cursor: Option<Cursor>,
    revision: u64,
) -> Knowledge<Page<serde_json::Value>> {
    if (CalendarWindow {
        start_unix_ms: start_ms,
        end_unix_ms: end_ms,
    })
    .validate()
    .is_err()
    {
        return Knowledge::Absent {
            refusal: Refusal::InvalidParameter {
                parameter: Parameter::Request,
                expected: Expected::CalendarRange {
                    start_ms,
                    end_ms,
                    min_ms: MIN_UNIX_MS,
                    max_ms: MAX_UNIX_MS,
                },
            },
        };
    }
    if runtime
        .features
        .calendar
        .topic_window()
        .as_ref()
        .is_none_or(|range| range.start_unix_ms == start_ms && range.end_unix_ms == end_ms)
        && let Some(refusal) = &runtime.features.calendar.read_failure
    {
        return Knowledge::Absent {
            refusal: refusal.clone(),
        };
    }
    match runtime.calendar_access() {
        Knowledge::Unknown { why } => return Knowledge::Unknown { why },
        Knowledge::Absent { refusal } => return Knowledge::Absent { refusal },
        Knowledge::Present { .. } => {}
    }
    let Some(snapshot) = runtime.features.calendar.complete.as_deref() else {
        return Knowledge::Unknown {
            why: Pending::NotObservedYet,
        };
    };
    if snapshot.window.start_unix_ms != start_ms || snapshot.window.end_unix_ms != end_ms {
        return Knowledge::Unknown {
            why: Pending::NotObservedYet,
        };
    }
    let after = match cursor_value(cursor.as_ref(), |cursor| {
        if let Cursor::StartAndId { start_ms, id } = cursor {
            Some((*start_ms, id.clone()))
        } else {
            None
        }
    }) {
        Ok(after) => after,
        Err(refusal) => return Knowledge::Absent { refusal },
    };
    let mut events: Vec<_> = snapshot.events.iter().collect();
    events.sort_by(|left, right| {
        (left.start_unix_ms, left.observation_id.as_str())
            .cmp(&(right.start_unix_ms, right.observation_id.as_str()))
    });
    if let Some((start, id)) = &after
        && !events
            .iter()
            .any(|event| event.start_unix_ms == *start && event.observation_id == *id)
    {
        return Knowledge::Absent {
            refusal: Refusal::InvalidParameter {
                parameter: Parameter::Identifier,
                expected: Expected::ValidIdentifier,
            },
        };
    }
    let total = u64::try_from(events.len()).unwrap_or(u64::MAX);
    let pending: Vec<_> = events
        .into_iter()
        .filter(|event| {
            after.as_ref().is_none_or(|(start, id)| {
                (event.start_unix_ms, event.observation_id.as_str()) > (*start, id.as_str())
            })
        })
        .collect();
    paginate(
        pending.iter().enumerate().map(|(index, event)| {
            let next = (index + 1 < pending.len()).then(|| Cursor::StartAndId {
                start_ms: event.start_unix_ms,
                id: event.observation_id.clone(),
            });
            (event, next)
        }),
        total,
        revision,
        Some(snapshot.observed_at_unix_ms),
        ReadId::AgendaRange,
    )
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
/// catalogue already pages toward Deimos (DEI-13). Without a connected
/// engine there is no catalogue to page, and that is a refusal, not an
/// empty list.
fn engine_tools(runtime: &Runtime, cursor: Option<Cursor>) -> Knowledge<Page<serde_json::Value>> {
    let skip = match index_cursor(cursor) {
        Ok(skip) => skip,
        Err(refusal) => return Knowledge::Absent { refusal },
    };
    match engine_knowledge_with_permissions(
        runtime.engine_status(),
        runtime.engine_failure_cause(),
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
mod ref207_size_tests {
    use super::*;

    #[test]
    fn accumulated_page_size_matches_serialized_json_at_the_ceiling()
    -> Result<(), Box<dyn std::error::Error>> {
        let cursor = Some(Cursor::StartAndId {
            start_ms: 1_800_000_000_000,
            id: "55555555-5555-4555-8555-000000000003".into(),
        });
        let answer = |items: Vec<serde_json::Value>, next: Option<Cursor>| Knowledge::Present {
            value: Page {
                items,
                next,
                total: 3,
                revision: 7,
                store_version: None,
            },
            as_of_ms: Some(1_800_000_000_000),
        };
        let ceiling = ReadRequest::AgendaRange {
            start_ms: 1_800_000_000_000,
            end_ms: 1_802_678_400_000,
            cursor: None,
        }
        .id()
        .max_bytes();
        let mut items = vec![
            serde_json::json!("alpha"),
            serde_json::json!("beta"),
            serde_json::json!(""),
        ];
        let small = serde_json::to_vec(&answer(items.clone(), cursor.clone()))?.len();
        let fill = ceiling
            .checked_sub(small)
            .ok_or("fixture already exceeds ceiling")?;
        items[2] = serde_json::json!("x".repeat(fill));
        let actual = serde_json::to_vec(&answer(items.clone(), cursor.clone()))?.len();
        assert_eq!(actual, ceiling);

        let base_bytes = serde_json::to_vec(&answer(Vec::new(), None))?.len();
        let next_bytes = serde_json::to_vec(&cursor)?.len();
        let mut accepted_item_bytes = 0;
        let mut estimated = base_bytes;
        for (index, item) in items.iter().enumerate() {
            estimated = candidate_page_bytes(
                base_bytes,
                accepted_item_bytes,
                index,
                serde_json::to_vec(item)?.len(),
                next_bytes,
            );
            accepted_item_bytes = estimated - base_bytes - (next_bytes - 4);
        }
        assert_eq!(estimated, actual);
        Ok(())
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod ref211_tests {
    use super::*;

    #[test]
    fn session_deadline_is_guarded_against_the_ipc_read_window() {
        assert!(SESSION_READ_DEADLINE < orbit_ipc::KEYED_READ_TIMEOUT);
    }

    /// SIM-30: the hub's whole page of conversations, 128 rows with
    /// canonical ids and the longest numbers, fits the `Sessions` read.
    #[test]
    fn sim30_a_full_sessions_page_fits_its_read() {
        let rows = (0..128)
            .map(|index| orbit_core::SessionSummary {
                id: format!("00000000-0000-4000-8000-{index:012}"),
                created_at_ms: i64::MAX,
                last_at_ms: i64::MAX,
                message_count: u64::MAX,
            })
            .collect::<Vec<_>>();
        let next = Some(Cursor::Id {
            id: rows[127].id.clone(),
        });
        let answer = bounded_page(&rows, next, u64::MAX, u64::MAX, ReadId::Sessions);
        let bytes = serde_json::to_vec(&answer).unwrap().len();
        eprintln!("SIM-30: a full sessions page is {bytes} bytes");
        assert!(matches!(answer, Knowledge::Present { .. }), "{answer:?}");
        assert!(bytes <= ReadId::Sessions.max_bytes());
    }

    /// SIM-30: a hub page at its 512 KiB ceiling of JSON, in characters that
    /// escape the same on both sides, still fits the `SessionMessages` read
    /// with the ids and `audio_available` the daemon adds.
    #[test]
    fn sim30_a_full_messages_page_fits_its_read() {
        let text = "\"".repeat(10 * 1024);
        let message = |index: usize| orbit_core::SessionMessage {
            id: format!("00000000-0000-4000-8000-{index:012}"),
            role: "user".into(),
            text: text.clone(),
            at_ms: Some(i64::MAX),
            voice: Some(orbit_core::SessionVoice {
                duration_ms: u64::MAX,
                audio_available: true,
            }),
        };
        // As many as the hub's page holds: 512 KiB less its 80-byte envelope.
        let mut messages = Vec::new();
        while serde_json::to_vec(&[&messages[..], &[message(messages.len())]].concat())
            .unwrap()
            .len()
            <= 512 * 1024 - 80
        {
            messages.push(message(messages.len()));
        }
        let hub_bytes = serde_json::to_vec(&messages).unwrap().len();
        assert!(hub_bytes >= 500 * 1024, "{hub_bytes}");
        let next = Some(Cursor::Id {
            id: i64::MAX.to_string(),
        });
        let answer = bounded_page(&messages, next, u64::MAX, u64::MAX, ReadId::SessionMessages);
        let bytes = serde_json::to_vec(&answer).unwrap().len();
        eprintln!("SIM-30: a full messages page is {bytes} bytes ({hub_bytes} from the hub)");
        assert!(matches!(answer, Knowledge::Present { .. }));
        assert!(bytes <= ReadId::SessionMessages.max_bytes());
    }
}
