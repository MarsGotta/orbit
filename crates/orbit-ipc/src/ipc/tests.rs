//! Pure response budgets plus owner-local UnixStream fixtures. No daemon state,
//! durable original, provider or OS service is modified by this projection.
#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::client::{encode_request, validate_response};
use super::server::{
    MAX_BUSY_READERS, MAX_CONNECTIONS, MAX_TOPIC_SUBSCRIPTIONS, handle, handle_connection,
    refused_response, reject_busy,
};
use super::*;

use orbit_core::{
    MAX_FRAME_BYTES,
    refusal::{Operation, Refusal, Resource},
    topics::{TopicData, TopicFrame, TopicId},
};

#[tokio::test]
async fn read_admission_refusal_keeps_its_numbers() {
    let directory = tempfile::tempdir().unwrap();
    let paths = Paths::resolve(Some(directory.path().to_path_buf())).unwrap();
    paths.initialize().unwrap();
    let listener = UnixListener::bind(&paths.socket).unwrap();
    let handler = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let (reader, mut writer) = stream.into_split();
        let request: Request =
            serde_json::from_slice(&read_frame(BufReader::new(reader)).await.unwrap()).unwrap();
        let refusal = Refusal::Busy {
            operation: Operation::IpcRequest,
            retry_after_ms: Some(125),
        };
        writer
            .write_all(
                &budget::encode_response(Response {
                    v: PROTOCOL_VERSION,
                    id: request.id,
                    ok: false,
                    status: None,
                    error: Some(refusal.to_string()),
                    refusal: Some(refusal),
                    epoch: None,
                    revisions: Some(BTreeMap::new()),
                })
                .unwrap(),
            )
            .await
            .unwrap();
    });
    let error = read_topic(
        &paths,
        orbit_core::reads::ReadRequest::NotesPage { cursor: None },
    )
    .await
    .unwrap_err();
    assert!(matches!(
        error.downcast_ref::<Refusal>(),
        Some(Refusal::Busy {
            operation: Operation::IpcRequest,
            retry_after_ms: Some(125)
        })
    ));
    handler.await.unwrap();
}

#[tokio::test]
async fn topic_admission_busy_keeps_retry_after() {
    let directory = tempfile::tempdir().unwrap();
    let paths = Paths::resolve(Some(directory.path().to_path_buf())).unwrap();
    paths.initialize().unwrap();
    let listener = UnixListener::bind(&paths.socket).unwrap();
    let handler = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let (reader, mut writer) = stream.into_split();
        let request: Request =
            serde_json::from_slice(&read_frame(BufReader::new(reader)).await.unwrap()).unwrap();
        let refusal = Refusal::Busy {
            operation: Operation::IpcRequest,
            retry_after_ms: Some(125),
        };
        writer
            .write_all(
                &budget::encode_response(Response {
                    v: PROTOCOL_VERSION,
                    id: request.id,
                    ok: false,
                    status: None,
                    error: Some(refusal.to_string()),
                    refusal: Some(refusal),
                    epoch: None,
                    revisions: Some(BTreeMap::new()),
                })
                .unwrap(),
            )
            .await
            .unwrap();
    });
    let error = match subscribe_topics(&paths, vec![TopicId::Status]).await {
        Ok(_) => panic!("busy topic admission must be refused"),
        Err(error) => error,
    };
    assert!(matches!(
        error.downcast_ref::<Refusal>(),
        Some(Refusal::Busy {
            operation: Operation::IpcRequest,
            retry_after_ms: Some(125),
        })
    ));
    handler.await.unwrap();
}

