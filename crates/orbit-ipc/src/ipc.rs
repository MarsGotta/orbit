//! Owner-only Unix IPC; bounded frames and one request per connection.
use crate::Paths;
use anyhow::{Context, Result, bail};
use orbit_core::{
    PROTOCOL_VERSION,
    limits::MAX_IPC_LINE_BYTES,
    protocol::{
        Command, Request, Response, TopicEvent, TopicResponse, VoiceNoteAudio,
        VoiceNoteAudioResponse,
    },
    reads::ReadRequest,
    refusal::{Refusal, Resource},
    topics::{TopicFrame, TopicId, read_transport_max_bytes, transport_max_bytes},
};
use std::{
    collections::BTreeMap,
    os::unix::fs::{FileTypeExt, MetadataExt, OpenOptionsExt, PermissionsExt},
    sync::Arc,
    time::Duration,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::{UnixListener, UnixStream, unix::OwnedReadHalf},
    sync::{Semaphore, mpsc, oneshot, watch},
};

mod budget;
pub use budget::{bound_topic_frame, bound_topic_response};
#[cfg(test)]
mod tests;

/// Four independent topic slots let the same four desktop clients hold one
/// topic stream each. The bound protects the registered
/// per-topic watch receivers per client; it is not a topic or command limit.
/// A fifth topic subscriber is refused with its 5/4 count.
const MAX_TOPIC_SUBSCRIPTIONS: usize = 4;

// The existing 64 slots bound active IPC handlers. Four extra readers let one
// rejected connection per desktop client recover its id, using at most 4 MiB
// of frame storage. Further peers get an immediate refusal without reading an
// id; they cannot create tasks or allocate another request frame.
const MAX_CONNECTIONS: usize = 64;
const MAX_BUSY_READERS: usize = 4;

pub struct Call {
    pub request: Request,
    pub reply: ReplySender,
}

#[derive(Debug)]
pub enum Reply {
    Command(Response),
    VoiceNoteAudio(VoiceNoteAudioResponse),
    Topics(TopicResponse),
}

impl From<Response> for Reply {
    fn from(response: Response) -> Self {
        Self::Command(response)
    }
}

impl From<VoiceNoteAudioResponse> for Reply {
    fn from(response: VoiceNoteAudioResponse) -> Self {
        Self::VoiceNoteAudio(response)
    }
}

impl From<TopicResponse> for Reply {
    fn from(response: TopicResponse) -> Self {
        Self::Topics(response)
    }
}

pub type TopicChannels = BTreeMap<TopicId, watch::Sender<TopicFrame>>;

pub struct ReplySender(oneshot::Sender<Reply>);

#[derive(Debug)]
pub struct ReplySendError;

impl ReplySender {
    pub async fn closed(&mut self) {
        self.0.closed().await;
    }

    pub fn send(self, response: impl Into<Reply>) -> std::result::Result<(), ReplySendError> {
        self.0.send(response.into()).map_err(|_| ReplySendError)
    }
}

/// How the daemon's thread ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DaemonEnded {
    /// Its closure ran to the end: a `Shutdown`, a signal or its own decision.
    Closed,
    /// It returned an error; the daemon's own words.
    Failed(String),
    /// The thread ended without a word: it panicked, and the panic was
    /// printed (and reported, if the user allows it) where it happened.
    Vanished,
}

impl DaemonEnded {
    pub fn describe(&self) -> String {
        match self {
            Self::Closed => "cerró en orden".into(),
            Self::Failed(error) => format!("terminó con un error: {error}"),
            Self::Vanished => "su hilo terminó sin resultado (un pánico)".into(),
        }
    }

    pub fn exit_code(&self) -> i32 {
        match self {
            Self::Closed => 0,
            Self::Failed(_) | Self::Vanished => 1,
        }
    }
}

/// App-core's typed access to the actor already inside its process. Receivers
/// do not keep topic producers alive; the thread owner alone reports its end.
#[derive(Clone)]
pub struct LocalClient {
    calls: mpsc::Sender<Call>,
    topics: BTreeMap<TopicId, watch::Receiver<TopicFrame>>,
    ended: watch::Receiver<Option<DaemonEnded>>,
}

