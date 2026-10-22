//! Outgoing device connection. This adapter never retries a submitted prompt.
use crate::config::{Settings, validate_hub_url};
use anyhow::{Context, Result, bail};
use futures_util::{SinkExt, StreamExt};
use orbit_core::limits::{
    MAX_HUB_HTTP_BODY_BYTES, MAX_HUB_INCOMING_FRAME_BYTES, MAX_HUB_OUTGOING_FRAME_BYTES,
};
use orbit_core::notice::Op;
use orbit_core::refusal::HubSessionField;
use orbit_core::wire::hub::{
    HUB_PROTOCOL, HubDecodeError, HubEnvelopeError, HubEvent as WireHubEvent, HubRequest,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{fmt, time::Duration};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::{Message, client::IntoClientRequest, http::HeaderValue};

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
/// The hub's session ids are UUIDs; this is room for what `open_session`
/// accepts, no more.
use orbit_core::limits::MAX_SESSION_ID_BYTES;
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
}

/// One page of a conversation: its messages in order, and the `before` of
/// the page of older ones.
#[derive(Debug, Deserialize)]
pub struct HubMessages {
    pub messages: Vec<HubMessage>,
    pub next: Option<i64>,
    /// Every message the conversation holds.
    pub total: u64,
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

#[derive(Deserialize)]
struct PairResponse {
    device_id: String,
    token: String,
}

#[derive(Deserialize)]
struct SpeechResponse {
    text: String,
}

#[derive(Deserialize)]
struct DeviceReplySummary {
    request_id: Option<String>,
    refusal: Option<Value>,
}

#[derive(Debug)]
pub enum HubEvent {
    Connected,
    Disconnected,
    Frame(Value),
    Decoded(Box<WireHubEvent>),
    InvalidFrame { event: String, reason: String },
    Problem(String),
    Revoked,
}

pub struct Events {
    pub generation: u64,
    pub sender: mpsc::Sender<(u64, HubEvent)>,
}

impl Events {
    async fn send(&self, event: HubEvent) -> Result<()> {
        self.sender.send((self.generation, event)).await?;
        Ok(())
    }
}

#[derive(Debug)]
struct UnsentFrames {
    count: usize,
    cause: String,
}

impl fmt::Display for UnsentFrames {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let frames = if self.count == 1 { "trama" } else { "tramas" };
        write!(
            formatter,
            "El hub no escribió {} {frames}: {}",
            self.count, self.cause
        )
    }
}

impl std::error::Error for UnsentFrames {}

pub(crate) fn unsent_count(error: &anyhow::Error) -> Option<usize> {
    error
        .downcast_ref::<UnsentFrames>()
        .map(|failure| failure.count)
}

pub(crate) fn invalid_event_name(reason: &str) -> &str {
    let name = reason
        .split_once(".data")
        .or_else(|| reason.split_once(".id"))
        .map_or("hub frame", |(name, _)| name);
    match name {
        "ready"
        | "pong"
        | "chat.interrupted"
        | "chat.session"
        | "chat.delta"
        | "chat.completed"
        | "chat.status"
        | "approval.required"
        | "clarify.required"
        | "interaction.resolved"
        | "interaction.expired"
        | "device.request"
        | "state.snapshot"
        | "error"
        | "revoked"
        | "recording.changed" => name,
        _ => "hub frame",
    }
}

/// A closed command sender ends the task after the connected socket drains.
/// `Ok` confirms local writes and closure, not that Hermes processed a frame;
/// a queued frame or socket error is returned to the stopping daemon.
pub async fn run(
    settings: Settings,
    token: String,
    development: bool,
    mut commands: mpsc::Receiver<Value>,
    events: Events,
) -> Result<()> {
    let mut delay = 1u64;
    let mut unsent = 0usize;
    loop {
        let result = connect(
            &settings,
            &token,
            development,
            &mut commands,
            &events,
            &mut unsent,
        )
        .await;
        unsent = unsent.saturating_add(result.as_ref().err().and_then(unsent_count).unwrap_or(0));
        if commands.is_closed() {
            unsent = unsent.saturating_add(commands.len());
            if unsent > 0 || result.is_err() {
                return Err(UnsentFrames {
                    count: unsent,
                    cause: result.err().map_or_else(
                        || "el emisor cerró antes de completar el vaciado".to_owned(),
                        |error| error.to_string(),
                    ),
                }
                .into());
            }
            return Ok(());
        }
        if let Err(error) = &result {
            events
                .send(HubEvent::Problem(format!("Hub no disponible: {error}")))
                .await?;
        }
        events.send(HubEvent::Disconnected).await?;
        // A frame queued while the socket failed must not be replayed.
        while commands.try_recv().is_ok() {
            unsent = unsent.saturating_add(1);
        }
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_secs(delay)) => {},
            command = commands.recv() => match command {
                Some(_) => unsent = unsent.saturating_add(1),
                None if unsent > 0 => return Err(UnsentFrames {
                    count: unsent,
                    cause: "la conexión estaba esperando un reintento".to_owned(),
                }.into()),
                None => return Ok(()),
            }
        }
        delay = if result.is_ok() {
            1
        } else {
            (delay * 2).min(30)
        };
    }
}

/// How long opening the hub's WebSocket (TCP, TLS and the upgrade) may take
/// before it counts as failed and the reconnection waits its backoff. What
/// it protects: a half-open network never stalls the reconnection loop. A
/// handshake over the tailnet takes well under a second; ten is room for a
/// slow network, not a measurement.
const HUB_CONNECT_WAIT: Duration = Duration::from_secs(10);

