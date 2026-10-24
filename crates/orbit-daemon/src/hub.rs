//! The daemon's link to the hub: the device's WebSocket (`link`) and the
//! HTTP calls made with its credential (pairing, the conversations, the
//! attachments, a voice turn). This adapter never retries a submitted prompt.
use crate::config::{Settings, validate_hub_url};
use anyhow::{Context, Result, bail};
use orbit_core::limits::{MAX_HUB_HTTP_BODY_BYTES, MAX_HUB_OUTGOING_FRAME_BYTES};
use orbit_core::notice::Op;
use serde::Deserialize;
use serde_json::{Value, json};
use std::time::Duration;

mod attachments;
mod conversations;
mod link;
mod voice;

pub use attachments::{
    attachment_from_capture, fetch_attachment, upload_attachment, upload_capture,
};
pub use conversations::{
    HubMessage, HubMessages, HubSession, HubSessions, session_messages_page, sessions_page,
};
pub(crate) use conversations::{HubReadStatus, HubSessionInvalidRow, HubSessionsCursorMismatch};
pub use link::{Events, HubEvent, run};
pub(crate) use link::{invalid_event_name, unsent_count};
pub use voice::transcribe;

/// Hard cap on an outgoing hub frame, matched to the server's `max_frame_bytes`
/// (server/orbit_hub/config.py). The read-side bound lives in
/// [`MAX_HUB_INCOMING_FRAME_BYTES`] (1 MiB, inbound scope); this one is the wire
/// limit the server enforces and the client must respect before the bytes leave
/// the Mac.
pub const MAX_OUTGOING_FRAME_BYTES: usize = MAX_HUB_OUTGOING_FRAME_BYTES;
/// First hub protocol that advertises typed refusal frames in `ready`.
/// The source contract in server/tests/test_ref405_hub_fixtures.py compares
/// this marker with the hub and the ready fixture.
#[cfg(test)]
pub(crate) const TYPED_REFUSAL_HUB_PROTOCOL: u64 = 1;
/// Bounds an untrusted hub refusal code before it enters a durable voice or
/// meeting notice. A scan of 179 literal `HubError` codes found a longest
/// name of 35 ASCII bytes; 64 leaves room for new names. This is a code
/// bound, not an HTTP body or audio bound.
pub(crate) const MAX_HUB_CODE_BYTES: usize = 64;
// Existing shared deadlines for ordinary hub calls. They do not govern voice:
// the transcription endpoint has its own ordered budget below.
pub(crate) const HUB_HTTP_TIMEOUT: Duration = Duration::from_secs(HUB_HTTP_TIMEOUT_SECONDS);
/// In seconds, as the interface reads it: how long it keeps a choice the hub
/// confirms (`apps/desktop/src/state/ceilings.ts`, `contracts/constants.json`).
const HUB_HTTP_TIMEOUT_SECONDS: u64 = 90;
const HUB_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// The hub's ceiling for one voice request, queue and provider, itself
/// outside the provider's 180 s call (`INTERACTIVE_TRANSCRIPTION_TIMEOUT_SECONDS`
/// in `server/orbit_hub/transcription.py`; contracts/constants.json compares
/// the two copies).
const HUB_VOICE_DEADLINE_SECONDS: u64 = 240;
/// Protects one daemon→hub voice request from a hub that never answers: the
/// hub's ceiling plus the existing 10 s connection budget, so the hub's causal
/// refusal reaches the note first. This is not the Whisper deadline, a retry
/// interval, or permission to send the audio twice. A literal, because the
/// real test on the Mac (`scripts/e2e-mac/real_test.py`) reads it from here.
const VOICE_HUB_HTTP_TIMEOUT: Duration = Duration::from_secs(250);
const _: () = assert!(VOICE_HUB_HTTP_TIMEOUT.as_secs() == HUB_VOICE_DEADLINE_SECONDS + 10);

#[derive(Debug, thiserror::Error)]
#[error("La respuesta HTTP mide {bytes} bytes y el máximo es {max_bytes}")]
pub(crate) struct HubBodyTooLarge {
    pub bytes: u64,
    pub max_bytes: u64,
}