impl LocalClient {
    pub fn new(
        calls: mpsc::Sender<Call>,
        topics: &TopicChannels,
        ended: watch::Receiver<Option<DaemonEnded>>,
    ) -> Self {
        Self {
            calls,
            topics: topics
                .iter()
                .map(|(id, sender)| (*id, sender.subscribe()))
                .collect(),
            ended,
        }
    }

    pub fn topics(&self) -> BTreeMap<TopicId, watch::Receiver<TopicFrame>> {
        self.topics.clone()
    }

    pub async fn ended(&self) -> DaemonEnded {
        let mut ended = self.ended.clone();
        match ended.wait_for(Option::is_some).await {
            Ok(value) => value.clone().unwrap_or(DaemonEnded::Vanished),
            Err(_) => DaemonEnded::Vanished,
        }
    }

    async fn stopped(&self) -> anyhow::Error {
        match self.ended().await {
            DaemonEnded::Closed => anyhow::anyhow!("orbit_closing"),
            DaemonEnded::Failed(cause) => anyhow::anyhow!("El servicio de Orbit terminó: {cause}"),
            DaemonEnded::Vanished => {
                anyhow::anyhow!("El hilo del servicio de Orbit terminó sin resultado (un pánico)")
            }
        }
    }

    async fn call(&self, command: Command, op: Option<orbit_core::notice::Op>) -> Result<Reply> {
        let (reply, receive) = oneshot::channel();
        let call = Call {
            request: Request {
                v: PROTOCOL_VERSION,
                id: uuid::Uuid::new_v4().to_string(),
                op,
                body: command,
            },
            reply: ReplySender(reply),
        };
        // The channel already bounds admission at 32 calls. Keep the existing
        // ten-second confirmation window, including queue time, until the
        // caller decides whether a slow order may still be running. Never retry.
        tokio::time::timeout(Duration::from_secs(10), async {
            if self.calls.send(call).await.is_err() {
                return Err(self.stopped().await);
            }
            match receive.await {
                Ok(reply) => Ok(reply),
                Err(_) => Err(self.stopped().await),
            }
        })
        .await
        .context("El daemon no confirmó en 10000 ms; no se repite la orden ni la lectura")?
    }

    pub async fn request_command(&self, command: Command) -> Result<Response> {
        self.request_command_as(command, new_op()).await
    }

    pub async fn request_command_as(
        &self,
        command: Command,
        op: orbit_core::notice::Op,
    ) -> Result<Response> {
        match self.call(command, Some(op)).await? {
            Reply::Command(response) => {
                if let Some(refusal) = response.refusal.clone() {
                    return Err(refusal.into());
                }
                if !response.ok {
                    bail!(
                        "{}",
                        response
                            .error
                            .as_deref()
                            .unwrap_or("Operación rechazada sin causa")
                    );
                }
                Ok(response)
            }
            Reply::Topics(_) | Reply::VoiceNoteAudio(_) => {
                bail!("El daemon entregó otra clase de respuesta a una orden")
            }
        }
    }

    pub async fn read_topic(&self, read: ReadRequest) -> Result<TopicResponse> {
        match self.call(Command::Read { read }, None).await? {
            Reply::Topics(response) => {
                if let Some(refusal) = response.refusal.clone() {
                    return Err(refusal.into());
                }
                if !response.ok {
                    bail!("El daemon rechazó la lectura sin causa tipada");
                }
                Ok(response)
            }
            // During recovery the actor answers a read with its typed refusal.
            Reply::Command(response) => match response.refusal {
                Some(refusal) => Err(refusal.into()),
                None => bail!("El daemon entregó una orden en lugar de la lectura"),
            },
            Reply::VoiceNoteAudio(_) => bail!("El daemon entregó audio en lugar de la lectura"),
        }
    }

