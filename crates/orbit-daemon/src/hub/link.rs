//! The device's WebSocket link to the hub: one connection at a time, its
//! frames decoded and handed to the daemon, the daemon's frames written in
//! order, and the reconnection with its backoff. A frame is never replayed.
use super::*;
use futures_util::{SinkExt, StreamExt};
use orbit_core::limits::MAX_HUB_INCOMING_FRAME_BYTES;
use orbit_core::wire::hub::{
    HUB_PROTOCOL, HubDecodeError, HubEnvelopeError, HubEvent as WireHubEvent, HubRequest,
};
use std::fmt;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::{Message, client::IntoClientRequest, http::HeaderValue};

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
            "The hub did not write {} {frames}: {}",
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
                        || "The sender closed before completing the drain".to_owned(),
                        |error| error.to_string(),
                    ),
                }
                .into());
            }
            return Ok(());
        }
        if let Err(error) = &result {
            events
                .send(HubEvent::Problem(format!("Hub unavailable: {error}")))
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
                    cause: "The connection was waiting for a retry".to_owned(),
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
            format!("The hub at {origin} refused the connection: nothing is listening on that port")
        }
        Error::Io(io) => format!("The network could not reach the hub at {origin}: {io}"),
        Error::Tls(tls) => format!("The TLS connection to the hub at {origin} failed: {tls}"),
        Error::Http(response) => format!(
            "The hub at {origin} returned {} when opening the WebSocket",
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
            format!("The WebSocket to the hub at {origin} did not open: {other}")
        }
    }
}

