//! The conversations the Mac reads from the hub, a page at a time: the
//! list of them and the messages of one.
use super::*;
/// The hub's session ids are UUIDs; this is room for what `open_session`
/// accepts, no more.
use orbit_core::limits::MAX_SESSION_ID_BYTES;
use orbit_core::refusal::HubSessionField;

#[derive(Debug, thiserror::Error)]
#[error(
    "La sesión {position} de las {rows} filas de la página del hub tiene un {field} de {bytes} bytes; se admiten de 1 a {max_bytes}"
)]
pub(crate) struct HubSessionInvalidRow {
    pub position: u64,
    pub rows: u64,
    pub field: HubSessionField,
    pub bytes: u64,
    pub max_bytes: u64,
}

#[derive(Debug, thiserror::Error)]
#[error("El cursor de la lista de {rows} sesiones no coincide con su última fila")]
pub(crate) struct HubSessionsCursorMismatch {
    pub rows: u64,
}

/// One conversation as the hub lists it. The app writes its title,
/// the date, from `created_at_ms`.
#[derive(Debug, Deserialize)]
pub struct HubSession {
    pub id: String,
    /// The agent it is with; a hub without agents sends none.
    #[serde(default)]
    pub agent_id: Option<String>,
    pub created_at_ms: i64,
    pub last_at_ms: i64,
    pub message_count: u64,
}

#[derive(Deserialize)]
struct SessionsResponse {
    sessions: Vec<HubSession>,
    total: u64,
    next: Option<String>,
}

/// One page of the conversations the Mac lists, and how many the hub holds.
#[derive(Debug)]
pub struct HubSessions {
    /// Newest first, as many as the hub's page holds.
    pub sessions: Vec<HubSession>,
    /// Every conversation the hub holds for this Mac, listed or not.
    pub total: u64,
    /// The `before` of the next page; `None` on the last one.
    pub next: Option<String>,
}

/// Admits a page and refuses only a malformed one, with its numbers.
fn admit_sessions(payload: SessionsResponse) -> Result<HubSessions> {
    let rows = payload.sessions.len();
    for (index, session) in payload.sessions.iter().enumerate() {
        if session.id.is_empty() || session.id.len() > MAX_SESSION_ID_BYTES {
            return Err(HubSessionInvalidRow {
                position: index as u64 + 1,
                rows: rows as u64,
                field: HubSessionField::Id,
                bytes: session.id.len() as u64,
                max_bytes: MAX_SESSION_ID_BYTES as u64,
            }
            .into());
        }
    }
    if payload
        .next
        .as_ref()
        .is_some_and(|id| payload.sessions.last().is_none_or(|last| &last.id != id))
    {
        return Err(HubSessionsCursorMismatch { rows: rows as u64 }.into());
    }
    Ok(HubSessions {
        sessions: payload.sessions,
        total: payload.total,
        next: payload.next,
    })
}

/// One message of a conversation as the hub keeps it.
#[derive(Debug, Deserialize)]
pub struct HubMessage {
    pub id: i64,
    pub role: String,
    pub text: String,
    pub at_ms: Option<i64>,
    pub kind: String,
    #[serde(default)]
    pub duration_ms: Option<u64>,
    #[serde(default)]
    pub client_id: Option<String>,
    /// The files the message carried; absent from a 0.9.0 hub.
    #[serde(default)]
    pub attachments: Vec<orbit_core::attachments::Attachment>,
    /// What an answer's turn did; absent from a hub up to 0.11.
    #[serde(default)]
    pub work: Option<orbit_core::agent_work::WorkSummary>,
}

/// One page of a conversation: its messages in order, and the `before` of
/// the page of older ones.
#[derive(Debug, Deserialize)]
pub struct HubMessages {
    pub messages: Vec<HubMessage>,
    pub next: Option<i64>,
    /// Every message the conversation holds.
    pub total: u64,
    #[serde(default)]
    pub handed_over_from: Option<orbit_core::topics::SessionOrigin>,
}

/// The hub refused a read with a status: its HTTP status and, when it said
/// one, the size and ceiling of what did not fit.
#[derive(Debug, thiserror::Error)]
#[error("La lectura al hub devolvió HTTP {status}")]
pub(crate) struct HubReadStatus {
    pub status: u16,
    pub bytes: Option<u64>,
    pub max_bytes: Option<u64>,
}