    pub async fn voice_note_audio(&self, message_id: String) -> Result<VoiceNoteAudio> {
        match self
            .call(Command::VoiceNoteAudio { message_id }, None)
            .await?
        {
            Reply::VoiceNoteAudio(response) => {
                if let Some(refusal) = response.refusal {
                    return Err(refusal.into());
                }
                if !response.ok {
                    bail!(
                        "{}",
                        response
                            .error
                            .as_deref()
                            .unwrap_or("Audio rechazado sin causa")
                    );
                }
                response
                    .audio
                    .context("El daemon no entregó el audio solicitado")
            }
            Reply::Command(response) => match response.refusal {
                Some(refusal) => Err(refusal.into()),
                None => bail!("El daemon entregó una orden en lugar del audio"),
            },
            Reply::Topics(_) => bail!("El daemon entregó una lectura en lugar del audio"),
        }
    }
}

pub struct Subscription {
    reader: BufReader<OwnedReadHalf>,
    // Kept alive on purpose: dropping the write half shuts the socket's write
    // side down, the daemon reads EOF and, since the daemon releases the slot on
    // EOF, ends the subscription right after the handshake. A subscriber that
    // never writes still has to hold its end of the socket open.
    _writer: tokio::net::unix::OwnedWriteHalf,
}

impl Subscription {
    pub async fn next_topic(&mut self) -> Result<TopicFrame> {
        let bytes = read_frame_with_limit(&mut self.reader, transport_max_bytes()).await?;
        match serde_json::from_slice::<TopicEvent>(&bytes)? {
            TopicEvent::Changed { frame } => Ok(frame),
        }
    }
}

/// Hold this file for the complete daemon lifetime. The lock is acquired before
/// opening storage, so a competing process cannot recover or rewrite live state.
pub fn lock_instance(paths: &Paths) -> Result<std::fs::File> {
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .mode(0o600)
        .custom_flags(i32::try_from(rustix::fs::OFlags::NOFOLLOW.bits())?)
        .open(paths.root.join("daemon.lock"))?;
    let metadata = lock.metadata()?;
    if !metadata.is_file() || metadata.uid() != rustix::process::geteuid().as_raw() {
        bail!("El bloqueo del daemon no pertenece al usuario actual");
    }
    rustix::fs::flock(&lock, rustix::fs::FlockOperation::NonBlockingLockExclusive)
        .context("Ya hay un servicio Orbit activo o no se puede bloquear este directorio")?;
    lock.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    Ok(lock)
}

/// Retire an owner-local socket path. The daemon calls this only while holding
/// daemon.lock; the wire listener calls it after proving the old peer is gone.
/// A symlink or a regular file is never removed as though it were a socket.
pub fn remove_owned_socket(paths: &Paths) -> Result<()> {
    let meta = match std::fs::symlink_metadata(&paths.socket) {
        Ok(meta) => meta,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error).context("No se pudo inspeccionar la ruta IPC retirada"),
    };
    if !meta.file_type().is_socket() {
        bail!(
            "Ruta IPC ocupada en {}: no es un socket; no se elimina",
            paths.socket.display()
        );
    }
    let owner = rustix::process::geteuid().as_raw();
    if meta.uid() != owner {
        bail!(
            "Ruta IPC ocupada en {}: uid {}, esperado {}; no se elimina",
            paths.socket.display(),
            meta.uid(),
            owner
        );
    }
    std::fs::remove_file(&paths.socket)?;
    Ok(())
}

pub async fn request(paths: &Paths, command: Command) -> Result<Response> {
    tokio::time::timeout(Duration::from_secs(10), request_inner(paths, command))
        .await
        .context("El daemon no respondió dentro del plazo; no se repite la orden")?
}

pub async fn request_command(paths: &Paths, command: Command) -> Result<Response> {
    request_command_as(paths, command, new_op()).await
}

/// A new op for an action that starts in this process, so its notices and
/// reports can be followed from end to end.
pub fn new_op() -> orbit_core::notice::Op {
    uuid::Uuid::new_v4().into()
}

