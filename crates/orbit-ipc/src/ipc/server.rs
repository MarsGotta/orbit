//! The daemon's listener: bounded connections and topic subscribers, one
//! request per connection, and a refusal with its numbers for what is over.
use super::*;
/// Four independent topic slots let the same four desktop clients hold one
/// topic stream each. The bound protects the registered
/// per-topic watch receivers per client; it is not a topic or command limit.
/// A fifth topic subscriber is refused with its 5/4 count.
pub(super) const MAX_TOPIC_SUBSCRIPTIONS: usize = 4;

// The existing 64 slots bound active IPC handlers. Four extra readers let one
// rejected connection per desktop client recover its id, using at most 4 MiB
// of frame storage. Further peers get an immediate refusal without reading an
// id; they cannot create tasks or allocate another request frame.
pub(super) const MAX_CONNECTIONS: usize = 64;
pub(super) const MAX_BUSY_READERS: usize = 4;

pub async fn listen(
    paths: &Paths,
    sender: mpsc::Sender<Call>,
    topic_channels: TopicChannels,
    ready: Option<oneshot::Sender<()>>,
) -> Result<()> {
    paths.initialize()?;
    if paths.socket.exists() {
        if UnixStream::connect(&paths.socket).await.is_ok() {
            bail!("An Orbit service is already active");
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
        // A peer that connected and closed without a byte (a liveness probe,
        // `listen`'s own check for a running instance) is gone before its
        // accept. Its slot used to be held until its handler read EOF: with
        // the host loaded, a burst of such probes filled the 64 slots ahead
        // of their handlers and the live order behind them was refused.
        if closed_without_a_byte(&stream) {
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
pub(super) async fn handle(stream: UnixStream, sender: mpsc::Sender<Call>) -> Result<()> {
    handle_connection(
        stream,
        sender,
        TopicChannels::new(),
        Arc::new(Semaphore::new(MAX_TOPIC_SUBSCRIPTIONS)),
    )
    .await
}

pub(super) async fn handle_connection(
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
        .context("The actor did not respond within the deadline")??;
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
                break Err(anyhow::anyhow!("The subscriber sent data after subscribing"));
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

/// The peer has already closed and sent nothing: a peek reads end-of-file.
/// It consumes nothing; a live peer that has not written yet reads
/// `WouldBlock` and keeps its connection.
fn closed_without_a_byte(stream: &UnixStream) -> bool {
    let mut byte = [0_u8; 1];
    matches!(
        rustix::net::recv(
            stream,
            &mut byte,
            rustix::net::RecvFlags::PEEK | rustix::net::RecvFlags::DONTWAIT,
        ),
        Ok((0, _))
    )
}

fn connection_refusal(count: usize) -> Refusal {
    Refusal::TooMany {
        resource: Resource::IpcRequest,
        count: count as u64,
        max: MAX_CONNECTIONS as u64,
    }
}

pub(super) async fn reject_busy(stream: UnixStream) -> Result<()> {
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
    .context("The IPC command did not finish within the deadline")?;
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
pub(super) fn refused_response(id: String, refusal: Refusal) -> Response {
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