#[derive(Deserialize)]
struct PairResponse {
    device_id: String,
    token: String,
}

pub fn command(id: &str, method: &str, params: Value) -> Result<Value> {
    let request = orbit_core::wire::hub::HubRequest::new(id.to_owned(), method, params)
        .map_err(anyhow::Error::msg)?;
    Ok(serde_json::to_value(request)?)
}

/// The header every request to the hub carries (the contract with the hub):
/// the op of the action it belongs to.
pub const OP_HEADER: &str = "X-Orbit-Op";

/// Admission for a paired, online device; a privacy stop always wins.
pub(crate) fn access<'a>(
    settings: &Settings,
    token: Option<&'a str>,
    privacy_stopped: bool,
    connection: orbit_core::Connection,
) -> Result<&'a str, orbit_core::refusal::Refusal> {
    use orbit_core::refusal::Refusal;
    if privacy_stopped {
        return Err(Refusal::PrivacyStopped { since_ms: None });
    }
    let token = token
        .filter(|_| settings.device_id.is_some())
        .ok_or(Refusal::NotPaired)?;
    match connection {
        orbit_core::Connection::Online => Ok(token),
        orbit_core::Connection::Offline
        | orbit_core::Connection::Connecting
        | orbit_core::Connection::Revoked => Err(Refusal::HubUnreachable { since_ms: None }),
    }
}

/// One pool for all actions; credentials, op and deadlines belong to requests.
fn client() -> Result<&'static reqwest::Client> {
    static CLIENT: std::sync::OnceLock<Result<reqwest::Client, reqwest::Error>> =
        std::sync::OnceLock::new();
    CLIENT
        .get_or_init(|| {
            reqwest::Client::builder()
                .connect_timeout(HUB_CONNECT_TIMEOUT)
                .redirect(reqwest::redirect::Policy::none())
                .build()
        })
        .as_ref()
        .map_err(|error| anyhow::anyhow!("No se pudo preparar el cliente HTTP: {error}"))
}