/// Sends one order on behalf of the action `op`, so the daemon, the hub and
/// every notice the order causes carry it.
pub async fn request_command_as(
    paths: &Paths,
    command: Command,
    op: orbit_core::notice::Op,
) -> Result<Response> {
    tokio::time::timeout(Duration::from_secs(10), async {
        let mut stream = connect(paths).await?;
        let request = Request {
            v: PROTOCOL_VERSION,
            id: uuid::Uuid::new_v4().to_string(),
            op: Some(op),
            body: command,
        };
        let frame = encode_request(&request)?;
        stream.write_all(&frame).await?;
        let bytes = read_frame(BufReader::new(stream)).await?;
        let response: Response = serde_json::from_slice(&bytes)?;
        if response.v != request.v || response.id != request.id {
            bail!("Respuesta IPC no válida");
        }
        if !response.ok {
            if let Some(refusal) = response.refusal {
                return Err(refusal.into());
            }
            bail!(
                "{}",
                response.error.as_deref().unwrap_or("Operación rechazada")
            );
        }
        Ok(response)
    })
    .await
    .context("El daemon no respondió dentro del plazo; no se repite la orden")?
}

pub async fn voice_note_audio(paths: &Paths, message_id: String) -> Result<VoiceNoteAudio> {
    tokio::time::timeout(
        Duration::from_secs(10),
        voice_note_audio_inner(paths, message_id),
    )
    .await
    .context("El daemon no respondió dentro del plazo; no se repite la lectura")?
}

pub async fn subscribe_topics(
    paths: &Paths,
    topics: Vec<TopicId>,
) -> Result<(TopicResponse, Subscription)> {
    tokio::time::timeout(Duration::from_secs(10), async {
        let mut stream = connect(paths).await?;
        let request = Request {
            v: PROTOCOL_VERSION,
            id: uuid::Uuid::new_v4().to_string(),
            op: None,
            body: Command::SubscribeTopics { topics },
        };
        let frame = encode_request(&request)?;
        stream.write_all(&frame).await?;
        let (reader, writer) = stream.into_split();
        let mut subscription = Subscription {
            reader: BufReader::new(reader),
            _writer: writer,
        };
        let bytes = read_frame_with_limit(&mut subscription.reader, transport_max_bytes()).await?;
        let response = decode_topic_response(&request, &bytes)?;
        Ok((response, subscription))
    })
    .await
    .context("El daemon no abrió la suscripción de temas dentro del plazo")?
}

/// A keyed read may cross a slow hub page. Ten seconds bounds this IPC
/// request; the daemon's eight-second session budget leaves two seconds to
/// frame a typed reply. This is not a hub response-time promise.
pub const KEYED_READ_TIMEOUT: Duration = Duration::from_secs(10);

pub async fn read_topic(paths: &Paths, read: ReadRequest) -> Result<TopicResponse> {
    tokio::time::timeout(KEYED_READ_TIMEOUT, async {
        let mut stream = connect(paths).await?;
        let request = Request {
            v: PROTOCOL_VERSION,
            id: uuid::Uuid::new_v4().to_string(),
            op: None,
            body: Command::Read { read },
        };
        let frame = encode_request(&request)?;
        stream.write_all(&frame).await?;
        let bytes =
            read_frame_with_limit(BufReader::new(stream), read_transport_max_bytes()).await?;
        decode_topic_response(&request, &bytes)
    })
    .await
    .context("El daemon no completó la lectura dentro del plazo")?
}

fn decode_topic_response(request: &Request, bytes: &[u8]) -> Result<TopicResponse> {
    let response: TopicResponse =
        serde_json::from_slice(bytes).context("Respuesta IPC de temas no válida")?;
    if response.id != request.id || response.v != PROTOCOL_VERSION {
        bail!(
            "Respuesta IPC de temas no válida: versión {} (requerida {}), identificador coincidente: {}",
            response.v,
            PROTOCOL_VERSION,
            response.id == request.id
        );
    }
    if !response.ok {
        if let Some(refusal) = response.refusal {
            return Err(refusal.into());
        }
        bail!("El daemon rechazó la orden de temas sin causa tipada");
    }
    Ok(response)
}