pub async fn sessions_page(
    settings: &Settings,
    token: &str,
    development: bool,
    before: Option<&str>,
    op: &Op,
) -> Result<HubSessions> {
    let base = validate_hub_url(
        settings.hub_url.as_deref().context("Falta el hub")?,
        development,
    )?;
    let mut url = base.join("v1/sessions")?;
    url.query_pairs_mut()
        .append_pair("before", before.unwrap_or(""));
    let body = hub_read(request(reqwest::Method::GET, url, Some(token), op)?).await?;
    admit_sessions(serde_json::from_value(body)?)
}

/// One page of a conversation, the newest one without `before`.
pub async fn session_messages_page(
    settings: &Settings,
    token: &str,
    development: bool,
    session_id: &str,
    before: Option<i64>,
    op: &Op,
) -> Result<HubMessages> {
    let base = validate_hub_url(
        settings.hub_url.as_deref().context("Falta el hub")?,
        development,
    )?;
    let mut url = base.join("v1/sessions/")?;
    url.path_segments_mut()
        .map_err(|()| anyhow::anyhow!("URL del hub sin ruta"))?
        .pop_if_empty()
        .extend([session_id, "messages"]);
    if let Some(before) = before {
        url.query_pairs_mut()
            .append_pair("before", &before.to_string());
    }
    let body = hub_read(request(reqwest::Method::GET, url, Some(token), op)?).await?;
    Ok(serde_json::from_value(body)?)
}

/// One bounded JSON read; a refusal keeps its status and, for a page that
/// does not fit, the hub's numbers.
async fn hub_read(request: reqwest::RequestBuilder) -> Result<Value> {
    let response = request.send().await?;
    let status = response.status();
    if !status.is_success() {
        let refusal = read_refusal(response, MAX_HUB_HTTP_BODY_BYTES)
            .await
            .unwrap_or_default();
        let number = |key: &str| refusal.numbers.get(key).copied();
        return Err(HubReadStatus {
            status: status.as_u16(),
            bytes: number("bytes"),
            max_bytes: number("max_bytes"),
        }
        .into());
    }
    bounded_json(response).await
}

#[cfg(test)]
mod session_list_tests {
    use super::*;

    #[test]
    fn a_page_is_admitted_with_its_rows_total_and_cursor() -> Result<()> {
        let admitted = admit_sessions(serde_json::from_value(json!({
            "sessions": [
                {"id": "session-b", "created_at_ms": 2, "last_at_ms": 9, "message_count": 4},
                {"id": "session-a", "created_at_ms": 1, "last_at_ms": 1, "message_count": 0},
            ],
            "total": 500,
            "next": "session-a",
        }))?)?;
        assert_eq!(admitted.sessions.len(), 2);
        assert_eq!(admitted.total, 500);
        assert_eq!(admitted.next.as_deref(), Some("session-a"));
        assert_eq!(admitted.sessions[0].last_at_ms, 9);
        Ok(())
    }

    #[test]
    fn a_malformed_conversation_is_refused_with_its_numbers() -> Result<()> {
        let payload = json!({"sessions": [
            {"id": "session-a", "created_at_ms": 3, "last_at_ms": 3, "message_count": 0},
            {"id": "", "created_at_ms": 2, "last_at_ms": 2, "message_count": 0},
        ], "total": 2, "next": null});
        let Err(error) = admit_sessions(serde_json::from_value(payload)?) else {
            bail!("an empty id was admitted");
        };
        assert_eq!(
            error.to_string(),
            "La sesión 2 de las 2 filas de la página del hub tiene un id de 0 bytes; se \
             admiten de 1 a 121"
        );
        Ok(())
    }

    #[test]
    fn a_cursor_that_is_not_the_last_row_is_refused() -> Result<()> {
        let payload = json!({"sessions": [
            {"id": "session-a", "created_at_ms": 3, "last_at_ms": 3, "message_count": 0},
        ], "total": 9, "next": "session-z"});
        let Err(error) = admit_sessions(serde_json::from_value(payload)?) else {
            bail!("a cursor past the page was admitted");
        };
        assert!(error.downcast_ref::<HubSessionsCursorMismatch>().is_some());
        Ok(())
    }
}