pub(crate) fn request(
    method: reqwest::Method,
    url: reqwest::Url,
    token: Option<&str>,
    op: &Op,
) -> Result<reqwest::RequestBuilder> {
    let request = client()?
        .request(method, url)
        .header(OP_HEADER, op.as_str())
        .timeout(HUB_HTTP_TIMEOUT);
    Ok(match token {
        Some(token) => request.bearer_auth(token),
        None => request,
    })
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum BodyError {
    #[error(transparent)]
    TooLarge(#[from] HubBodyTooLarge),
    #[error("La respuesta del hub se cortó tras {received_bytes} bytes: {source}")]
    Cut {
        received_bytes: usize,
        source: reqwest::Error,
    },
}

pub(crate) async fn bounded_body(
    mut response: reqwest::Response,
    limit: usize,
) -> Result<Vec<u8>, BodyError> {
    if let Some(bytes) = response
        .content_length()
        .filter(|size| *size > limit as u64)
    {
        return Err(HubBodyTooLarge {
            bytes,
            max_bytes: limit as u64,
        }
        .into());
    }
    let mut bytes = Vec::new();
    loop {
        let chunk = response.chunk().await.map_err(|source| BodyError::Cut {
            received_bytes: bytes.len(),
            source,
        })?;
        let Some(chunk) = chunk else {
            return Ok(bytes);
        };
        let size = bytes.len().saturating_add(chunk.len());
        if size > limit {
            return Err(HubBodyTooLarge {
                bytes: size as u64,
                max_bytes: limit as u64,
            }
            .into());
        }
        bytes.extend_from_slice(&chunk);
    }
}

pub(crate) async fn json_body(response: reqwest::Response, limit: usize) -> Result<Value> {
    let bytes = bounded_body(response, limit)
        .await
        .map_err(|error| match error {
            BodyError::TooLarge(size) => anyhow::Error::from(size),
            BodyError::Cut { source, .. } => anyhow::Error::from(source),
        })?;
    Ok(serde_json::from_slice(&bytes)?)
}

pub async fn bounded_json(response: reqwest::Response) -> Result<Value> {
    json_body(response, MAX_HUB_HTTP_BODY_BYTES).await
}

/// One hub error envelope. Keep its unsigned facts separate from free text;
/// each caller retains its own status policy and error-body ceiling.
#[derive(Default)]
pub(crate) struct HubRefusal {
    pub code: Option<String>,
    pub message: Option<String>,
    pub details: Value,
    pub numbers: std::collections::BTreeMap<String, u64>,
}

pub(crate) async fn read_refusal(response: reqwest::Response, limit: usize) -> Result<HubRefusal> {
    let envelope = json_body(response, limit).await?;
    let details = envelope["refusal"].clone();
    let numbers = details
        .as_object()
        .map(|object| {
            object
                .iter()
                .filter_map(|(key, value)| Some((key.clone(), value.as_u64()?)))
                .collect()
        })
        .unwrap_or_default();
    Ok(HubRefusal {
        code: envelope["code"].as_str().map(str::to_owned),
        message: envelope["message"]
            .as_str()
            .map(|message| {
                message
                    .chars()
                    .filter(|character| !character.is_control())
                    .take(300)
                    .collect::<String>()
            })
            .filter(|message| !message.is_empty()),
        details,
        numbers,
    })
}

/// A hub code is snake_case ASCII; never echo arbitrary content as a code.
pub(crate) fn hub_code(code: &str) -> Option<&str> {
    Some(code).filter(|code| {
        !code.is_empty()
            && code.len() <= MAX_HUB_CODE_BYTES
            && code
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte == b'_')
    })
}

#[derive(Clone, Copy)]
pub(crate) enum TransportCause {
    ConnectionTimeout,
    Timeout,
    Connection,
    Cut,
}

impl TransportCause {
    pub(crate) fn of(error: &reqwest::Error) -> Self {
        match (error.is_timeout(), error.is_connect()) {
            (true, true) => Self::ConnectionTimeout,
            (true, false) => Self::Timeout,
            (false, true) => Self::Connection,
            (false, false) => Self::Cut,
        }
    }
}

pub async fn pair(
    settings: &Settings,
    code: &str,
    development: bool,
    op: &Op,
) -> Result<(String, String)> {
    let base = validate_hub_url(
        settings
            .hub_url
            .as_deref()
            .context("Falta la URL del hub")?,
        development,
    )?;
    let response = request(reqwest::Method::POST, base.join("v1/pair")?, None, op)?
        .json(&json!({"code":code,"name":settings.device_name}))
        .send()
        .await?;
    if !response.status().is_success() {
        bail!(
            "El hub rechazó el código de emparejamiento ({})",
            response.status()
        );
    }
    let value: PairResponse = serde_json::from_value(bounded_json(response).await?)
        .context("Respuesta de emparejamiento fuera de contrato")?;
    Ok((value.device_id, value.token))
}

#[cfg(test)]
mod http_client_tests {
    use super::*;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::sync::mpsc;

    async fn reply_once(wire: String) -> Result<(Settings, tokio::task::JoinHandle<Result<()>>)> {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let settings = Settings {
            hub_url: Some(format!("http://{}", listener.local_addr()?)),
            ..Settings::default()
        };
        let server = tokio::spawn(async move {
            let (socket, _) = listener.accept().await?;
            let mut socket = BufReader::new(socket);
            loop {
                let mut line = String::new();
                if socket.read_line(&mut line).await? == 0 {
                    bail!("request ended before its headers");
                }
                if line == "\r\n" {
                    break;
                }
            }
            socket.write_all(wire.as_bytes()).await?;
            Ok(())
        });
        Ok((settings, server))
    }

    #[tokio::test]
    async fn notes_and_chat_bound_declared_and_streamed_bodies_and_keep_cut_counts() -> Result<()> {
        use crate::hub_notes::{Answer, NoteHub, Unreached};
        let oversize = MAX_HUB_HTTP_BODY_BYTES + 1;
        for (wire, cut) in [
            (
                format!("HTTP/1.1 200 OK\r\nContent-Length: {oversize}\r\n\r\n"),
                false,
            ),
            (
                format!(
                    "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n{oversize:x}\r\n{}\r\n0\r\n\r\n",
                    " ".repeat(oversize)
                ),
                false,
            ),
            (
                "HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\n{".into(),
                true,
            ),
        ] {
            for notes in [false, true] {
                let (settings, server) = reply_once(wire.clone()).await?;
                if notes {
                    let hub = NoteHub {
                        base: reqwest::Url::parse(
                            settings.hub_url.as_deref().context("fixture URL missing")?,
                        )?,
                        token: "synthetic".into(),
                        op: orbit_ipc::new_op(),
                    };
                    let answer = hub.document("synthetic").await;
                    if cut {
                        assert!(
                            matches!(
                                answer,
                                Answer::Unreachable(Unreached::Cut { received_bytes: 1 })
                            ),
                            "{answer:?}"
                        );
                    } else {
                        assert!(
                            matches!(answer, Answer::Unreachable(Unreached::TooLarge { bytes, max_bytes }) if bytes == oversize as u64 && max_bytes == MAX_HUB_HTTP_BODY_BYTES),
                            "{answer:?}"
                        );
                    }
                } else {
                    let error =
                        sessions_page(&settings, "synthetic", true, None, &orbit_ipc::new_op())
                            .await
                            .err()
                            .context("oversized or cut body was admitted")?;
                    if cut {
                        assert!(
                            error.downcast_ref::<reqwest::Error>().is_some(),
                            "{error:#}"
                        );
                    } else {
                        let size = error
                            .downcast_ref::<HubBodyTooLarge>()
                            .context("size facts missing")?;
                        assert_eq!(
                            (size.bytes, size.max_bytes),
                            (oversize as u64, MAX_HUB_HTTP_BODY_BYTES as u64)
                        );
                    }
                }
                server.await??;
            }
        }
        Ok(())
    }

    #[tokio::test]
    async fn successive_reads_reuse_connection_without_sharing_op_or_credential() -> Result<()> {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let settings = Settings {
            hub_url: Some(format!("http://{}", listener.local_addr()?)),
            ..Settings::default()
        };
        let (sent, mut received) = mpsc::channel(2);
        let server = tokio::spawn(async move {
            let mut peers = tokio::task::JoinSet::new();
            while let Ok((socket, address)) = listener.accept().await {
                let sent = sent.clone();
                peers.spawn(async move {
                    let mut socket = BufReader::new(socket);
                    loop {
                        let mut headers = String::new();
                        loop {
                            let mut line = String::new();
                            if socket.read_line(&mut line).await? == 0 {
                                return anyhow::Ok(());
                            }
                            if line == "\r\n" {
                                break;
                            }
                            headers.push_str(&line);
                        }
                        sent.send((address, headers)).await?;
                        let body = r#"{"sessions":[],"total":0,"next":null}"#;
                        socket
                            .write_all(
                                format!(
                                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{body}",
                                    body.len()
                                )
                                .as_bytes(),
                            )
                            .await?;
                    }
                });
            }
        });
        let first_op = orbit_ipc::new_op();
        let second_op = orbit_ipc::new_op();
        sessions_page(&settings, "first-token", true, None, &first_op).await?;
        sessions_page(&settings, "second-token", true, None, &second_op).await?;
        let first = received.recv().await.context("first request missing")?;
        let second = received.recv().await.context("second request missing")?;
        server.abort();
        assert!(first.1.contains(first_op.as_str()), "{}", first.1);
        assert!(second.1.contains(second_op.as_str()), "{}", second.1);
        assert!(!second.1.contains(first_op.as_str()), "{}", second.1);
        assert!(first.1.contains("Bearer first-token"));
        assert!(second.1.contains("Bearer second-token"));
        assert!(!second.1.contains("first-token"));
        assert_eq!(first.0, second.0, "each read opened a new TCP connection");
        Ok(())
    }
}