async fn request_inner(paths: &Paths, command: Command) -> Result<Response> {
    let mut stream = connect(paths).await?;
    let request = Request {
        v: PROTOCOL_VERSION,
        id: uuid::Uuid::new_v4().to_string(),
        op: Some(new_op()),
        body: command,
    };
    let frame = encode_request(&request)?;
    stream.write_all(&frame).await?;
    let bytes = read_frame(BufReader::new(stream)).await?;
    let response: Response = serde_json::from_slice(&bytes)?;
    validate_response(&request, &response)?;
    Ok(response)
}

async fn voice_note_audio_inner(paths: &Paths, message_id: String) -> Result<VoiceNoteAudio> {
    let mut stream = connect(paths).await?;
    let request = Request {
        v: PROTOCOL_VERSION,
        id: uuid::Uuid::new_v4().to_string(),
        op: None,
        body: Command::VoiceNoteAudio { message_id },
    };
    let frame = encode_request(&request)?;
    stream.write_all(&frame).await?;
    let bytes = read_frame_with_limit(
        BufReader::new(stream),
        budget::MAX_VOICE_NOTE_RESPONSE_FRAME_BYTES,
    )
    .await?;
    let response: VoiceNoteAudioResponse = serde_json::from_slice(&bytes)?;
    if response.v != request.v || response.id != request.id {
        bail!("Respuesta IPC no válida");
    }
    if !response.ok {
        if let Some(refusal) = response.refusal {
            return Err(refusal.into());
        }
        bail!(
            "{}",
            response.error.as_deref().unwrap_or("Operación rechazada")
        );
    }
    response.audio.context("Respuesta de nota de voz no válida")
}

async fn connect(paths: &Paths) -> Result<UnixStream> {
    let stream = UnixStream::connect(&paths.socket)
        .await
        .context("El servicio de Orbit no está activo. Abre Orbit.app")?;
    if stream.peer_cred()?.uid() != rustix::process::geteuid().as_raw() {
        bail!("El servicio IPC pertenece a otro usuario; no se ha enviado la orden");
    }
    Ok(stream)
}

fn validate_response(request: &Request, response: &Response) -> Result<()> {
    validate_response_identity(request, response)?;
    if !response.ok {
        if let Some(refusal) = response.refusal.clone() {
            return Err(refusal.into());
        }
        bail!(
            "{}",
            response.error.as_deref().unwrap_or("Operación rechazada")
        );
    }
    Ok(())
}

fn validate_response_identity(request: &Request, response: &Response) -> Result<()> {
    if response.v != PROTOCOL_VERSION || response.id != request.id {
        bail!("Respuesta IPC no válida");
    }
    Ok(())
}