/// Why the hub's WebSocket did not open, and where. Each cause points at a
/// different thing to look at (the hub down, the network, a certificate, a
/// proxy's answer); one word for all of them sent the reader to the wrong
/// one (on the Mac, 2026-10-18: «TLS» for a plain-HTTP hub that was down).
fn unreached(error: &tokio_tungstenite::tungstenite::Error, origin: &str) -> String {
    use tokio_tungstenite::tungstenite::Error;
    match error {
        Error::Io(io) if io.kind() == std::io::ErrorKind::ConnectionRefused => {
            format!("el hub en {origin} rechazó la conexión: nadie escucha en ese puerto")
        }
        Error::Io(io) => format!("la red no llegó al hub en {origin}: {io}"),
        Error::Tls(tls) => format!("la conexión TLS con el hub en {origin} falló: {tls}"),
        Error::Http(response) => format!(
            "el hub en {origin} contestó {} al abrir el WebSocket",
            response.status().as_u16()
        ),
        other @ (Error::ConnectionClosed
        | Error::AlreadyClosed
        | Error::Capacity(_)
        | Error::Protocol(_)
        | Error::WriteBufferFull(_)
        | Error::Utf8(_)
        | Error::AttackAttempt
        | Error::Url(_)
        | Error::HttpFormat(_)) => {
            format!("el WebSocket con el hub en {origin} no se abrió: {other}")
        }
    }
}

fn timed_out(origin: &str) -> String {
    format!(
        "el hub en {origin} no completó la conexión en {} s",
        HUB_CONNECT_WAIT.as_secs()
    )
}

async fn connect(
    settings: &Settings,
    token: &str,
    development: bool,
    commands: &mut mpsc::Receiver<Value>,
    events: &Events,
    // Frames lost since the last one written. A later write resets it: the
    // earlier loss was reported when it happened, and the drain answers for
    // what is still unwritten, above all the stop's interrupt.
    unsent: &mut usize,
) -> Result<()> {
    let mut url = validate_hub_url(
        settings
            .hub_url
            .as_deref()
            .context("Dispositivo sin emparejar")?,
        development,
    )?;
    let scheme = if url.scheme() == "https" { "wss" } else { "ws" };
    url.set_scheme(scheme)
        .map_err(|_| anyhow::anyhow!("Esquema no válido"))?;
    url.set_path("/v1/ws");
    let mut request = url.as_str().into_client_request()?;
    request.headers_mut().insert(
        "Authorization",
        HeaderValue::from_str(&format!("Bearer {token}"))?,
    );
    // Each connection attempt is an action of its own: its upgrade
    // request names a new op; the frames sent on it carry their own.
    request.headers_mut().insert(
        OP_HEADER,
        HeaderValue::from_str(orbit_ipc::new_op().as_str())?,
    );
    let config = tokio_tungstenite::tungstenite::protocol::WebSocketConfig::default()
        .max_message_size(Some(MAX_HUB_INCOMING_FRAME_BYTES))
        .max_frame_size(Some(MAX_HUB_INCOMING_FRAME_BYTES));
    let origin = format!(
        "{}:{}",
        url.host_str().unwrap_or("?"),
        url.port_or_known_default().unwrap_or_default()
    );
    let connect = tokio_tungstenite::connect_async_with_config(request, Some(config), false);
    let (mut socket, _) = match tokio::time::timeout(HUB_CONNECT_WAIT, connect).await {
        Err(_) => bail!("{}", timed_out(&origin)),
        Ok(Ok(value)) => value,
        Ok(Err(tokio_tungstenite::tungstenite::Error::Http(response)))
            if matches!(response.status().as_u16(), 401 | 403) =>
        {
            events.send(HubEvent::Revoked).await?;
            bail!("Credencial rechazada; vuelve a emparejar el dispositivo");
        }
        Ok(Err(error)) => bail!("{}", unreached(&error, &origin)),
    };
    events.send(HubEvent::Connected).await?;
    let mut ping = tokio::time::interval(Duration::from_secs(10));
    let mut watchdog = Box::pin(tokio::time::sleep(Duration::from_secs(30)));
    loop {
        tokio::select! {
            message = socket.next() => match message {
                Some(Ok(Message::Text(text))) => {
                    let value: Value = match serde_json::from_str(&text) {
                        Ok(value) => value,
                        Err(error) => {
                            bail!("El hub envió JSON inválido: versión local {HUB_PROTOCOL}, versión del hub ilegible ({error})");
                        }
                    };
                    let rejected_bridge = (value.get("event").and_then(Value::as_str) == Some("device.request")
                        && value.get("data").and_then(|data| data.get("request_id"))
                            .and_then(Value::as_str).is_some()).then(|| value.clone());
                    match WireHubEvent::decode_detailed(value) {
                        Ok(decoded) => {
                            events.send(HubEvent::Decoded(Box::new(decoded))).await?;
                        }
                        Err(HubDecodeError::Envelope(HubEnvelopeError::UnsupportedVersion { received })) => {
                            bail!("Versión de hub incompatible: versión local {HUB_PROTOCOL}, versión del hub {received}");
                        }
                        Err(HubDecodeError::Envelope(HubEnvelopeError::MissingOrInvalidVersion)) => {
                            bail!("Versión de hub incompatible: versión local {HUB_PROTOCOL}, versión del hub ausente o ilegible");
                        }
                        Err(HubDecodeError::Envelope(HubEnvelopeError::NotObject)) => {
                            bail!("La trama del hub no es un objeto JSON: versión local {HUB_PROTOCOL}, versión del hub desconocida");
                        }
                        Err(HubDecodeError::Event(reason)) => {
                            if let Some(value) = rejected_bridge {
                                // A malformed bridge request still has an owner waiting for a
                                // refusal. Keep its raw shape only until the daemon names the
                                // invalid parameter; never execute it.
                                events.send(HubEvent::Frame(value)).await?;
                            } else {
                                let event = invalid_event_name(&reason).to_owned();
                                events.send(HubEvent::InvalidFrame { event, reason }).await?;
                            }
                        }
                    }
                    watchdog.as_mut().reset(tokio::time::Instant::now() + Duration::from_secs(30));
                },
                Some(Ok(Message::Ping(data))) => {
                    let body = Message::Pong(data);
                    tokio::time::timeout(Duration::from_secs(5), socket.send(body))
                        .await
                        .context("Envío de Pong al hub agotó el plazo de 5 s")??;
                    watchdog.as_mut().reset(tokio::time::Instant::now() + Duration::from_secs(30));
                },
                Some(Ok(Message::Close(_))) | None => return Ok(()),
                Some(Err(_)) => bail!("La conexión se interrumpió; no se repiten órdenes"),
                _ => {}
            },
            command = commands.recv() => match command {
                Some(command) => {
                    let body = serde_json::to_string(&command)?;
                    if body.len() > MAX_OUTGOING_FRAME_BYTES {
                        events.send(HubEvent::Problem(format!(
                            "Trama de {} bytes supera el límite del hub ({} bytes): el turno no se envía",
                            body.len(),
                            MAX_OUTGOING_FRAME_BYTES,
                        ))).await?;
                        // The frame does not go, but the silence does not
                        // stay either. Whoever asked gets the cause instead of
                        // waiting out its whole timeout on a Mac that already
                        // decided.
                        if let Some(refusal) = oversize_refusal(&command, body.len())? {
                            let encoded = serde_json::to_string(&refusal)?;
                            if encoded.len() <= MAX_OUTGOING_FRAME_BYTES {
                                let body = Message::Text(encoded.into());
                                let sent = tokio::time::timeout(Duration::from_secs(5), socket.send(body))
                                    .await
                                    .context("Envío al hub agotó el plazo de 5 s")
                                    .and_then(|outcome| outcome.map_err(Into::into));
                                if let Err(error) = sent {
                                    return Err(UnsentFrames { count: 1, cause: error.to_string() }.into());
                                }
                            }
                        }
                    } else {
                        let body = Message::Text(body.into());
                        let sent = tokio::time::timeout(Duration::from_secs(5), socket.send(body))
                            .await
                            .context("Envío al hub agotó el plazo de 5 s")
                            .and_then(|outcome| outcome.map_err(Into::into));
                        if let Err(error) = sent {
                            return Err(UnsentFrames { count: 1, cause: error.to_string() }.into());
                        }
                        *unsent = 0;
                    }
                },
                None => { socket.close(None).await.context("Cierre del socket del hub falló")?; return Ok(()); }
            },
            _ = ping.tick() => {
                tokio::time::timeout(Duration::from_secs(5), socket.send(Message::Ping(Vec::new().into())))
                    .await
                    .context("Envío de ping al hub agotó el plazo de 5 s")??;
            },
            _ = watchdog.as_mut() => {
                bail!("El servidor permaneció en silencio durante el plazo de vida (30 s)");
            }
        }
    }
}

