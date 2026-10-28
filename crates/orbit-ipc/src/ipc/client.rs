//! A process that reaches the daemon over its socket: one request per
//! connection, its answer checked to be this request's, and topic streams.
use super::*;

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

pub async fn request(paths: &Paths, command: Command) -> Result<Response> {
    tokio::time::timeout(Duration::from_secs(10), request_inner(paths, command))
        .await
        .context("The daemon did not respond within the deadline; the command is not repeated")?
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
        check_answer(
            "IPC response",
            &request,
            response.v,
            &response.id,
            response.refusal.as_ref(),
        )?;
        if !response.ok {
            if let Some(refusal) = response.refusal {
                return Err(refusal.into());
            }
            bail!(
                "{}",
                response.error.as_deref().unwrap_or("Operation refused")
            );
        }
        Ok(response)
    })
    .await
    .context("The daemon did not respond within the deadline; the command is not repeated")?
}

pub async fn voice_note_audio(paths: &Paths, message_id: String) -> Result<VoiceNoteAudio> {
    tokio::time::timeout(
        Duration::from_secs(10),
        voice_note_audio_inner(paths, message_id),
    )
    .await
    .context("The daemon did not respond within the deadline; the read is not repeated")?
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
    .context("The daemon did not open the topic subscription within the deadline")?
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
    .context("The daemon did not complete the read within the deadline")?
}

fn decode_topic_response(request: &Request, bytes: &[u8]) -> Result<TopicResponse> {
    let response: TopicResponse =
        serde_json::from_slice(bytes).context("Invalid topic IPC response")?;
    check_answer(
        "IPC response de temas",
        request,
        response.v,
        &response.id,
        response.refusal.as_ref(),
    )?;
    if !response.ok {
        if let Some(refusal) = response.refusal {
            return Err(refusal.into());
        }
        bail!("The daemon refused the topic command without a typed cause");
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
    check_answer(
        "IPC response",
        &request,
        response.v,
        &response.id,
        response.refusal.as_ref(),
    )?;
    if !response.ok {
        if let Some(refusal) = response.refusal {
            return Err(refusal.into());
        }
        bail!(
            "{}",
            response.error.as_deref().unwrap_or("Operation refused")
        );
    }
    response.audio.context("Invalid voice note response")
}

async fn connect(paths: &Paths) -> Result<UnixStream> {
    let stream = UnixStream::connect(&paths.socket)
        .await
        .context("The Orbit service is not active. Open Orbit.app")?;
    if stream.peer_cred()?.uid() != rustix::process::geteuid().as_raw() {
        bail!("The IPC service belongs to another user; the command was not sent");
    }
    Ok(stream)
}

pub(super) fn validate_response(request: &Request, response: &Response) -> Result<()> {
    check_answer(
        "IPC response",
        request,
        response.v,
        &response.id,
        response.refusal.as_ref(),
    )?;
    if !response.ok {
        if let Some(refusal) = response.refusal.clone() {
            return Err(refusal.into());
        }
        bail!(
            "{}",
            response.error.as_deref().unwrap_or("Operation refused")
        );
    }
    Ok(())
}

/// Whether a frame read back is this request's answer. The daemon writes an
/// empty id only when it refused the connection before it could read one
/// (every slot taken, or a frame without a readable id) and it invents none:
/// that refusal is this connection's answer and reaches the caller with its
/// numbers. It can only ever be an error, never another order's success.
/// Any other mismatch names both versions and both ids, never the body.
fn check_answer(
    what: &str,
    request: &Request,
    v: u8,
    id: &str,
    refusal: Option<&Refusal>,
) -> Result<()> {
    if v == PROTOCOL_VERSION && id == request.id {
        return Ok(());
    }
    if id.is_empty()
        && let Some(refusal) = refusal
    {
        return Err(refusal.clone().into());
    }
    // Request ids are at most 128 bytes (the daemon refuses longer ones). The
    // received id is cut to 128 characters so a malformed answer cannot grow
    // this message to the size of its frame.
    let received: String = id.chars().take(128).collect();
    bail!(
        "Invalid {what}: version {v} (expected {PROTOCOL_VERSION}), identifier «{received}» (expected «{}»)",
        request.id
    );
}

pub(super) fn encode_request(request: &Request) -> Result<Vec<u8>> {
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