pub async fn listen(
    paths: &Paths,
    sender: mpsc::Sender<Call>,
    topic_channels: TopicChannels,
    ready: Option<oneshot::Sender<()>>,
) -> Result<()> {
    paths.initialize()?;
    if paths.socket.exists() {
        if UnixStream::connect(&paths.socket).await.is_ok() {
            bail!("Ya hay un servicio Orbit activo");
        }
        remove_owned_socket(paths)?;
    }
    let listener = UnixListener::bind(&paths.socket)?;
    std::fs::set_permissions(&paths.socket, std::fs::Permissions::from_mode(0o600))?;
    if let Some(ready) = ready {
        let _ = ready.send(());
    }
    let capacity = Arc::new(Semaphore::new(MAX_CONNECTIONS));
    let busy_readers = Arc::new(Semaphore::new(MAX_BUSY_READERS));
    let topic_subscribers = Arc::new(Semaphore::new(MAX_TOPIC_SUBSCRIPTIONS));
    loop {
        let (stream, _) = listener.accept().await?;
        // On Darwin, a peer may disconnect between accept and getpeereid.
        // An invalid individual connection must never stop the IPC listener.
        if !stream
            .peer_cred()
            .is_ok_and(|peer| peer.uid() == rustix::process::geteuid().as_raw())
        {
            continue;
        }
        let Ok(permit) = capacity.clone().try_acquire_owned() else {
            if let Ok(permit) = busy_readers.clone().try_acquire_owned() {
                tokio::spawn(async move {
                    let _permit = permit;
                    let _ =
                        tokio::time::timeout(Duration::from_secs(10), reject_busy(stream)).await;
                });
            } else {
                let response = budget::encode_response(refused_response(
                    String::new(),
                    connection_refusal(MAX_CONNECTIONS + MAX_BUSY_READERS + 1),
                ))?;
                // No reader slot: a nonblocking write cannot stall admission.
                let mut stream = stream.into_std()?;
                let _ = std::io::Write::write_all(&mut stream, &response);
            }
            continue;
        };
        let sender = sender.clone();
        let topic_subscribers = topic_subscribers.clone();
        let topic_channels = topic_channels.clone();
        tokio::spawn(async move {
            let _permit = permit;
            let _ = handle_connection(stream, sender, topic_channels, topic_subscribers).await;
        });
    }
}

#[cfg(test)]
async fn handle(stream: UnixStream, sender: mpsc::Sender<Call>) -> Result<()> {
    handle_connection(
        stream,
        sender,
        TopicChannels::new(),
        Arc::new(Semaphore::new(MAX_TOPIC_SUBSCRIPTIONS)),
    )
    .await
}

async fn handle_connection(
    stream: UnixStream,
    sender: mpsc::Sender<Call>,
    topic_channels: TopicChannels,
    topic_subscribers: Arc<Semaphore>,
) -> Result<()> {
    let (reader, mut writer) = stream.into_split();
    let mut reader = BufReader::new(reader);
    let Some(request) = receive_request(&mut reader, &mut writer).await? else {
        return Ok(());
    };
    let topic_subscription = matches!(request.body, Command::SubscribeTopics { .. });
    let selected = if let Command::SubscribeTopics { topics } = &request.body {
        let ids = if topics.is_empty() {
            TopicId::ALL.to_vec()
        } else {
            topics.clone()
        };
        ids.into_iter()
            .filter_map(|id| {
                topic_channels
                    .get(&id)
                    .map(|sender| (id, sender.subscribe()))
            })
            .collect::<BTreeMap<_, _>>()
    } else {
        BTreeMap::new()
    };
    let subscriber_permit = if topic_subscription {
        let max = MAX_TOPIC_SUBSCRIPTIONS;
        match topic_subscribers.try_acquire_owned() {
            Ok(permit) => Some(permit),
            Err(_) => {
                let refusal = Refusal::TooMany {
                    resource: Resource::Subscriptions,
                    count: (max + 1) as u64,
                    max: max as u64,
                };
                let response = Response {
                    v: PROTOCOL_VERSION,
                    id: request.id,
                    ok: false,
                    status: None,
                    error: Some(refusal.to_string()),
                    refusal: Some(refusal),
                    epoch: None,
                    revisions: Some(BTreeMap::new()),
                };
                writer
                    .write_all(&budget::encode_response(response)?)
                    .await?;
                return Ok(());
            }
        }
    } else {
        None
    };
    let (reply, receive) = oneshot::channel();
    sender
        .send(Call {
            request,
            reply: ReplySender(reply),
        })
        .await?;
    let response = tokio::time::timeout(Duration::from_secs(10), receive)
        .await
        .context("El actor no respondió dentro del plazo")??;
    let response = budget::encode_reply(response)?;
    writer.write_all(&response).await?;
    if topic_subscription {
        let _subscriber_permit = subscriber_permit;
        stream_topics(&mut reader, &mut writer, selected).await?;
    }
    Ok(())
}