/// The short answer that replaces a reply too large to send.
///
/// Only a `device.reply` gets a substitute, and that is deliberate. It is the
/// one outgoing shape with a caller on the other side holding an id and a
/// timeout; everything else the daemon sends is a turn of the person's own
/// conversation, whose only listener is the person, and who is already told
/// through [`HubEvent::Problem`]. Inventing a wire answer for those would put a
/// frame on the socket that nobody asked for.
///
/// A refusal that is itself too large gets no substitute either: there is
/// nothing shorter left to say that would not be a second invention.
fn oversize_refusal(frame: &Value, size: usize) -> Result<Option<Value>> {
    let Ok(HubRequest::DeviceReply { id, params, .. }) = HubRequest::decode(frame.clone()) else {
        return Ok(None);
    };
    let summary: DeviceReplySummary = serde_json::from_value(serde_json::to_value(params)?)
        .context("device.reply.params fuera de contrato")?;
    if summary.refusal.is_some() {
        return Ok(None);
    }
    let Some(request_id) = summary.request_id.as_deref() else {
        return Ok(None);
    };
    let refusal = orbit_core::refusal::Refusal::TooLarge {
        resource: orbit_core::refusal::Resource::Message,
        bytes: size as u64,
        max_bytes: MAX_OUTGOING_FRAME_BYTES as u64,
    };
    let params = json!({"request_id": request_id, "refusal": refusal});
    Ok(Some(command(&id, "device.reply", params)?))
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

/// The one way a capture's bytes reach the hub.
///
/// Both captures use this. The native helper's picture and the engine's are
/// the same kind of thing — a PNG far too big for a control frame — and the
/// split that makes them work is the same split: the bytes go over HTTP, bound
/// to the bridge request that asked for them, and the frame that answers
/// carries only the identifier. Two uploads that merely resembled each other
/// would drift, and only one of them would be the one anybody remembered.
///
/// Returns the digest it sent, so the answer can name the exact bytes the hub
/// now holds rather than recomputing them somewhere else.
pub async fn upload_capture(
    hub_url: &str,
    token: &str,
    development: bool,
    capture_id: &str,
    request_id: &str,
    bytes: Vec<u8>,
    op: &Op,
) -> Result<String> {
    let sha256 = format!("{:x}", <sha2::Sha256 as sha2::Digest>::digest(&bytes));
    let base = validate_hub_url(hub_url, development)?;
    let response = request(
        reqwest::Method::PUT,
        base.join(&format!("v1/captures/{capture_id}"))?,
        Some(token),
        op,
    )?
    .header("Content-Type", "image/png")
    .header("X-Orbit-Request", request_id)
    .header("X-Orbit-Sha256", &sha256)
    .body(bytes)
    .send()
    .await?;
    if !response.status().is_success() {
        let status = response.status();
        let body = read_refusal(response, MAX_HUB_HTTP_BODY_BYTES).await?;
        if let Ok(refusal) = serde_json::from_value::<orbit_core::refusal::Refusal>(body.details) {
            if body.code.as_deref() == Some("capture_capacity")
                && matches!(
                    &refusal,
                    orbit_core::refusal::Refusal::Busy {
                        operation: orbit_core::refusal::Operation::Capture,
                        ..
                    }
                )
            {
                return Err(orbit_core::refusal::Refusal::UpstreamFailed {
                    service: orbit_core::refusal::Service::Hub,
                    status: Some(status.as_u16()),
                }
                .into());
            }
            return Err(refusal.into());
        }
        bail!("El hub rechazó la captura (HTTP {status})")
    }
    Ok(sha256)
}

/// The header that names an attachment, percent-encoded UTF-8 (hub-v1).
const ATTACHMENT_NAME_HEADER: &str = "X-Orbit-Attachment-Name";

/// `name` as UTF-8 bytes, each one outside `A-Za-z0-9-._~` as `%XX`: a
/// header carries Latin-1, a file name anything.
fn percent_encoded(name: &str) -> String {
    let mut encoded = String::with_capacity(name.len());
    for byte in name.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            encoded.push(char::from(byte));
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

/// The inverse of `percent_encoded`; a `%` without two hex digits stays as it is.
fn percent_decoded(encoded: &str) -> String {
    let bytes = encoded.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        let pair = bytes
            .get(index + 1..index + 3)
            .and_then(|pair| std::str::from_utf8(pair).ok())
            .and_then(|pair| u8::from_str_radix(pair, 16).ok());
        match (bytes[index], pair) {
            (b'%', Some(byte)) => {
                decoded.push(byte);
                index += 3;
            }
            (byte, _) => {
                decoded.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8_lossy(&decoded).into_owned()
}

/// What a refused attachment request says: the hub's typed attachment
/// refusal when it is one, its status otherwise.
async fn attachment_refusal(response: reqwest::Response) -> anyhow::Error {
    let status = response.status().as_u16();
    match read_refusal(response, MAX_HUB_HTTP_BODY_BYTES).await {
        Ok(body) => {
            if let Some(refusal) = orbit_core::attachments::hub_refusal(&body.details) {
                return refusal.into();
            }
            if let Ok(refusal) =
                serde_json::from_value::<orbit_core::refusal::Refusal>(body.details)
            {
                return refusal.into();
            }
        }
        Err(error) => return error,
    }
    orbit_core::refusal::Refusal::UpstreamFailed {
        service: orbit_core::refusal::Service::Hub,
        status: Some(status),
    }
    .into()
}

/// An attachment request that got no answer from the hub. A hub stopped,
/// out of reach or gone mid-answer is said as such; one that kept quiet
/// past `HUB_HTTP_TIMEOUT` names that deadline. Never the request library's
/// text, which carries the hub's address.
fn attachment_unanswered(error: &reqwest::Error) -> anyhow::Error {
    use orbit_core::refusal::{Operation, Refusal};
    match TransportCause::of(error) {
        TransportCause::Timeout => {
            let deadline_ms = u64::try_from(HUB_HTTP_TIMEOUT.as_millis()).unwrap_or(u64::MAX);
            Refusal::Timeout {
                operation: Operation::Files,
                waited_ms: deadline_ms,
                deadline_ms,
            }
        }
        TransportCause::ConnectionTimeout | TransportCause::Connection | TransportCause::Cut => {
            Refusal::HubUnreachable { since_ms: None }
        }
    }
    .into()
}

/// One file for the composer, uploaded whole (`POST /v1/attachments`); the
/// hub classifies it and answers with what it keeps.
pub async fn upload_attachment(
    settings: &Settings,
    token: &str,
    development: bool,
    name: &str,
    bytes: Vec<u8>,
    op: &Op,
) -> Result<orbit_core::attachments::Attachment> {
    #[derive(Deserialize)]
    struct Uploaded {
        attachment: orbit_core::attachments::Attachment,
    }
    let base = validate_hub_url(
        settings.hub_url.as_deref().context("Falta el hub")?,
        development,
    )?;
    let sha256 = format!("{:x}", <sha2::Sha256 as sha2::Digest>::digest(&bytes));
    let encoded_name = percent_encoded(name);
    let response = request(
        reqwest::Method::POST,
        base.join("v1/attachments")?,
        Some(token),
        op,
    )?
    .header(ATTACHMENT_NAME_HEADER, encoded_name)
    .header("X-Orbit-Sha256", &sha256)
    .body(bytes)
    .send()
    .await
    .map_err(|error| attachment_unanswered(&error))?;
    if !response.status().is_success() {
        return Err(attachment_refusal(response).await);
    }
    let uploaded: Uploaded = serde_json::from_value(bounded_json(response).await?)
        .context("Respuesta del adjunto fuera de contrato")?;
    Ok(uploaded.attachment)
}

/// «Adjuntar ventana»: the capture uploaded for `request_id`
/// (`PUT /v1/captures`) kept as an attachment named `name`.
pub async fn attachment_from_capture(
    settings: &Settings,
    token: &str,
    development: bool,
    capture_id: &str,
    request_id: &str,
    name: &str,
    op: &Op,
) -> Result<orbit_core::attachments::Attachment> {
    #[derive(Deserialize)]
    struct Kept {
        attachment: orbit_core::attachments::Attachment,
    }
    let base = validate_hub_url(
        settings.hub_url.as_deref().context("Falta el hub")?,
        development,
    )?;
    let response = request(
        reqwest::Method::POST,
        base.join("v1/attachments/capture")?,
        Some(token),
        op,
    )?
    .json(&json!({"capture_id": capture_id, "request_id": request_id, "name": name}))
    .send()
    .await
    .map_err(|error| attachment_unanswered(&error))?;
    if !response.status().is_success() {
        return Err(attachment_refusal(response).await);
    }
    let kept: Kept = serde_json::from_value(bounded_json(response).await?)
        .context("Respuesta del adjunto fuera de contrato")?;
    Ok(kept.attachment)
}

/// One attachment's bytes as the hub keeps them, at most
/// `MAX_ATTACHMENT_BYTES` (`GET /v1/attachments/{id}`).
pub async fn fetch_attachment(
    settings: &Settings,
    token: &str,
    development: bool,
    attachment_id: &str,
    op: &Op,
) -> Result<orbit_core::protocol::AttachmentContent> {
    let base = validate_hub_url(
        settings.hub_url.as_deref().context("Falta el hub")?,
        development,
    )?;
    let mut url = base.join("v1/attachments/")?;
    url.path_segments_mut()
        .map_err(|()| anyhow::anyhow!("URL del hub sin ruta"))?
        .pop_if_empty()
        .push(attachment_id);
    let response = request(reqwest::Method::GET, url, Some(token), op)?
        .send()
        .await
        .map_err(|error| attachment_unanswered(&error))?;
    if !response.status().is_success() {
        return Err(attachment_refusal(response).await);
    }
    let mime = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("application/octet-stream")
        .to_owned();
    let name = response
        .headers()
        .get(ATTACHMENT_NAME_HEADER)
        .and_then(|value| value.to_str().ok())
        .map(percent_decoded)
        .unwrap_or_else(|| "adjunto".to_owned());
    let bytes = bounded_body(response, orbit_core::limits::MAX_ATTACHMENT_BYTES)
        .await
        .map_err(|error| match error {
            BodyError::TooLarge(size) => anyhow::Error::from(size),
            BodyError::Cut { source, .. } => attachment_unanswered(&source),
        })?;
    Ok(orbit_core::protocol::AttachmentContent { name, mime, bytes })
}

pub async fn transcribe(
    settings: &Settings,
    token: &str,
    development: bool,
    wav: Vec<u8>,
    op: &Op,
) -> std::result::Result<String, orbit_core::VoiceTranscriptionFailure> {
    use orbit_core::VoiceTranscriptionFailure as Failure;
    let failure = |code: &str, hub_status: Option<u16>| Failure {
        code: code.to_owned(),
        provider_status: None,
        hub_status,
        timeout_ms: None,
        bytes: None,
        max_bytes: None,
        attempts: None,
    };
    let invalid_response = |error: anyhow::Error, status| {
        let mut result = failure("transcription_hub_invalid_response", Some(status));
        if let Some(size) = error.downcast_ref::<HubBodyTooLarge>() {
            result.code = "transcription_hub_response_too_large".into();
            result.bytes = Some(size.bytes);
            result.max_bytes = Some(size.max_bytes);
        }
        result.attempts = Some(1);
        result
    };
    let base = validate_hub_url(
        settings
            .hub_url
            .as_deref()
            .ok_or_else(|| failure("transcription_hub_not_configured", None))?,
        development,
    )
    .map_err(|_| failure("transcription_hub_invalid_url", None))?;
    let url = base
        .join("v1/audio/transcribe")
        .map_err(|_| failure("transcription_hub_invalid_url", None))?;
    let response = request(reqwest::Method::POST, url, Some(token), op)
        .map_err(|_| failure("transcription_hub_client_error", None))?
        .timeout(VOICE_HUB_HTTP_TIMEOUT)
        .header("Content-Type", "audio/wav")
        .body(wav)
        .send()
        .await
        .map_err(|error| {
            let mut result = failure(
                match TransportCause::of(&error) {
                    TransportCause::ConnectionTimeout => "transcription_hub_connection_timeout",
                    TransportCause::Timeout => "transcription_hub_timeout",
                    TransportCause::Connection => "transcription_hub_connection_error",
                    TransportCause::Cut => "transcription_hub_transport_error",
                },
                None,
            );
            if error.is_timeout() {
                // This reports the daemon→hub bound that fired, not elapsed
                // time and not either of the hub→Whisper budgets.
                result.timeout_ms = Some(
                    if error.is_connect() {
                        HUB_CONNECT_TIMEOUT
                    } else {
                        VOICE_HUB_HTTP_TIMEOUT
                    }
                    .as_millis() as u64,
                );
            }
            result.attempts = Some(1);
            result
        })?;
    if !response.status().is_success() {
        let status = response.status().as_u16();
        let value = read_refusal(response, MAX_HUB_HTTP_BODY_BYTES)
            .await
            .map_err(|error| invalid_response(error, status))?;
        let code = value.details["code"]
            .as_str()
            .or(value.code.as_deref())
            .and_then(hub_code)
            .unwrap_or("transcription_hub_invalid_response");
        let number = |key: &str| value.numbers.get(key).copied();
        let mut result = failure(code, Some(status));
        result.provider_status = number("provider_status")
            .and_then(|number| u16::try_from(number).ok())
            .filter(|status| (100..=599).contains(status));
        result.timeout_ms = number("timeout_ms");
        result.bytes = number("bytes");
        result.max_bytes = number("max_bytes");
        result.attempts = number("attempts")
            .and_then(|number| u32::try_from(number).ok())
            .or(Some(1));
        return Err(result);
    }
    let status = response.status().as_u16();
    let value: SpeechResponse = serde_json::from_value(
        bounded_json(response)
            .await
            .map_err(|error| invalid_response(error, status))?,
    )
    .map_err(|_| {
        let mut result = failure("transcription_hub_invalid_response", Some(status));
        result.attempts = Some(1);
        result
    })?;
    Ok(value.text)
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

#[cfg(test)]
mod voice_transport_tests {
    use super::*;

    #[tokio::test]
    async fn missing_and_invalid_hub_urls_have_distinct_causes() -> Result<()> {
        let missing = Settings::default();
        let invalid = Settings {
            hub_url: Some("not a hub URL".into()),
            ..Settings::default()
        };
        let Err(missing_failure) =
            transcribe(&missing, "synthetic", true, vec![0], &orbit_ipc::new_op()).await
        else {
            anyhow::bail!("missing hub URL unexpectedly transcribed voice")
        };
        let Err(invalid_failure) =
            transcribe(&invalid, "synthetic", true, vec![0], &orbit_ipc::new_op()).await
        else {
            anyhow::bail!("invalid hub URL unexpectedly transcribed voice")
        };
        assert_eq!(missing_failure.code, "transcription_hub_not_configured");
        assert_eq!(invalid_failure.code, "transcription_hub_invalid_url");
        Ok(())
    }

    #[tokio::test]
    async fn an_unreachable_hub_is_not_reported_as_unreachable_whisper() -> Result<()> {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let url = format!("http://{}", listener.local_addr()?);
        drop(listener);
        let settings = Settings {
            hub_url: Some(url),
            ..Settings::default()
        };
        let Err(failure) =
            transcribe(&settings, "synthetic", true, vec![0], &orbit_ipc::new_op()).await
        else {
            bail!("the closed loopback port unexpectedly accepted the request");
        };
        assert_eq!(failure.code, "transcription_hub_connection_error");
        assert_eq!(failure.attempts, Some(1));
        assert_eq!(failure.provider_status, None);
        Ok(())
    }
}

#[cfg(test)]
mod frame_tests {
    use super::*;

    #[test]
    fn largest_engine_page_frame_fits_outgoing_hub_bound() -> Result<()> {
        let page = orbit_core::reads::ReadId::EngineTools.max_bytes();
        let frame = command(
            "00000000-0000-4000-8000-000000000001",
            "device.reply",
            json!({
                "request_id":"00000000-0000-4000-8000-000000000001",
                "result":{
                    "tools":[{"name":"x","input_schema":"x".repeat(page)}],
                    "after_index":0,
                    "total":1
                }
            }),
        )?;
        let size = serde_json::to_vec(&frame)?.len();
        println!("outgoing engine page frame: {size} bytes");
        assert!(size <= MAX_OUTGOING_FRAME_BYTES);
        Ok(())
    }
}

#[cfg(test)]
mod watchdog_tests {
    //! Loopback WebSocket peers only: no real hub, provider or credential is
    //! contacted, and every frame is synthetic fixture noise. The watchdog
    //! window runs on the paused clock (no sleeping); the real handshakes run
    //! with the clock running so socket I/O is never starved.
    use super::*;
    use tokio::net::TcpListener;

    const CREDENTIAL: &str = "synthetic-watchdog-credential";

    /// One notification per accepted connection. The peer completes the
    /// handshake and then never speaks again: no frames, no Pong.
    async fn silent_hub() -> Result<(std::net::SocketAddr, mpsc::Receiver<()>)> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?;
        let (connections, observed) = mpsc::channel::<()>(8);
        tokio::spawn(async move {
            while let Ok((tcp, _)) = listener.accept().await {
                let connections = connections.clone();
                tokio::spawn(async move {
                    if let Ok(socket) = tokio_tungstenite::accept_async(tcp).await {
                        if connections.send(()).await.is_err() {
                            return;
                        }
                        // Hold the accepted stream open, never reading or
                        // writing again: the peer answers the handshake and
                        // then goes silent without closing the connection.
                        let _silence = socket;
                        std::future::pending::<()>().await;
                    }
                });
            }
        });
        Ok((address, observed))
    }

    struct Harness {
        commands: mpsc::Sender<Value>,
        events: mpsc::Receiver<(u64, HubEvent)>,
        task: tokio::task::JoinHandle<Result<()>>,
    }

    async fn client(address: std::net::SocketAddr) -> Harness {
        let (commands, commands_rx) = mpsc::channel(16);
        let (sender, events) = mpsc::channel(64);
        let settings = Settings {
            hub_url: Some(format!("http://{address}")),
            ..Settings::default()
        };
        let task = tokio::spawn(run(
            settings,
            CREDENTIAL.into(),
            true,
            commands_rx,
            Events {
                generation: 0,
                sender,
            },
        ));
        Harness {
            commands,
            events,
            task,
        }
    }

    #[tokio::test]
    async fn mistyped_device_request_crosses_socket_to_the_refusal_path() -> Result<()> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?;
        let server = tokio::spawn(async move {
            let (tcp, _) = listener.accept().await?;
            let mut socket = tokio_tungstenite::accept_async(tcp).await?;
            socket
                .send(Message::Text(
                    json!({"v":1,"event":"device.request","data":{
                        "request_id":"mistyped-filter","capability":"screenshot",
                        "parameters":{"application":7}
                    }})
                    .to_string()
                    .into(),
                ))
                .await?;
            while let Some(message) = socket.next().await {
                if matches!(message?, Message::Close(_)) {
                    break;
                }
            }
            anyhow::Ok(())
        });
        let mut harness = client(address).await;
        connected(&mut harness.events).await?;
        let (_, event) = tokio::time::timeout(Duration::from_secs(3), harness.events.recv())
            .await?
            .context("socket did not forward the malformed bridge request")?;
        let HubEvent::Frame(frame) = event else {
            bail!("malformed bridge request bypassed the refusal path: {event:?}");
        };
        assert_eq!(frame["data"]["request_id"], "mistyped-filter");
        assert_eq!(frame["data"]["parameters"]["application"], 7);
        drop(harness.commands);
        tokio::time::timeout(Duration::from_secs(3), harness.task).await???;
        server.await??;
        Ok(())
    }

    #[tokio::test]
    async fn incompatible_or_unreadable_hub_envelope_reports_cause_and_disconnects() -> Result<()> {
        for (frame, cause) in [
            (
                r#"{"v":2,"event":"ready","data":{"device_id":"synthetic"}}"#,
                "Versión de hub incompatible",
            ),
            ("{broken", "JSON inválido"),
            ("[]", "no es un objeto JSON"),
        ] {
            let listener = TcpListener::bind("127.0.0.1:0").await?;
            let address = listener.local_addr()?;
            let server = tokio::spawn(async move {
                let (tcp, _) = listener.accept().await?;
                let mut socket = tokio_tungstenite::accept_async(tcp).await?;
                socket.send(Message::Text(frame.into())).await?;
                anyhow::Ok(())
            });
            let mut harness = client(address).await;
            tokio::time::timeout(Duration::from_secs(3), connected(&mut harness.events)).await??;
            let (_, event) = tokio::time::timeout(Duration::from_secs(3), harness.events.recv())
                .await?
                .context("hub failure produced no notice")?;
            let HubEvent::Problem(message) = event else {
                bail!("hub failure produced no visible cause: {event:?}");
            };
            assert!(message.contains(cause), "{message}");
            assert!(message.contains("versión local 1"), "{message}");
            if cause == "Versión de hub incompatible" {
                assert!(message.contains("versión del hub 2"), "{message}");
            }
            let (_, event) = tokio::time::timeout(Duration::from_secs(3), harness.events.recv())
                .await?
                .context("hub failure left the connection pending")?;
            assert!(matches!(event, HubEvent::Disconnected), "{event:?}");
            drop(harness.commands);
            tokio::time::timeout(Duration::from_secs(3), harness.task).await???;
            server.await??;
        }
        Ok(())
    }

    #[tokio::test]
    async fn a_command_consumed_during_reconnect_is_reported_unsent() -> Result<()> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?;
        let server = tokio::spawn(async move {
            let (first, _) = listener.accept().await?;
            let mut first = tokio_tungstenite::accept_async(first).await?;
            first.close(None).await?;
            let _ = first.next().await;
            let (second, _) = listener.accept().await?;
            let mut second = tokio_tungstenite::accept_async(second).await?;
            second.close(None).await?;
            let _ = second.next().await;
            anyhow::Ok(())
        });
        let mut harness = client(address).await;
        while let Some((_, event)) = harness.events.recv().await {
            if matches!(event, HubEvent::Disconnected) {
                break;
            }
        }
        harness
            .commands
            .send(command(
                "reconnect",
                "chat.interrupt",
                serde_json::json!({"session_id":"session","turn_id":"turn"}),
            )?)
            .await?;
        drop(harness.commands);
        let result = tokio::time::timeout(Duration::from_secs(3), harness.task).await??;
        let Err(error) = result else {
            anyhow::bail!("a consumed interrupt had no socket write");
        };
        assert!(
            error.to_string().contains("1 trama"),
            "missing real unsent count: {error:#}"
        );
        server.await??;
        Ok(())
    }

    /// A frame lost in an earlier reconnect was already reported then. Once a
    /// later interrupt is written, the drain must not blame it on that loss.
    #[tokio::test]
    async fn an_old_loss_does_not_mark_a_written_interrupt_unsent() -> Result<()> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?;
        let (written, mut received) = mpsc::channel::<String>(1);
        let server = tokio::spawn(async move {
            let (first, _) = listener.accept().await?;
            let mut first = tokio_tungstenite::accept_async(first).await?;
            first.close(None).await?;
            let _ = first.next().await;
            let (second, _) = listener.accept().await?;
            let mut second = tokio_tungstenite::accept_async(second).await?;
            // The frame under test is the first text one: the client may ping
            // first, since its ping interval's first tick is immediate.
            while let Some(Ok(message)) = second.next().await {
                if let Message::Text(text) = message {
                    written.send(text.to_string()).await?;
                    break;
                }
            }
            let _ = second.next().await;
            anyhow::Ok(())
        });
        let mut harness = client(address).await;
        connected(&mut harness.events).await?;
        disconnected(&mut harness.events).await?;
        harness
            .commands
            .send(command(
                "lost",
                "chat.send",
                serde_json::json!({"text":"hola"}),
            )?)
            .await?;
        connected(&mut harness.events).await?;
        harness
            .commands
            .send(command(
                "written",
                "chat.interrupt",
                serde_json::json!({"session_id":"session","turn_id":"turn"}),
            )?)
            .await?;
        // A guard against a hang, not a measurement: the assertion is what the
        // second hub received. Three seconds ran out on the Mac with the whole
        // daemon suite in parallel (0.3.13 build, 29-09) and passed 5/5 alone.
        let text = tokio::time::timeout(Duration::from_secs(30), received.recv())
            .await?
            .context("the second hub never received the interrupt")?;
        assert!(text.contains("chat.interrupt"), "unexpected frame: {text}");
        drop(harness.commands);
        let result = tokio::time::timeout(Duration::from_secs(30), harness.task).await??;
        assert!(
            result.is_ok(),
            "a written interrupt was reported unsent: {:#}",
            result
                .err()
                .map_or_else(String::new, |error| format!("{error:#}"))
        );
        server.await??;
        Ok(())
    }

    async fn connected(events: &mut mpsc::Receiver<(u64, HubEvent)>) -> Result<()> {
        while let Some((_, event)) = events.recv().await {
            if matches!(event, HubEvent::Connected) {
                return Ok(());
            }
        }
        bail!("the hub client stopped emitting events before connecting");
    }

    async fn disconnected(events: &mut mpsc::Receiver<(u64, HubEvent)>) -> Result<()> {
        while let Some((_, event)) = events.recv().await {
            if matches!(event, HubEvent::Disconnected) {
                return Ok(());
            }
        }
        bail!("the hub client stopped emitting events without declaring the drop");
    }

    async fn problem(events: &mut mpsc::Receiver<(u64, HubEvent)>) -> Result<String> {
        while let Some((_, event)) = events.recv().await {
            if let HubEvent::Problem(message) = event {
                return Ok(message);
            }
        }
        bail!("the hub client stopped emitting events without a notice");
    }

    #[tokio::test]
    async fn silent_hub_is_declared_down_within_35_seconds_and_reconnects() -> Result<()> {
        let (address, mut connections) = silent_hub().await?;
        let mut harness = client(address).await;
        // Real-time connect: the synthetic handshake completes in milliseconds.
        tokio::time::timeout(Duration::from_secs(5), connected(&mut harness.events))
            .await
            .context("the client must connect to the synthetic hub")??;
        assert!(
            tokio::time::timeout(Duration::from_secs(5), connections.recv())
                .await
                .context("the synthetic hub must observe the first connection")?
                .is_some()
        );
        // Watchdog window: the hub stays silent past three pings (30 s) without
        // any frame or Pong; the paused clock waits without real sleeping.
        tokio::time::pause();
        tokio::time::timeout(
            Duration::from_secs(35),
            disconnected(&mut harness.events),
        )
        .await
        .context(
            "with no frames nor Pong for 30 s (three pings) the connection must be declared down within 35 s",
        )??;
        // The reconnection runs in real time with the existing backoff.
        tokio::time::resume();
        tokio::time::timeout(Duration::from_secs(10), connected(&mut harness.events))
            .await
            .context("after the drop the client must reconnect and report Connected")??;
        assert!(
            tokio::time::timeout(Duration::from_secs(5), connections.recv())
                .await
                .context("the synthetic hub must observe the reconnection")?
                .is_some()
        );
        harness.task.abort();
        Ok(())
    }

    #[tokio::test]
    async fn oversized_outgoing_frame_is_rejected_locally_with_a_notice() -> Result<()> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?;
        let (sent_sizes, mut observed_sizes) = mpsc::channel::<usize>(8);
        let server = tokio::spawn(async move {
            if let Ok((tcp, _)) = listener.accept().await
                && let Ok(mut socket) = tokio_tungstenite::accept_async(tcp).await
            {
                while let Some(message) = socket.next().await {
                    if let Ok(Message::Text(text)) = message
                        && sent_sizes.send(text.len()).await.is_err()
                    {
                        return;
                    }
                }
            }
        });
        let mut harness = client(address).await;
        tokio::time::timeout(Duration::from_secs(5), connected(&mut harness.events))
            .await
            .context("the client must connect to the synthetic hub")??;
        let oversized = command(
            "fixture-oversized",
            "chat.send",
            json!({"text": "x".repeat(200 * 1024)}),
        )?;
        assert!(
            serde_json::to_string(&oversized)?.len() > 128 * 1024,
            "the fixture frame must exceed the server's 128 KiB frame limit"
        );
        harness.commands.send(oversized).await?;
        tokio::time::timeout(Duration::from_secs(5), problem(&mut harness.events))
            .await
            .context("a 200 KiB frame must be rejected locally with a notice")??;
        assert!(
            tokio::time::timeout(Duration::from_secs(1), observed_sizes.recv())
                .await
                .is_err(),
            "the 200 KiB frame must never reach the hub socket"
        );
        harness.task.abort();
        server.abort();
        Ok(())
    }

    /// One accepted connection that reports back every text frame it received.
    async fn recording_hub() -> Result<(std::net::SocketAddr, mpsc::Receiver<String>)> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?;
        let (sent, observed) = mpsc::channel::<String>(8);
        tokio::spawn(async move {
            if let Ok((tcp, _)) = listener.accept().await
                && let Ok(mut socket) = tokio_tungstenite::accept_async(tcp).await
            {
                let ready = json!({"v":1,"event":"ready","data":{"device_id":"synthetic-device","hub_protocol":TYPED_REFUSAL_HUB_PROTOCOL}});
                if socket
                    .send(Message::Text(ready.to_string().into()))
                    .await
                    .is_err()
                {
                    return;
                }
                while let Some(message) = socket.next().await {
                    if let Ok(Message::Text(text)) = message
                        && sent.send(text.to_string()).await.is_err()
                    {
                        return;
                    }
                }
            }
        });
        Ok((address, observed))
    }

    #[tokio::test]
    async fn oversized_reply_to_modern_hub_keeps_its_size_and_ceiling() -> Result<()> {
        let (address, mut observed) = recording_hub().await?;
        let mut harness = client(address).await;
        connected(&mut harness.events).await?;
        let ready = harness.events.recv().await.context("modern ready absent")?;
        assert!(matches!(&ready, (_, HubEvent::Decoded(_))), "{ready:?}");
        let oversized = command(
            "oversized-reply",
            "device.reply",
            json!({"request_id":"oversized-reply","result":{"windows":[{"application":"x".repeat(200 * 1024),"title":"Ventana"}]}}),
        )?;
        let bytes = serde_json::to_string(&oversized)?.len();
        harness.commands.send(oversized).await?;
        let wire = observed.recv().await.context("typed refusal absent")?;
        let frame: Value = serde_json::from_str(&wire)?;
        assert_eq!(frame["params"]["refusal"]["code"], "too_large");
        assert_eq!(frame["params"]["refusal"]["bytes"], bytes);
        assert_eq!(
            frame["params"]["refusal"]["max_bytes"],
            MAX_OUTGOING_FRAME_BYTES
        );
        assert!(frame["params"].get("refused").is_none());
        harness.task.abort();
        Ok(())
    }

    /// The substitute exists for the one shape that has a caller waiting on
    /// an id. A turn of the person's own conversation keeps the behaviour it
    /// had: the notice, and nothing invented on the wire.
    #[test]
    fn only_a_reply_with_a_caller_gets_a_substitute() -> Result<()> {
        let reply = command(
            "request",
            "device.reply",
            json!({"request_id": "request", "result": {"tools": [], "after_index": 0, "total": 0}}),
        )?;
        let substitute = oversize_refusal(&reply, 200_000)?.context("a reply is answered")?;
        assert_eq!(substitute["params"]["refusal"]["code"], "too_large");
        assert_eq!(substitute["params"]["refusal"]["bytes"], 200_000);
        assert_eq!(
            substitute["params"]["refusal"]["max_bytes"],
            MAX_OUTGOING_FRAME_BYTES
        );

        // A conversation turn has no waiting caller and no id to answer.
        let turn = command("turn", "chat.send", json!({"text": "hola"}))?;
        assert!(oversize_refusal(&turn, 200_000)?.is_none());

        // A refusal that is itself too large has nothing shorter left to say.
        let refusal = command(
            "request",
            "device.reply",
            json!({"request_id": "request", "refusal": {"code":"helper_not_running"}}),
        )?;
        assert!(oversize_refusal(&refusal, 200_000)?.is_none());

        // A reply without a request id has no destination.
        let orphan = json!({"v": 1, "id": "request", "method": "device.reply", "params": {"result": {"windows": []}}});
        assert!(oversize_refusal(&orphan, 200_000)?.is_none());
        Ok(())
    }
}