#[tokio::test]
async fn four_topic_clients_are_admitted_and_the_fifth_is_numbered() {
    let directory = tempfile::tempdir().unwrap();
    let paths = Paths::resolve(Some(directory.path().to_path_buf())).unwrap();
    let (calls, mut incoming) = mpsc::channel(16);
    let (topic_sender, _receiver) = watch::channel(TopicFrame {
        epoch: "synthetic".into(),
        revision: 1,
        data: TopicData::not_observed(TopicId::Status),
    });
    let channels = [(TopicId::Status, topic_sender)].into();
    let listener_paths = paths.clone();
    let listener = tokio::spawn(async move {
        let _ = listen(&listener_paths, calls, channels, None).await;
    });
    tokio::time::timeout(Duration::from_secs(5), async {
        while UnixStream::connect(&paths.socket).await.is_err() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let actor = tokio::spawn(async move {
        while let Some(call) = incoming.recv().await {
            let request = call.request;
            match request.body {
                Command::SubscribeTopics { .. } => {
                    let _ = call.reply.send(TopicResponse {
                        v: PROTOCOL_VERSION,
                        id: request.id,
                        ok: true,
                        revisions: [(TopicId::Status, 1)].into(),
                        read: None,
                        answer: None,
                        refusal: None,
                    });
                }
                _ => panic!("unexpected synthetic command"),
            }
        }
    });
    let mut topics = Vec::new();
    for _ in 0..4 {
        topics.push(
            subscribe_topics(&paths, vec![TopicId::Status])
                .await
                .unwrap()
                .1,
        );
    }
    assert_eq!(topics.len(), 4);
    let fifth = match subscribe_topics(&paths, vec![TopicId::Status]).await {
        Ok(_) => panic!("fifth topic stream must be refused"),
        Err(error) => error,
    };
    assert!(matches!(
        fifth.downcast_ref::<Refusal>(),
        Some(Refusal::TooMany {
            resource: Resource::Subscriptions,
            count: 5,
            max: 4,
        })
    ));
    drop(topics);
    actor.abort();
    listener.abort();
}

#[tokio::test]
async fn topic_subscription_sends_an_initial_frame_and_later_revision() {
    let (mut client, server) = UnixStream::pair().unwrap();
    let (sender, mut actor) = mpsc::channel(1);
    let initial = TopicFrame {
        epoch: "synthetic-epoch".into(),
        revision: 1,
        data: TopicData::not_observed(TopicId::Status),
    };
    let (status_sender, receiver) = watch::channel(initial.clone());
    drop(receiver);
    let channels = [(TopicId::Status, status_sender.clone())].into();
    let handler = tokio::spawn(handle_connection(
        server,
        sender,
        channels,
        Arc::new(Semaphore::new(MAX_TOPIC_SUBSCRIPTIONS)),
    ));
    let request = Request {
        v: PROTOCOL_VERSION,
        id: id(204),
        op: None,
        body: Command::SubscribeTopics {
            topics: vec![TopicId::Status],
        },
    };
    let mut wire = serde_json::to_vec(&request).unwrap();
    wire.push(b'\n');
    client.write_all(&wire).await.unwrap();
    let call = actor.recv().await.unwrap();
    assert!(matches!(call.request.body, Command::SubscribeTopics { .. }));
    call.reply
        .send(TopicResponse {
            v: PROTOCOL_VERSION,
            id: call.request.id,
            ok: true,
            revisions: [(TopicId::Status, 1)].into(),
            read: None,
            answer: None,
            refusal: None,
        })
        .unwrap();
    let mut reader = BufReader::new(client);
    let handshake: TopicResponse =
        serde_json::from_slice(&read_frame(&mut reader).await.unwrap()).unwrap();
    assert_eq!(handshake.revisions[&TopicId::Status], 1);
    let first: orbit_core::protocol::TopicEvent =
        serde_json::from_slice(&read_frame(&mut reader).await.unwrap()).unwrap();
    assert!(
        matches!(first, orbit_core::protocol::TopicEvent::Changed { frame } if frame == initial)
    );
    status_sender.send_replace(TopicFrame {
        revision: 2,
        ..initial
    });
    let next: orbit_core::protocol::TopicEvent =
        serde_json::from_slice(&read_frame(&mut reader).await.unwrap()).unwrap();
    assert!(
        matches!(next, orbit_core::protocol::TopicEvent::Changed { frame } if frame.revision == 2)
    );
    drop(reader);
    handler.await.unwrap().unwrap();
}

#[test]
fn oversized_read_page_becomes_a_numbered_absence() {
    let response = TopicResponse {
        v: PROTOCOL_VERSION,
        id: id(205),
        ok: true,
        revisions: [(TopicId::Notes, 3)].into(),
        read: Some(orbit_core::reads::ReadId::NotesPage),
        answer: Some(orbit_core::topics::Knowledge::Present {
            value: orbit_core::topics::Page {
                items: vec![json!({"text": "x".repeat(650 * 1024)})],
                next: None,
                total: 1,
                revision: 3,
                store_version: None,
            },
            as_of_ms: None,
        }),
        refusal: None,
    };
    let bytes = budget::encode_reply(Reply::Topics(response)).unwrap();
    let decoded: TopicResponse = serde_json::from_slice(&bytes).unwrap();
    let Some(orbit_core::topics::Knowledge::Absent {
        refusal:
            Refusal::TooLarge {
                resource: Resource::IpcResponse,
                bytes: actual,
                max_bytes,
            },
    }) = decoded.answer
    else {
        panic!("page must be absent with a numbered refusal")
    };
    assert!(actual > max_bytes);
    // A page of the hub's list and this Mac's own recordings is
    // bounded at 640 KiB plus the envelope.
    assert_eq!(max_bytes, 644 * 1024);
}
use serde_json::json;

#[tokio::test]
async fn topic_wire_command_reaches_the_actor() {
    let (mut client, server) = UnixStream::pair().unwrap();
    let (sender, mut actor) = mpsc::channel(1);
    let handler = tokio::spawn(handle(server, sender));
    let request = json!({
        "v": PROTOCOL_VERSION,
        "id": "admission",
        "body": {"command": "subscribe_topics", "topics": ["status"]}
    });
    let mut wire = serde_json::to_vec(&request).unwrap();
    wire.push(b'\n');
    client.write_all(&wire).await.unwrap();
    let call = actor
        .recv()
        .await
        .expect("topic command must reach the actor");
    call.reply.send(response()).unwrap();
    let _ = read_frame(BufReader::new(&mut client)).await.unwrap();
    drop(client);
    handler.await.unwrap().unwrap();
}

fn id(number: u32) -> String {
    format!("00000000-0000-4000-8000-{number:012x}")
}

fn response() -> Response {
    Response {
        refusal: None,
        v: PROTOCOL_VERSION,
        id: "request-id".into(),
        ok: true,
        status: None,
        error: None,
        epoch: None,
        revisions: Some(BTreeMap::new()),
    }
}
fn decode(bytes: &[u8]) -> Response {
    assert!(bytes.len() <= MAX_FRAME_BYTES);
    assert_eq!(bytes.last(), Some(&b'\n'));
    serde_json::from_slice(bytes).unwrap()
}

fn assert_oversize(response: &Response) {
    let Some(Refusal::TooLarge {
        resource,
        bytes,
        max_bytes,
    }) = &response.refusal
    else {
        panic!("expected a typed size refusal: {:?}", response.refusal);
    };
    assert_eq!(*resource, Resource::IpcResponse);
    assert!(*bytes > *max_bytes);
    assert_eq!(*max_bytes, MAX_FRAME_BYTES as u64);
    assert_eq!(
        response.refusal.as_ref().unwrap().retry(),
        orbit_core::refusal::Retry::CheckOutcome
    );
    assert_eq!(
        response.error.as_deref(),
        response
            .refusal
            .as_ref()
            .map(ToString::to_string)
            .as_deref()
    );
}

#[tokio::test]
async fn public_request_preserves_refusal_through_unix_ipc() {
    let directory = tempfile::tempdir().unwrap();
    let paths = Paths::resolve(Some(directory.path().to_owned())).unwrap();
    paths.initialize().unwrap();
    let listener = UnixListener::bind(&paths.socket).unwrap();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let request: Request =
            serde_json::from_slice(&read_frame(BufReader::new(&mut stream)).await.unwrap())
                .unwrap();
        let refusal = Refusal::TooLarge {
            resource: Resource::IpcResponse,
            bytes: 101,
            max_bytes: 100,
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
        stream
            .write_all(&budget::encode_response(response).unwrap())
            .await
            .unwrap();
    });
    let error = request(&paths, Command::Status).await.unwrap_err();
    assert_eq!(
        error.downcast_ref::<Refusal>(),
        Some(&Refusal::TooLarge {
            resource: Resource::IpcResponse,
            bytes: 101,
            max_bytes: 100,
        })
    );
    server.await.unwrap();
}

#[tokio::test]
async fn capacity_and_busy_are_typed_on_the_wire() {
    let (mut client, server) = UnixStream::pair().unwrap();
    let (sender, mut actor) = mpsc::channel(1);
    let handler = tokio::spawn(handle_connection(
        server,
        sender,
        TopicChannels::new(),
        Arc::new(Semaphore::new(0)),
    ));
    client
        .write_all(&topic_subscription_frame(42))
        .await
        .unwrap();
    let response = decode(&read_frame(BufReader::new(client)).await.unwrap());
    assert_eq!(
        response.refusal,
        Some(Refusal::TooMany {
            resource: Resource::Subscriptions,
            count: 5,
            max: 4
        })
    );
    assert!(actor.try_recv().is_err());
    handler.await.unwrap().unwrap();

    let (mut client, server) = UnixStream::pair().unwrap();
    let busy = tokio::spawn(reject_busy(server));
    client
        .write_all(&topic_subscription_frame(43))
        .await
        .unwrap();
    let response = decode(&read_frame(BufReader::new(client)).await.unwrap());
    assert_eq!(
        response.refusal,
        Some(Refusal::TooMany {
            resource: Resource::IpcRequest,
            count: 65,
            max: 64
        })
    );
    busy.await.unwrap().unwrap();
}

#[test]
fn fitting_response_is_byte_identical_with_unicode_and_control_escaping() {
    let mut message = response();
    message.error = Some("α e\u{301} 🛰 \" \\ \n \u{0001} \u{0085} \u{202e}".into());
    let mut expected = serde_json::to_vec(&message).unwrap();
    expected.push(b'\n');
    assert_eq!(budget::encode_response(message).unwrap(), expected);
}

#[test]
fn exact_boundary_includes_newline_and_one_more_byte_returns_small_error() {
    let mut message = response();
    message.error = Some(String::new());
    let empty_size = serde_json::to_vec(&message).unwrap().len() + 1;
    message.error = Some("x".repeat(MAX_FRAME_BYTES - empty_size));
    assert_eq!(
        budget::encode_response(message).unwrap().len(),
        MAX_FRAME_BYTES
    );
    let mut bigger = response();
    bigger.error = Some("x".repeat(MAX_FRAME_BYTES - empty_size + 1));
    let bytes = budget::encode_response(bigger).unwrap();
    assert!(bytes.len() < 1024);
    assert!(!decode(&bytes).ok);
}

#[test]
fn oversized_error_keeps_exact_escaped_correlation() {
    let message = Response {
        refusal: None,
        v: PROTOCOL_VERSION,
        id: "\u{0001}".repeat(128),
        ok: false,
        status: None,
        error: Some("error".repeat(MAX_FRAME_BYTES)),
        epoch: None,
        revisions: Some(BTreeMap::new()),
    };
    let received = decode(&budget::encode_response(message).unwrap());
    assert_eq!(received.id, "\u{0001}".repeat(128));
    assert_oversize(&received);
}

async fn refused_request(body: serde_json::Value, identity: &str) -> serde_json::Value {
    let (mut client, server) = UnixStream::pair().unwrap();
    let (sender, mut actor) = mpsc::channel(1);
    let handler = tokio::spawn(handle(server, sender));
    let mut bytes =
        serde_json::to_vec(&json!({"v": PROTOCOL_VERSION, "id": identity, "body": body})).unwrap();
    bytes.push(b'\n');
    client.write_all(&bytes).await.unwrap();
    let mut reader = BufReader::new(client);
    let result = tokio::select! {
        result = read_frame(&mut reader) => result,
        call = actor.recv() => {
            assert!(call.is_none(), "invalid request reached the actor");
            read_frame(&mut reader).await
        }
    };
    assert!(
        actor.try_recv().is_err(),
        "invalid request reached the actor"
    );
    let _ = handler.await.unwrap();
    serde_json::from_slice(&result.expect("request must receive its numbered refusal")).unwrap()
}

#[tokio::test]
async fn oversized_identity_receives_its_bytes_and_ceiling() {
    let reply = refused_request(json!({"command":"status"}), &"x".repeat(129)).await;
    assert_eq!(reply["id"], "x".repeat(129));
    assert_eq!(reply["refusal"]["code"], "invalid_parameter");
    assert_eq!(reply["refusal"]["expected"]["bytes"], 129);
    assert_eq!(reply["refusal"]["expected"]["max_bytes"], 128);
}

#[tokio::test]
async fn frame_reader_refuses_an_oversized_request_with_observed_bytes_and_ceiling() {
    let body =
        json!({"command":"chat", "text":"x".repeat(orbit_core::limits::MAX_DEFAULT_COMMAND_BYTES)});
    let bytes = serde_json::to_vec(&json!({"v": PROTOCOL_VERSION,"id":"budget","body":body}))
        .unwrap()
        .len()
        + 1;
    let reply = refused_request(body, "budget").await;
    assert_eq!(reply["refusal"]["code"], "too_large");
    let observed = reply["refusal"]["bytes"].as_u64().unwrap() as usize;
    assert!(observed > MAX_IPC_LINE_BYTES && observed <= bytes);
    assert_eq!(reply["refusal"]["resource"], "ipc_request");
    assert_eq!(reply["refusal"]["max_bytes"], MAX_IPC_LINE_BYTES);
}

#[tokio::test]
async fn oversized_frame_error_carries_observed_bytes_and_ceiling() {
    let bytes = b"12345\n";
    let error = read_frame_with_limit(&bytes[..], 5).await.unwrap_err();
    assert!(error.to_string().contains('6'), "{error}");
    assert!(error.to_string().contains('5'), "{error}");
}

#[test]
fn oversized_topic_envelope_names_its_bytes_without_calling_it_voice_audio() {
    let response = TopicResponse {
        v: PROTOCOL_VERSION,
        id: "x".repeat(orbit_core::topics::transport_max_bytes()),
        ok: true,
        revisions: BTreeMap::new(),
        read: None,
        answer: None,
        refusal: None,
    };
    let bytes = serde_json::to_vec(&response).unwrap().len() + 1;
    let error = budget::encode_reply(Reply::Topics(response)).unwrap_err();
    let text = error.to_string();
    assert!(
        matches!(error.downcast_ref::<Refusal>(), Some(Refusal::TooLarge { bytes: observed, .. }) if *observed >= bytes as u64),
        "{text}"
    );
    assert!(
        text.contains(&orbit_core::topics::transport_max_bytes().to_string()),
        "{text}"
    );
    assert!(!text.contains("nota de voz"), "{text}");
}

fn topic_subscription_frame(number: u32) -> Vec<u8> {
    let mut frame = serde_json::to_vec(&Request {
        v: PROTOCOL_VERSION,
        id: id(number),
        op: None,
        body: Command::SubscribeTopics {
            topics: vec![TopicId::Status],
        },
    })
    .unwrap();
    frame.push(b'\n');
    frame
}

#[tokio::test]
async fn an_identity_near_the_frame_ceiling_still_receives_its_numbered_refusal() {
    let reply = refused_request(
        json!({"command":"status"}),
        &"x".repeat(MAX_IPC_LINE_BYTES - 128),
    )
    .await;
    assert_eq!(reply["refusal"]["code"], "invalid_parameter");
    assert_eq!(
        reply["refusal"]["expected"]["bytes"],
        MAX_IPC_LINE_BYTES - 128
    );
    assert_eq!(reply["refusal"]["expected"]["max_bytes"], 128);
}

#[test]
fn the_client_refuses_an_oversized_command_before_sending_it_with_its_numbers() {
    let request = Request {
        v: PROTOCOL_VERSION,
        id: "local".into(),
        op: None,
        body: Command::Chat {
            attachments: Vec::new(),
            text: "x".repeat(MAX_IPC_LINE_BYTES),
            primary: false,
        },
    };
    let bytes = serde_json::to_vec(&request).unwrap().len() + 1;
    let error = encode_request(&request).unwrap_err();
    assert_eq!(
        error.downcast_ref::<Refusal>(),
        Some(&Refusal::TooLarge {
            resource: Resource::IpcRequest,
            bytes: bytes as u64,
            max_bytes: orbit_core::limits::MAX_DEFAULT_COMMAND_BYTES as u64,
        })
    );
}

#[tokio::test]
async fn topic_subscriber_eof_releases_the_slot_without_a_new_topic_change() {
    let directory = tempfile::tempdir().unwrap();
    let paths = Paths::resolve(Some(directory.path().to_path_buf())).unwrap();
    let (calls, mut incoming) = mpsc::channel(16);
    let (ready, bound) = oneshot::channel();
    let listener_paths = paths.clone();
    let listener = tokio::spawn(async move {
        listen(&listener_paths, calls, TopicChannels::new(), Some(ready)).await
    });
    bound.await.unwrap();
    let actor = tokio::spawn(async move {
        while let Some(call) = incoming.recv().await {
            call.reply
                .send(TopicResponse {
                    v: PROTOCOL_VERSION,
                    id: call.request.id,
                    ok: true,
                    revisions: BTreeMap::new(),
                    read: None,
                    answer: None,
                    refusal: None,
                })
                .unwrap();
        }
    });
    let mut subscribers = Vec::new();
    for _ in 0..MAX_TOPIC_SUBSCRIPTIONS {
        subscribers.push(subscribe_topics(&paths, vec![]).await.unwrap().1);
    }
    drop(subscribers.pop());
    // The old EOF regression must remain covered at the surviving subscription boundary.
    let replacement = tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            match subscribe_topics(&paths, vec![]).await {
                Ok((_, stream)) => break stream,
                Err(error)
                    if matches!(
                        error.downcast_ref::<Refusal>(),
                        Some(Refusal::TooMany {
                            resource: Resource::Subscriptions,
                            ..
                        })
                    ) =>
                {
                    tokio::task::yield_now().await
                }
                Err(error) => panic!("replacement topic stream: {error}"),
            }
        }
    })
    .await
    .unwrap();
    drop(replacement);
    drop(subscribers);
    actor.abort();
    listener.abort();
}