async fn stream_topics(
    reader: &mut BufReader<OwnedReadHalf>,
    writer: &mut tokio::net::unix::OwnedWriteHalf,
    mut selected: BTreeMap<TopicId, watch::Receiver<TopicFrame>>,
) -> Result<()> {
    let (notifier, mut notified) = mpsc::channel(TopicId::ALL.len());
    let mut watchers = tokio::task::JoinSet::new();
    let mut last_sent = BTreeMap::new();
    for (id, receiver) in &mut selected {
        let frame = receiver.borrow_and_update().clone();
        writer
            .write_all(&budget::encode_topic_frame(&frame)?)
            .await?;
        last_sent.insert(*id, frame.revision);
        let mut receiver = receiver.clone();
        let notifier = notifier.clone();
        let id = *id;
        watchers.spawn(async move {
            while receiver.changed().await.is_ok() {
                if notifier.send(id).await.is_err() {
                    break;
                }
            }
        });
    }
    drop(notifier);
    let result = loop {
        tokio::select! {
            input = reader.fill_buf() => {
                if input?.is_empty() { break Ok(()); }
                break Err(anyhow::anyhow!("El suscriptor envió datos después de suscribirse"));
            }
            Some(id) = notified.recv() => {
                if let Some(receiver) = selected.get_mut(&id) {
                    let frame = receiver.borrow_and_update().clone();
                    if last_sent.get(&id) != Some(&frame.revision) {
                        writer.write_all(&budget::encode_topic_frame(&frame)?).await?;
                        last_sent.insert(id, frame.revision);
                    }
                }
            }
        }
    };
    watchers.abort_all();
    result
}

fn connection_refusal(count: usize) -> Refusal {
    Refusal::TooMany {
        resource: Resource::IpcRequest,
        count: count as u64,
        max: MAX_CONNECTIONS as u64,
    }
}

async fn reject_busy(stream: UnixStream) -> Result<()> {
    let (reader, mut writer) = stream.into_split();
    if let Some(request) = receive_request(&mut BufReader::new(reader), &mut writer).await? {
        writer
            .write_all(&budget::encode_response(refused_response(
                request.id,
                connection_refusal(MAX_CONNECTIONS + 1),
            ))?)
            .await?;
    }
    Ok(())
}

// Keep an id already read even if a later field contains invalid JSON or the
// frame exceeds its bound. No identity is invented from unparsed text.
fn request_identity(bytes: &[u8]) -> String {
    struct Identity<'a>(&'a mut String);
    impl<'de> serde::de::Visitor<'de> for Identity<'_> {
        type Value = ();
        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("un objeto IPC")
        }
        fn visit_map<M: serde::de::MapAccess<'de>>(
            self,
            mut map: M,
        ) -> std::result::Result<(), M::Error> {
            while let Some(key) = map.next_key::<String>()? {
                if key == "id" {
                    *self.0 = map.next_value::<String>()?;
                } else {
                    map.next_value::<serde::de::IgnoredAny>()?;
                }
            }
            Ok(())
        }
    }
    let mut id = String::new();
    let _ = serde::de::Deserializer::deserialize_map(
        &mut serde_json::Deserializer::from_slice(bytes),
        Identity(&mut id),
    );
    id
}

fn decode_request(bytes: &[u8]) -> std::result::Result<Request, Refusal> {
    let invalid = |error: serde_json::Error, syntax| Refusal::InvalidParameter {
        parameter: orbit_core::refusal::Parameter::Request,
        expected: orbit_core::refusal::Expected::IpcPayload {
            bytes: bytes.len() as u64,
            line: error.line() as u64,
            column: error.column() as u64,
            syntax,
            error: error.to_string(),
        },
    };
    let value: serde_json::Value =
        serde_json::from_slice(bytes).map_err(|error| invalid(error, true))?;
    if let Some(version) = value.get("v").and_then(serde_json::Value::as_u64)
        && version != u64::from(PROTOCOL_VERSION)
    {
        return Err(Refusal::ProtocolVersion {
            received: version,
            required: u64::from(PROTOCOL_VERSION),
        });
    }
    let request: Request = serde_json::from_slice(bytes).map_err(|error| invalid(error, false))?;
    if request.id.len() > 128 {
        return Err(Refusal::InvalidParameter {
            parameter: orbit_core::refusal::Parameter::Identifier,
            expected: orbit_core::refusal::Expected::TextLength {
                bytes: request.id.len() as u64,
                max_bytes: 128,
            },
        });
    }
    if let Err(orbit_core::protocol::WireRefusal::Budget { bytes, max_bytes }) =
        request.body.wire_gate(bytes.len())
    {
        return Err(Refusal::TooLarge {
            resource: Resource::IpcRequest,
            bytes: bytes as u64,
            max_bytes: max_bytes as u64,
        });
    }
    Ok(request)
}

