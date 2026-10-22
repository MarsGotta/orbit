use super::*;
use crate::{ChatMessage, ChatRole, ChatSessionSummary, VoiceNote};
use orbit_core::{
    reads::ReadRequest,
    topics::{Cursor, Knowledge, Page},
};

impl DaemonClient {
    /// One page of the conversations the hub lists. The caller owns the
    /// cursor and the list.
    pub async fn read_sessions_page(
        &self,
        cursor: Option<Cursor>,
    ) -> Result<Knowledge<Page<ChatSessionSummary>>, AppError> {
        self.read_page(ReadRequest::Sessions { cursor }, |item| {
            let session: orbit_core::SessionSummary = serde_json::from_value(item)?;
            Ok(ChatSessionSummary {
                id: session.id,
                agent_id: session.agent_id,
                created_at_ms: session.created_at_ms,
                last_at_ms: session.last_at_ms,
                message_count: session.message_count,
            })
        })
        .await
    }

    /// One page of a conversation as the hub keeps it, in order;
    /// without a cursor, the newest one. The screen prepends older pages.
    pub async fn read_session_messages_page(
        &self,
        session_id: String,
        cursor: Option<Cursor>,
    ) -> Result<Knowledge<Page<ChatMessage>>, AppError> {
        self.read_page(
            ReadRequest::SessionMessages { session_id, cursor },
            |item| {
                let message: orbit_core::SessionMessage = serde_json::from_value(item)?;
                Ok(ChatMessage {
                    id: message.id,
                    role: match message.role.as_str() {
                        "user" => ChatRole::User,
                        "assistant" => ChatRole::Assistant,
                        _ => ChatRole::System,
                    },
                    at_ms: message.at_ms.unwrap_or(0),
                    streaming: false,
                    voice: message.voice.map(|voice| VoiceNote {
                        duration_ms: voice.duration_ms,
                        transcript: Some(message.text.clone()),
                        failure: None,
                        audio_available: voice.audio_available,
                    }),
                    text: message.text,
                    receipts: Vec::new(),
                })
            },
        )
        .await
    }

    async fn read_page<T>(
        &self,
        read: ReadRequest,
        item: impl Fn(serde_json::Value) -> Result<T, serde_json::Error>,
    ) -> Result<Knowledge<Page<T>>, AppError> {
        let response = self
            .inner
            .daemon
            .read_topic(read)
            .await
            .map_err(|error| AppError::from_daemon(error.as_ref()))?;
        let answer = response.answer.ok_or_else(AppError::unavailable)?;
        match answer {
            Knowledge::Unknown { why } => Ok(Knowledge::Unknown { why }),
            Knowledge::Absent { refusal } => Ok(Knowledge::Absent { refusal }),
            Knowledge::Present { value, as_of_ms } => {
                let items = value
                    .items
                    .into_iter()
                    .map(item)
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|error| AppError::daemon(format!("Página inválida: {error}")))?;
                Ok(Knowledge::Present {
                    value: Page {
                        items,
                        next: value.next,
                        total: value.total,
                        revision: value.revision,
                        store_version: value.store_version,
                    },
                    as_of_ms,
                })
            }
        }
    }
}