#[tokio::test]
async fn topic_command_budget_receives_its_bytes_and_ceiling() {
    let body = json!({"command":"subscribe_topics", "topics": vec!["status"; transport_max_bytes() / 9 + 1]});
    let bytes =
        serde_json::to_vec(&json!({"v": PROTOCOL_VERSION, "id":"topic-budget", "body":body}))
            .unwrap()
            .len()
            + 1;
    assert!(
        bytes < MAX_IPC_LINE_BYTES,
        "this case must reach the command gate, not the frame reader"
    );
    let reply = refused_request(body, "topic-budget").await;
    assert_eq!(reply["id"], "topic-budget");
    assert_eq!(reply["refusal"]["code"], "too_large");
    assert_eq!(reply["refusal"]["bytes"], bytes);
    assert_eq!(reply["refusal"]["max_bytes"], transport_max_bytes());
}

async fn raw_refusal(bytes: &[u8], busy: bool) -> serde_json::Value {
    let (mut client, server) = UnixStream::pair().unwrap();
    let (sender, mut actor) = mpsc::channel(1);
    let handler = tokio::spawn(async move {
        if busy {
            reject_busy(server).await
        } else {
            handle(server, sender).await
        }
    });
    // Write concurrently: an oversized request can exceed the socket buffer.
    let (read, mut write) = client.split();
    let (response, _) = tokio::join!(read_frame(BufReader::new(read)), write.write_all(bytes));
    let _ = handler.await.unwrap();
    assert!(
        actor.try_recv().is_err(),
        "invalid request reached the actor"
    );
    serde_json::from_slice(&response.expect("socket must answer with its cause")).unwrap()
}