async fn receive_request(
    reader: &mut BufReader<OwnedReadHalf>,
    writer: &mut tokio::net::unix::OwnedWriteHalf,
) -> Result<Option<Request>> {
    let mut bytes = Vec::new();
    let read = tokio::time::timeout(
        Duration::from_secs(10),
        read_frame_into(reader, MAX_IPC_LINE_BYTES, Resource::IpcRequest, &mut bytes),
    )
    .await
    .context("La orden IPC no terminó dentro del plazo")?;
    let request = match read {
        Ok(()) => decode_request(&bytes),
        Err(error) => match error.downcast_ref::<Refusal>() {
            Some(refusal) => Err(refusal.clone()),
            None => return Err(error),
        },
    };
    match request {
        Ok(request) => Ok(Some(request)),
        Err(refusal) => {
            writer
                .write_all(&budget::encode_response(refused_response(
                    request_identity(&bytes),
                    refusal,
                ))?)
                .await?;
            Ok(None)
        }
    }
}

pub async fn read_frame<R: tokio::io::AsyncBufRead + Unpin>(reader: R) -> Result<Vec<u8>> {
    read_frame_with_limit(reader, orbit_core::limits::MAX_COMMAND_RESPONSE_BYTES).await
}

pub async fn read_frame_with_limit<R: tokio::io::AsyncBufRead + Unpin>(
    reader: R,
    limit: usize,
) -> Result<Vec<u8>> {
    let mut frame = Vec::new();
    read_frame_into(reader, limit, Resource::IpcResponse, &mut frame).await?;
    Ok(frame)
}

async fn read_frame_into<R: tokio::io::AsyncBufRead + Unpin>(
    mut reader: R,
    limit: usize,
    resource: Resource,
    frame: &mut Vec<u8>,
) -> Result<()> {
    loop {
        let available = reader.fill_buf().await?;
        if available.is_empty() {
            bail!("Conexión cerrada antes del fin del mensaje");
        }
        let end = available.iter().position(|b| *b == b'\n');
        let count = end.map_or(available.len(), |i| i + 1);
        let observed = frame.len() + count;
        if observed > limit {
            frame.extend_from_slice(&available[..limit.saturating_sub(frame.len())]);
            return Err(Refusal::TooLarge {
                resource,
                bytes: observed as u64,
                max_bytes: limit as u64,
            }
            .into());
        }
        frame.extend_from_slice(&available[..count]);
        reader.consume(count);
        if end.is_some() {
            return Ok(());
        }
    }
}

fn refused_response(id: String, refusal: Refusal) -> Response {
    Response {
        v: PROTOCOL_VERSION,
        id,
        ok: false,
        status: None,
        error: Some(refusal.to_string()),
        refusal: Some(refusal),
        epoch: None,
        revisions: Some(BTreeMap::new()),
    }
}

fn encode_request(request: &Request) -> Result<Vec<u8>> {
    let mut frame = serde_json::to_vec(request)?;
    frame.push(b'\n');
    if let Err(orbit_core::protocol::WireRefusal::Budget { bytes, max_bytes }) =
        request.body.wire_gate(frame.len())
    {
        return Err(Refusal::TooLarge {
            resource: Resource::IpcRequest,
            bytes: bytes as u64,
            max_bytes: max_bytes as u64,
        }
        .into());
    }
    Ok(frame)
}