fn timed_out(origin: &str) -> String {
    format!(
        "The hub at {origin} did not complete the connection within {} s",
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
    let (request, origin) = upgrade_request(settings, token, development)?;
    let mut socket = open(request, &origin, events).await?;
    events.send(HubEvent::Connected).await?;
    let mut ping = tokio::time::interval(Duration::from_secs(10));
    let mut watchdog = Box::pin(tokio::time::sleep(Duration::from_secs(30)));
    loop {
        tokio::select! {
            message = socket.next() => match message {
                Some(Ok(Message::Text(text))) => {
                    relay_frame(&text, events).await?;
                    watchdog.as_mut().reset(tokio::time::Instant::now() + Duration::from_secs(30));
                },
                Some(Ok(Message::Ping(data))) => {
                    let body = Message::Pong(data);
                    tokio::time::timeout(Duration::from_secs(5), socket.send(body))
                        .await
                        .context("Sending Pong to the hub exceeded the 5 s deadline")??;
                    watchdog.as_mut().reset(tokio::time::Instant::now() + Duration::from_secs(30));
                },
                Some(Ok(Message::Close(_))) | None => return Ok(()),
                Some(Err(_)) => bail!("The connection was interrupted; commands are not repeated"),
                _ => {}
            },
            command = commands.recv() => match command {
                Some(command) => send_command(&mut socket, events, &command, unsent).await?,
                None => { socket.close(None).await.context("Closing the hub socket failed")?; return Ok(()); }
            },
            _ = ping.tick() => {
                tokio::time::timeout(Duration::from_secs(5), socket.send(Message::Ping(Vec::new().into())))
                    .await
                    .context("Sending ping to the hub exceeded the 5 s deadline")??;
            },
            _ = watchdog.as_mut() => {
                bail!("The server remained silent for the liveness deadline (30 s)");
            }
        }
    }
}

/// The hub's WebSocket while it is open.
type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// The upgrade request to the hub's `/v1/ws`, with the device's credential
/// and a new op, and the hub's host and port to name it by.
fn upgrade_request(
    settings: &Settings,
    token: &str,
    development: bool,
) -> Result<(
    tokio_tungstenite::tungstenite::handshake::client::Request,
    String,
)> {
    let mut url = validate_hub_url(
        settings.hub_url.as_deref().context("Device not paired")?,
        development,
    )?;
    let scheme = if url.scheme() == "https" { "wss" } else { "ws" };
    url.set_scheme(scheme)
        .map_err(|_| anyhow::anyhow!("Invalid scheme"))?;
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
    let origin = format!(
        "{}:{}",
        url.host_str().unwrap_or("?"),
        url.port_or_known_default().unwrap_or_default()
    );
    Ok((request, origin))
}

/// Opens the WebSocket within [`HUB_CONNECT_WAIT`]. A credential the hub
/// refuses is said as revoked; every other failure names its cause.
async fn open(
    request: tokio_tungstenite::tungstenite::handshake::client::Request,
    origin: &str,
    events: &Events,
) -> Result<Socket> {
    let config = tokio_tungstenite::tungstenite::protocol::WebSocketConfig::default()
        .max_message_size(Some(MAX_HUB_INCOMING_FRAME_BYTES))
        .max_frame_size(Some(MAX_HUB_INCOMING_FRAME_BYTES));
    let connect = tokio_tungstenite::connect_async_with_config(request, Some(config), false);
    match tokio::time::timeout(HUB_CONNECT_WAIT, connect).await {
        Err(_) => bail!("{}", timed_out(origin)),
        Ok(Ok((socket, _))) => Ok(socket),
        Ok(Err(tokio_tungstenite::tungstenite::Error::Http(response)))
            if matches!(response.status().as_u16(), 401 | 403) =>
        {
            events.send(HubEvent::Revoked).await?;
            bail!("Credential rejected; pair the device again");
        }
        Ok(Err(error)) => bail!("{}", unreached(&error, origin)),
    }
}

/// One text frame from the hub, decoded and handed to the daemon. A frame
/// of another protocol version ends the connection; an event that does not
/// decode is said by its name.
async fn relay_frame(text: &str, events: &Events) -> Result<()> {
    let value: Value = match serde_json::from_str(text) {
        Ok(value) => value,
        Err(error) => {
            bail!(
                "The hub sent invalid JSON: local version {HUB_PROTOCOL}, unreadable hub version ({error})"
            );
        }
    };
    let rejected_bridge = (value.get("event").and_then(Value::as_str) == Some("device.request")
        && value
            .get("data")
            .and_then(|data| data.get("request_id"))
            .and_then(Value::as_str)
            .is_some())
    .then(|| value.clone());
    match WireHubEvent::decode_detailed(value) {
        Ok(decoded) => {
            events.send(HubEvent::Decoded(Box::new(decoded))).await?;
        }
        Err(HubDecodeError::Envelope(HubEnvelopeError::UnsupportedVersion { received })) => {
            bail!("Incompatible hub version: local version {HUB_PROTOCOL}, hub version {received}");
        }
        Err(HubDecodeError::Envelope(HubEnvelopeError::MissingOrInvalidVersion)) => {
            bail!(
                "Incompatible hub version: local version {HUB_PROTOCOL}, hub version absent or unreadable"
            );
        }
        Err(HubDecodeError::Envelope(HubEnvelopeError::NotObject)) => {
            bail!(
                "The hub frame is not a JSON object: local version {HUB_PROTOCOL}, hub version unknown"
            );
        }
        Err(HubDecodeError::Event(reason)) => {
            if let Some(value) = rejected_bridge {
                // A malformed bridge request still has an owner waiting for a
                // refusal. Keep its raw shape only until the daemon names the
                // invalid parameter; never execute it.
                events.send(HubEvent::Frame(value)).await?;
            } else {
                let event = invalid_event_name(&reason).to_owned();
                events
                    .send(HubEvent::InvalidFrame { event, reason })
                    .await?;
            }
        }
    }
    Ok(())
}

/// One frame of the daemon's, written to the hub. A frame larger than the
/// hub takes does not go; the person is told, and a reply someone waits for
/// is answered with a short refusal in its place.
async fn send_command(
    socket: &mut Socket,
    events: &Events,
    command: &Value,
    unsent: &mut usize,
) -> Result<()> {
    let body = serde_json::to_string(command)?;
    if body.len() > MAX_OUTGOING_FRAME_BYTES {
        events
            .send(HubEvent::Problem(format!(
                "Frame of {} bytes exceeds the hub limit ({} bytes): the turn is not sent",
                body.len(),
                MAX_OUTGOING_FRAME_BYTES,
            )))
            .await?;
        // The frame does not go, but the silence does not
        // stay either. Whoever asked gets the cause instead of
        // waiting out its whole timeout on a Mac that already
        // decided.
        if let Some(refusal) = oversize_refusal(command, body.len())? {
            let encoded = serde_json::to_string(&refusal)?;
            if encoded.len() <= MAX_OUTGOING_FRAME_BYTES {
                write_frame(socket, encoded).await?;
            }
        }
    } else {
        write_frame(socket, body).await?;
        *unsent = 0;
    }
    Ok(())
}

/// Writes one text frame within 5 s; a frame that did not go is counted
/// as unsent, with its cause.
async fn write_frame(socket: &mut Socket, body: String) -> Result<()> {
    let body = Message::Text(body.into());
    let sent = tokio::time::timeout(Duration::from_secs(5), socket.send(body))
        .await
        .context("Sending to the hub exceeded the 5 s deadline")
        .and_then(|outcome| outcome.map_err(Into::into));
    if let Err(error) = sent {
        return Err(UnsentFrames {
            count: 1,
            cause: error.to_string(),
        }
        .into());
    }
    Ok(())
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
        .context("device.reply.params violates the contract")?;
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
                "Incompatible hub version",
            ),
            ("{broken", "invalid JSON"),
            ("[]", "is not a JSON object"),
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
            assert!(message.contains("local version 1"), "{message}");
            if cause == "Incompatible hub version" {
                assert!(message.contains("hub version 2"), "{message}");
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
            refused.contains("refused the connection") && refused.contains(origin),
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