#[tokio::test]
async fn malformed_json_receives_its_identity_bytes_and_position() {
    let frame = b"{\"v\":1,\"id\":\"broken\",\"body\":!}\n";
    let response = raw_refusal(frame, false).await;
    assert_eq!(response["id"], "broken");
    let expected = &response["refusal"]["expected"];
    assert_eq!(response["refusal"]["code"], "invalid_parameter");
    assert_eq!(expected["kind"], "ipc_payload");
    assert_eq!(expected["syntax"], true);
    assert_eq!(expected["bytes"], frame.len());
    assert_eq!(expected["line"], 1);
    assert_eq!(expected["column"], 29);
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(response["error"].as_str().unwrap()).unwrap(),
        response["refusal"]
    );
}

#[tokio::test]
async fn unknown_and_retired_commands_receive_their_cause_and_size() {
    for command in ["unknown_order", "subscribe", "jobs"] {
        let frame =
            format!("{{\"v\":1,\"id\":\"unknown\",\"body\":{{\"command\":\"{command}\"}}}}\n");
        let response = raw_refusal(frame.as_bytes(), false).await;
        assert_eq!(response["id"], "unknown");
        let expected = &response["refusal"]["expected"];
        assert_eq!(expected["syntax"], false);
        assert_eq!(expected["bytes"], frame.len());
        assert!(expected["error"].as_str().unwrap().contains(command));
    }
}

