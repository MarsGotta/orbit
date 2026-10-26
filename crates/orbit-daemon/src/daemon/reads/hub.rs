//! The hub's conversations read off the actor: the sessions and a session's
//! messages, under their deadline and their page's ceiling.
use super::*;

/// A read the daemon makes of the hub's conversations, captured on
/// the actor and answered off it, so a slow hub never blocks audio.
pub(in crate::daemon) struct HubReadContext {
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
    pub(in crate::daemon) fn capture(runtime: &Runtime, revision: u64) -> Self {
        Self {
            settings: runtime.settings.clone(),
            token: runtime.link.token.clone(),
            development: runtime.development,
            revision,
            privacy_stopped: runtime.state.privacy_stopped(),
            connection: runtime.state.connection(),
            op: runtime.turn.op.clone(),
        }
    }

    /// Why the hub is not asked at all, if it is not. Without the hub there
    /// is no chat (a product decision of 2026-10-15): offline, the read says so.
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
    pub(in crate::daemon) async fn sessions(
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
                agent_id: session.agent_id,
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

    /// One page of a conversation, newest first without a cursor.
    /// Each voice note says whether this Mac still holds its audio.
    pub(in crate::daemon) async fn messages(
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
        let held = match held_voice_audio(voice_audio, voice_ids).await {
            Ok(held) => held,
            Err(refusal) => return Knowledge::Absent { refusal },
        };
        let total = page.total;
        let next = page.next.map(|id| Cursor::Id { id: id.to_string() });
        let messages = page
            .messages
            .into_iter()
            .map(|message| session_message(message, &held))
            .collect::<Vec<_>>();
        bounded_conversation_page(
            &messages,
            next,
            total,
            self.revision,
            ReadId::SessionMessages,
            page.handed_over_from.map(Box::new),
        )
    }
}

/// Which of a page's voice notes this Mac still holds the audio of, read
/// off the actor with its own reader.
async fn held_voice_audio(
    voice_audio: VoiceAudioReader,
    voice_ids: Vec<String>,
) -> Result<std::collections::BTreeSet<String>, Refusal> {
    tokio::task::spawn_blocking(move || {
        let store = voice_audio
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        voice_ids
            .into_iter()
            .filter(|id| {
                store.list_chunks(id, None, 2).is_ok_and(|chunks| {
                    super::super::voice_notes::voice_note_chunk(chunks).is_some()
                })
            })
            .collect::<std::collections::BTreeSet<_>>()
    })
    .await
    .map_err(|error| Refusal::StorageFailed {
        operation: Operation::Storage,
        cause: Some(format!("La lectura local terminó sin respuesta: {error}").into()),
    })
}

/// One of the hub's messages as the app reads it; a voice note says
/// whether its audio is still here.
fn session_message(
    message: crate::hub::HubMessage,
    held: &std::collections::BTreeSet<String>,
) -> orbit_core::SessionMessage {
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
        attachments: message.attachments,
        work: message.work,
    }
}

/// The page the hub gave, whole, under the read's ceiling, or refused with
/// its size.
pub(in crate::daemon) fn bounded_page<T: Serialize>(
    items: &[T],
    next: Option<Cursor>,
    total: u64,
    revision: u64,
    read: ReadId,
) -> Knowledge<Page<serde_json::Value>> {
    bounded_conversation_page(items, next, total, revision, read, None)
}

fn bounded_conversation_page<T: Serialize>(
    items: &[T],
    next: Option<Cursor>,
    total: u64,
    revision: u64,
    read: ReadId,
    handed_over_from: Option<Box<orbit_core::topics::SessionOrigin>>,
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
            handed_over_from,
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

pub(super) fn timed_out(started: Instant, operation: Operation) -> Refusal {
    Refusal::Timeout {
        operation,
        waited_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
        deadline_ms: SESSION_READ_DEADLINE.as_millis() as u64,
    }
}

/// What the hub's refusal of a read was, with its numbers.
pub(super) fn hub_refusal(
    error: &anyhow::Error,
    started: Instant,
    operation: Operation,
) -> Refusal {
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