#[cfg(test)]
mod hub_unreached_tests {
    use super::*;
    use tokio_tungstenite::tungstenite::{Error, http::Response};

    /// Every failure to open the hub's WebSocket said «No se pudo establecer
    /// una conexión TLS autenticada», also a plain-HTTP hub that was down
    /// (Orbit Pruebas, 2026-10-18): each cause is named, with where.
    #[test]
    fn a_failed_connection_names_its_cause_and_where() {
        let origin = "100.64.0.1:8710";
        let refused = unreached(
            &Error::Io(std::io::Error::from(std::io::ErrorKind::ConnectionRefused)),
            origin,
        );
        assert!(
            refused.contains("rechazó la conexión") && refused.contains(origin),
            "{refused}"
        );
        assert!(!refused.contains("TLS"), "{refused}");
        let status = unreached(
            &Error::Http(Box::new(
                Response::builder()
                    .status(502)
                    .body(None)
                    .unwrap_or_else(|_| Response::new(None)),
            )),
            origin,
        );
        assert!(
            status.contains("502") && status.contains(origin),
            "{status}"
        );
        let late = timed_out(origin);
        assert!(late.contains("10 s") && late.contains(origin), "{late}");
    }
}

#[cfg(test)]
mod http_client_tests {
    use super::*;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

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