#[tokio::test]
async fn version_is_refused_before_decoding_the_command() {
    let response = raw_refusal(b"{\"v\":2,\"id\":\"version\",\"body\":false}\n", false).await;
    assert_eq!(response["id"], "version");
    assert_eq!(
        response["refusal"],
        serde_json::json!({"code":"protocol_version", "received":2, "required":1, "retry":{"kind":"after_fix"}})
    );
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(response["error"].as_str().unwrap()).unwrap(),
        response["refusal"]
    );
}

#[tokio::test]
async fn busy_socket_refuses_long_identity_with_its_numbers() {
    let frame = format!(
        "{{\"v\":1,\"id\":\"{}\",\"body\":{{\"command\":\"status\"}}}}\n",
        "x".repeat(129)
    );
    let response = raw_refusal(frame.as_bytes(), true).await;
    assert_eq!(response["id"], "x".repeat(129));
    assert_eq!(response["refusal"]["expected"]["bytes"], 129);
    assert_eq!(response["refusal"]["expected"]["max_bytes"], 128);
}

#[tokio::test]
async fn busy_socket_refuses_large_frame_with_observed_bytes_and_ceiling() {
    let frame = format!(
        "{{\"id\":\"huge\",\"body\":\"{}\"}}\n",
        "x".repeat(MAX_IPC_LINE_BYTES)
    );
    let response = raw_refusal(frame.as_bytes(), true).await;
    assert_eq!(response["id"], "huge");
    assert_eq!(response["refusal"]["resource"], "ipc_request");
    assert!(response["refusal"]["bytes"].as_u64().unwrap() > MAX_IPC_LINE_BYTES as u64);
    assert_eq!(response["refusal"]["max_bytes"], MAX_IPC_LINE_BYTES);
}

#[tokio::test]
async fn response_reader_names_the_response_and_observed_size() {
    let error = read_frame_with_limit(&b"123456789\n"[..], 7)
        .await
        .unwrap_err();
    assert_eq!(
        error.downcast_ref::<Refusal>(),
        Some(&Refusal::TooLarge {
            resource: Resource::IpcResponse,
            bytes: 10,
            max_bytes: 7,
        })
    );
}

#[tokio::test]
async fn busy_readers_are_bounded_before_allocating_another_frame() {
    let directory = tempfile::tempdir().unwrap();
    let paths = Paths::resolve(Some(directory.path().to_path_buf())).unwrap();
    let (calls, _actor) = mpsc::channel(1);
    let (ready, bound) = oneshot::channel();
    let server_paths = paths.clone();
    let listener = tokio::spawn(async move {
        listen(&server_paths, calls, TopicChannels::new(), Some(ready)).await
    });
    bound.await.unwrap();
    let mut occupied = Vec::new();
    for _ in 0..MAX_CONNECTIONS + MAX_BUSY_READERS {
        let mut stream = UnixStream::connect(&paths.socket).await.unwrap();
        stream.write_all(b"{").await.unwrap();
        occupied.push(stream);
    }
    let mut extra = UnixStream::connect(&paths.socket).await.unwrap();
    extra
        .write_all(&topic_subscription_frame(99))
        .await
        .unwrap();
    let response = read_frame(BufReader::new(extra)).await.unwrap();
    let response = decode(&response);
    // All reader slots are held by incomplete frames: no id has been read on
    // this socket. An unbounded reject_busy spawn would return its actual id.
    assert_eq!(response.id, "");
    assert_eq!(
        response.refusal,
        Some(Refusal::TooMany {
            resource: Resource::IpcRequest,
            count: 69,
            max: 64,
        })
    );
    drop(occupied);
    listener.abort();
    let _ = listener.await;
}

#[tokio::test]
async fn a_malformed_request_without_a_readable_id_still_receives_its_cause() {
    let response = raw_refusal(b"!\n", false).await;
    assert_eq!(response["id"], "");
    assert_eq!(response["refusal"]["code"], "invalid_parameter");
    let expected = &response["refusal"]["expected"];
    assert_eq!(expected["syntax"], true);
    assert_eq!(expected["bytes"], 2);
    assert_eq!(expected["line"], 1);
    assert_eq!(expected["column"], 1);
}

#[tokio::test]
async fn local_command_and_reads_preserve_refusals_numbers_and_op() -> Result<()> {
    let (calls, mut actor) = mpsc::channel::<Call>(32);
    let (_ended, end) = watch::channel(None);
    let client = LocalClient::new(calls, &TopicChannels::new(), end);
    let op = new_op();
    let sent_op = op.clone();
    let refusal = Refusal::TooMany {
        resource: Resource::Subscriptions,
        count: 5,
        max: 4,
    };
    let actor_refusal = refusal.clone();
    let actor_task = tokio::spawn(async move {
        for index in 0..3 {
            let call = actor.recv().await.unwrap();
            if index == 0 {
                assert_eq!(call.request.op, Some(sent_op.clone()));
            }
            let response = refused_response(call.request.id, actor_refusal.clone());
            match call.request.body {
                Command::Status | Command::Read { .. } => {
                    call.reply.send(response).unwrap();
                }
                Command::VoiceNoteAudio { .. } => {
                    call.reply
                        .send(VoiceNoteAudioResponse {
                            v: PROTOCOL_VERSION,
                            id: response.id,
                            ok: false,
                            audio: None,
                            error: response.error,
                            refusal: response.refusal,
                        })
                        .unwrap();
                }
                other => panic!("unexpected local command: {other:?}"),
            }
        }
    });
    let command = client
        .request_command_as(Command::Status, op)
        .await
        .unwrap_err();
    let read = client
        .read_topic(ReadRequest::NotesPage { cursor: None })
        .await
        .unwrap_err();
    let audio = client.voice_note_audio("voice".into()).await.unwrap_err();
    for error in [command, read, audio] {
        assert_eq!(error.downcast_ref::<Refusal>(), Some(&refusal));
    }
    actor_task.await?;
    Ok(())
}

#[tokio::test]
async fn local_actor_end_retains_failure_and_a_lost_owner_is_not_a_clean_quit() {
    let (calls, actor) = mpsc::channel::<Call>(32);
    let (ended, end) = watch::channel(None);
    let client = LocalClient::new(calls, &TopicChannels::new(), end);
    drop(actor);
    ended.send_replace(Some(DaemonEnded::Failed(
        "disco: 7 escrituras pendientes".into(),
    )));
    let error = client.request_command(Command::Status).await.unwrap_err();
    assert!(error.to_string().contains("disco: 7 escrituras pendientes"));
    let (calls, actor) = mpsc::channel::<Call>(32);
    let (ended, end) = watch::channel(None);
    let client = LocalClient::new(calls, &TopicChannels::new(), end);
    drop(actor);
    drop(ended);
    assert_eq!(client.ended().await, DaemonEnded::Vanished);
    assert!(
        client
            .request_command(Command::Status)
            .await
            .unwrap_err()
            .to_string()
            .contains("pánico")
    );
}

/// Ajustes › Agentes: one call to the actor and the outcome it answers,
/// whole; an answer of another kind is not taken for it.
#[tokio::test]
async fn an_agent_call_returns_its_outcome_and_refuses_another_reply() -> Result<()> {
    use orbit_core::agent_config::{AgentCall, AgentOutcome, AgentRefusal, RefusalNumber};
    let (calls, mut actor) = mpsc::channel::<Call>(32);
    let (_ended, end) = watch::channel(None);
    let client = LocalClient::new(calls, &TopicChannels::new(), end);
    let refused = AgentOutcome::Refused {
        refusal: AgentRefusal {
            code: "too_many_skills".into(),
            status: 409,
            numbers: [
                ("count".to_owned(), RefusalNumber::Integer(64)),
                ("max".to_owned(), RefusalNumber::Integer(64)),
            ]
            .into_iter()
            .collect(),
        },
    };
    let answered = refused.clone();
    let actor_task = tokio::spawn(async move {
        let call = actor.recv().await.unwrap();
        assert_eq!(
            call.request.body,
            Command::Agent {
                call: AgentCall::Config {
                    agent_id: "vega".into()
                }
            }
        );
        call.reply
            .send(AgentResponse {
                v: PROTOCOL_VERSION,
                id: call.request.id,
                outcome: answered,
            })
            .unwrap();
        let call = actor.recv().await.unwrap();
        call.reply.send(response()).unwrap();
    });
    let config = AgentCall::Config {
        agent_id: "vega".into(),
    };
    assert_eq!(client.agent(config.clone()).await?, refused);
    assert!(client.agent(config).await.is_err());
    actor_task.await?;
    let encoded = budget::encode_reply(Reply::Agent(AgentResponse {
        v: PROTOCOL_VERSION,
        id: id(1),
        outcome: AgentOutcome::Done,
    }))?;
    let decoded: serde_json::Value = serde_json::from_slice(&encoded)?;
    assert_eq!(decoded["outcome"]["outcome"], "done");
    Ok(())
}

/// Answers every call with an `ok` status, the way the daemon's actor would.
fn answering_actor(mut calls: mpsc::Receiver<Call>) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        while let Some(call) = calls.recv().await {
            let mut answer = response();
            answer.id = call.request.id;
            let _ = call.reply.send(answer);
        }
    })
}

#[tokio::test]
async fn probes_closed_before_their_accept_hold_no_connection_slot() {
    let directory = tempfile::tempdir().unwrap();
    let paths = Paths::resolve(Some(directory.path().to_path_buf())).unwrap();
    let (calls, actor) = mpsc::channel(1);
    let actor = answering_actor(actor);
    let (ready, bound) = oneshot::channel();
    let server_paths = paths.clone();
    let listener = tokio::spawn(async move {
        listen(&server_paths, calls, TopicChannels::new(), Some(ready)).await
    });
    bound.await.unwrap();
    // On this single-threaded runtime the listener cannot run until the
    // request below yields: every probe is connected and closed in the
    // backlog before the listener accepts the first one, which is how a
    // loaded CI runner saw it with the real daemon.
    let probes = MAX_CONNECTIONS + MAX_BUSY_READERS + 32;
    for _ in 0..probes {
        drop(std::os::unix::net::UnixStream::connect(&paths.socket).unwrap());
    }
    let answer = request(&paths, Command::Status).await;
    assert!(
        answer.as_ref().is_ok_and(|answer| answer.ok),
        "{probes} closed probes held the slots of a live order: {answer:?}"
    );
    listener.abort();
    let _ = listener.await;
    actor.abort();
}

#[tokio::test]
async fn a_refusal_written_before_the_id_was_read_reaches_the_caller() {
    let directory = tempfile::tempdir().unwrap();
    let paths = Paths::resolve(Some(directory.path().to_path_buf())).unwrap();
    let (calls, _actor) = mpsc::channel(1);
    let (ready, bound) = oneshot::channel();
    let server_paths = paths.clone();
    let listener = tokio::spawn(async move {
        listen(&server_paths, calls, TopicChannels::new(), Some(ready)).await
    });
    bound.await.unwrap();
    let mut occupied = Vec::new();
    for _ in 0..MAX_CONNECTIONS + MAX_BUSY_READERS {
        let mut stream = UnixStream::connect(&paths.socket).await.unwrap();
        stream.write_all(b"{").await.unwrap();
        occupied.push(stream);
    }
    let error = request(&paths, Command::Status).await.unwrap_err();
    assert_eq!(
        error.downcast_ref::<Refusal>(),
        Some(&Refusal::TooMany {
            resource: Resource::IpcRequest,
            count: 69,
            max: 64,
        }),
        "{error:#}"
    );
    drop(occupied);
    listener.abort();
    let _ = listener.await;
}

#[test]
fn a_mismatched_answer_names_both_versions_and_ids() {
    let request = Request {
        v: PROTOCOL_VERSION,
        id: id(7),
        op: None,
        body: Command::Status,
    };
    let mut answer = response();
    answer.id = id(8);
    answer.v = PROTOCOL_VERSION + 1;
    answer.error = Some("cuerpo privado".into());
    let message = validate_response(&request, &answer)
        .unwrap_err()
        .to_string();
    assert_eq!(
        message,
        format!(
            "Respuesta IPC no válida: versión {} (esperada {PROTOCOL_VERSION}), identificador «{}» (esperado «{}»)",
            PROTOCOL_VERSION + 1,
            id(8),
            id(7)
        )
    );
    // An empty id without a refusal is no answer either, and says so.
    answer.v = PROTOCOL_VERSION;
    answer.id = String::new();
    answer.ok = true;
    let message = validate_response(&request, &answer)
        .unwrap_err()
        .to_string();
    assert!(message.contains("identificador «» (esperado"), "{message}");
}
