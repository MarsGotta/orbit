//! Conversation pages at the app boundary, with isolated HTTP fixtures and the in-process daemon.
#![allow(clippy::unwrap_used)]
use orbit_app_core::DaemonClient;
use orbit_core::{protocol::Command, refusal::Refusal, topics::Knowledge};
use orbit_ipc::Paths;
use serde_json::{Value, json};
use std::{
    error::Error,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    task::JoinHandle,
};

#[path = "support/embedded_daemon.rs"]
mod embedded_daemon;
use embedded_daemon::TestDaemon;
#[path = "support/websocket.rs"]
mod websocket;

const DEVICE_ID: &str = "00000000-0000-4000-8000-000000000211";
const TOKEN: &str = "synthetic-sessions-token";
// The adjacent daemon fixture measured 3,969 ms worst cold start
// under load. Thirty seconds bounds a stuck fixture, not product latency.
// The same fixture uses thirty seconds to catch a lost local reply; this is
// a test hang guard, not a hub or app response-time promise.
const READ_BUDGET: Duration = Duration::from_secs(30);
// A daemon recovery was measured at 36.4 s worst on a loaded disk. This 120 s guard
// detects a jammed fixture; it is not a daemon startup target.
const RECOVERY_JAM_GUARD: Duration = Duration::from_secs(120);

struct TestHub {
    url: String,
    task: JoinHandle<()>,
    sessions_requests: Arc<AtomicUsize>,
}

#[derive(Clone, Copy)]
enum HubBehavior {
    Paged,
    Repeated,
    MissingCursor,
    Oversized,
    InvalidRow,
    WrongCursor,
    Stalled,
}
impl Drop for TestHub {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn hub(behavior: HubBehavior) -> Result<TestHub, Box<dyn Error>> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let url = format!("http://{}", listener.local_addr()?);
    let sessions_requests = Arc::new(AtomicUsize::new(0));
    let request_count = Arc::clone(&sessions_requests);
    let rows = Arc::new(
        (0..500)
            .rev()
            .map(|index| {
                let mut row = json!({
                    "id": format!("00000000-0000-4000-8000-{index:012x}"),
                    "created_at_ms": 1_700_000_000_000_i64 + index,
                    "last_at_ms": 1_700_000_000_000_i64 + index,
                    "message_count": 0,
                });
                // Every other conversation is with a second agent; the
                // rest carry no agent, as a hub without agents lists them.
                if index % 2 == 1 {
                    row["agent_id"] = json!("atlas");
                }
                row
            })
            .collect::<Vec<_>>(),
    );
    let task = tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let rows = Arc::clone(&rows);
            let sessions_requests = Arc::clone(&sessions_requests);
            tokio::spawn(async move {
                let _ = answer(stream, rows.as_slice(), behavior, sessions_requests).await;
            });
        }
    });
    Ok(TestHub {
        url,
        task,
        sessions_requests: request_count,
    })
}

async fn answer(
    mut stream: TcpStream,
    rows: &[Value],
    behavior: HubBehavior,
    sessions_requests: Arc<AtomicUsize>,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    let mut bytes = Vec::new();
    let mut chunk = [0_u8; 4096];
    let header_end = loop {
        if let Some(index) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
            break index + 4;
        }
        let count = stream.read(&mut chunk).await?;
        // The synthetic requests have sub-2 KiB headers. This 64 KiB guard
        // protects fixture memory if a client loops; it is not a product cap.
        if count == 0 || bytes.len() + count > 64 * 1024 {
            return Ok(());
        }
        bytes.extend_from_slice(&chunk[..count]);
    };
    let head = String::from_utf8_lossy(&bytes[..header_end]).into_owned();
    let path = head.split_whitespace().nth(1).unwrap_or("");
    if path == "/v1/ws" {
        // The daemon reads the conversations only with the hub
        // online, so the hub says `ready` and keeps the socket.
        let accepted = websocket::switching_protocols(&head)?;
        stream.write_all(accepted.as_bytes()).await?;
        let stream = stream.into_std()?;
        stream.set_nonblocking(false)?;
        tokio::task::spawn_blocking(move || websocket::serve_ready(stream, DEVICE_ID)).await?;
        return Ok(());
    }
    let content_length = head
        .lines()
        .find_map(|line| {
            line.to_ascii_lowercase()
                .strip_prefix("content-length:")
                .and_then(|value| value.trim().parse::<usize>().ok())
        })
        .unwrap_or(0);
    while bytes.len() < header_end + content_length {
        let count = stream.read(&mut chunk).await?;
        if count == 0 {
            return Ok(());
        }
        bytes.extend_from_slice(&chunk[..count]);
    }
    let body = if path == "/v1/pair" {
        json!({"device_id": DEVICE_ID, "token": TOKEN})
    } else if path.starts_with("/v1/sessions/") && path.ends_with("/messages") {
        json!({"messages": [
            {"id": 1, "role": "user", "text": "Hola", "at_ms": 1_700_000_000_000_i64,
                "kind": "voice", "duration_ms": 1_500, "client_id": "voice-sessions"},
            {"id": 2, "role": "assistant", "text": "Hola, Alina", "at_ms": 1_700_000_001_000_i64,
                "kind": "text"}
        ], "next": null, "total": 2})
    } else if path.starts_with("/v1/sessions") {
        sessions_requests.fetch_add(1, Ordering::SeqCst);
        if matches!(behavior, HubBehavior::Stalled) {
            tokio::time::sleep(Duration::from_secs(105)).await;
        }
        if matches!(behavior, HubBehavior::Oversized) {
            let header = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                orbit_core::MAX_FRAME_BYTES + 1
            );
            stream.write_all(header.as_bytes()).await?;
            return Ok(());
        }
        let before = path.split_once("?before=").map(|(_, before)| before);
        let start = if matches!(behavior, HubBehavior::Paged | HubBehavior::MissingCursor) {
            before
                .filter(|before| !before.is_empty())
                .and_then(|before| rows.iter().position(|row| row["id"] == before))
                .map_or(0, |index| index + 1)
        } else {
            0
        };
        let end = (start + 128).min(rows.len());
        let mut page = rows[start..end].to_vec();
        if matches!(behavior, HubBehavior::InvalidRow) {
            page[0]["id"] = json!("");
        }
        let next = (end < rows.len())
            .then(|| page.last().map(|row| row["id"].as_str().unwrap_or("")))
            .flatten();
        let next = if matches!(behavior, HubBehavior::WrongCursor) {
            Some("00000000-0000-4000-8000-ffffffffffff")
        } else {
            next
        };
        json!({"sessions": page, "total": rows.len(), "next": next})
    } else {
        json!({})
    };
    let encoded = serde_json::to_vec(&body)?;
    let status = if matches!(behavior, HubBehavior::MissingCursor)
        && path.starts_with("/v1/sessions?before=")
        && path != "/v1/sessions?before="
    {
        "404 Not Found"
    } else if path == "/v1/pair" || path.starts_with("/v1/sessions") {
        "200 OK"
    } else {
        "404 Not Found"
    };
    let header = format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        encoded.len()
    );
    stream.write_all(header.as_bytes()).await?;
    stream.write_all(&encoded).await?;
    Ok(())
}

async fn paired_client(
    hub: &TestHub,
) -> Result<(tempfile::TempDir, TestDaemon, DaemonClient), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let paths = Paths::resolve(Some(directory.path().to_path_buf()))?;
    let daemon = TestDaemon::start(directory.path(), None)?;
    daemon.ready(RECOVERY_JAM_GUARD).await?;
    let (_, mut subscription) = embedded_daemon::subscribe_topics(
        &daemon.client,
        vec![orbit_core::topics::TopicId::Status],
    )
    .await?;
    let response = embedded_daemon::request(
        &daemon.client,
        Command::Pair {
            hub_url: hub.url.clone(),
            code: "synthetic-code".into(),
            device_name: "Sessions fixture".into(),
        },
    )
    .await?;
    assert!(response.ok, "pair refused: {:?}", response.refusal);
    // Waits on the event (the hub's `ready`), not on a deadline; the guard
    // only catches a fixture that never connects.
    tokio::time::timeout(READ_BUDGET, async {
        loop {
            let frame = subscription.next_topic().await?;
            if matches!(frame.data, orbit_core::topics::TopicData::Status(Knowledge::Present { value, .. }) if value.hub == orbit_core::Connection::Online) {
                return Ok::<_, Box<dyn Error>>(());
            }
        }
    })
    .await??;
    let app = DaemonClient::connect(paths, daemon.client.clone());
    Ok((directory, daemon, app))
}

#[tokio::test(flavor = "multi_thread")]
async fn five_hundred_hub_conversations_reach_the_app_in_four_pages() -> Result<(), Box<dyn Error>>
{
    let hub = hub(HubBehavior::Paged).await?;
    let (_directory, _daemon, app) = paired_client(&hub).await?;
    let initial_requests = hub.sessions_requests.load(Ordering::SeqCst);
    let mut cursor = None;
    let mut ids = Vec::new();
    let mut pages = 0;
    loop {
        let started = Instant::now();
        let answer = tokio::time::timeout(READ_BUDGET, app.read_sessions_page(cursor)).await??;
        eprintln!("session page {} ms", started.elapsed().as_millis());
        let Knowledge::Present { value, .. } = answer else {
            panic!("expected a complete page")
        };
        assert_eq!(value.total, 500);
        ids.extend(value.items.into_iter().map(|session| session.id));
        pages += 1;
        cursor = value.next;
        if cursor.is_none() {
            break;
        }
    }
    assert_eq!(pages, 4);
    assert_eq!(
        hub.sessions_requests.load(Ordering::SeqCst) - initial_requests,
        4,
        "the keyed read must not invoke the old automatic sessions refresh"
    );
    assert_eq!(ids.len(), 500);
    assert_eq!(
        ids.iter().collect::<std::collections::BTreeSet<_>>().len(),
        500
    );
    Ok(())
}

/// Each conversation reaches the app with the agent it is with.
#[tokio::test(flavor = "multi_thread")]
async fn each_conversation_reaches_the_app_with_its_agent() -> Result<(), Box<dyn Error>> {
    let hub = hub(HubBehavior::Paged).await?;
    let (_directory, _daemon, app) = paired_client(&hub).await?;
    let answer = tokio::time::timeout(READ_BUDGET, app.read_sessions_page(None)).await??;
    let Knowledge::Present { value, .. } = answer else {
        panic!("expected a complete page")
    };
    // Newest first: index 499 (atlas), then 498 (none).
    let agents = value
        .items
        .iter()
        .take(2)
        .map(|session| session.agent_id.as_deref())
        .collect::<Vec<_>>();
    assert_eq!(agents, [Some("atlas"), None]);
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn a_repeated_hub_page_cannot_pose_as_the_next_page() -> Result<(), Box<dyn Error>> {
    let hub = hub(HubBehavior::Repeated).await?;
    let (_directory, _daemon, app) = paired_client(&hub).await?;
    let first = tokio::time::timeout(READ_BUDGET, app.read_sessions_page(None)).await??;
    let Knowledge::Present { value, .. } = first else {
        panic!("expected the first page")
    };
    let second = tokio::time::timeout(READ_BUDGET, app.read_sessions_page(value.next)).await??;
    assert!(matches!(
        second,
        Knowledge::Absent {
            refusal: Refusal::HubSessionsUnpaged {
                delivered: 128,
                total: 500
            }
        }
    ));
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn a_missing_hub_cursor_is_a_conversation_not_found() -> Result<(), Box<dyn Error>> {
    let hub = hub(HubBehavior::MissingCursor).await?;
    let (_directory, _daemon, app) = paired_client(&hub).await?;
    let first = tokio::time::timeout(READ_BUDGET, app.read_sessions_page(None)).await??;
    let Knowledge::Present { value, .. } = first else {
        panic!("expected the first page")
    };
    let second = tokio::time::timeout(READ_BUDGET, app.read_sessions_page(value.next)).await??;
    // The hub's 404 is a conversation it does not know, named as such.
    assert!(matches!(
        second,
        Knowledge::Absent {
            refusal: Refusal::NotFound {
                resource: orbit_core::refusal::Resource::Session
            }
        }
    ));
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn a_privacy_stop_prevents_the_sessions_http_request() -> Result<(), Box<dyn Error>> {
    let hub = hub(HubBehavior::Paged).await?;
    let (_directory, _daemon, app) = paired_client(&hub).await?;
    let stopped = embedded_daemon::request(&_daemon.client, Command::PrivacyStop).await?;
    assert!(stopped.ok, "privacy stop: {:?}", stopped.refusal);
    let before = hub.sessions_requests.load(Ordering::SeqCst);
    let answer = tokio::time::timeout(READ_BUDGET, app.read_sessions_page(None)).await??;
    assert!(matches!(
        answer,
        Knowledge::Absent {
            refusal: Refusal::PrivacyStopped { .. }
        }
    ));
    assert_eq!(hub.sessions_requests.load(Ordering::SeqCst), before);
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn a_connection_failure_names_the_hub() -> Result<(), Box<dyn Error>> {
    let mut hub = hub(HubBehavior::Paged).await?;
    let (_directory, _daemon, app) = paired_client(&hub).await?;
    // `abort` only schedules the cancellation: until a worker drops the
    // accept loop, its listener stays open, the kernel accepts the read's
    // connection into the backlog and then resets it, which is a hub that
    // answered badly, not one that is down (3 gate runs on 2026-10-21 under
    // load). The handle resolves only after the loop, and its listener, are
    // dropped.
    hub.task.abort();
    let _ = (&mut hub.task).await;
    let answer = tokio::time::timeout(READ_BUDGET, app.read_sessions_page(None)).await??;
    assert!(
        matches!(
            answer,
            Knowledge::Absent {
                refusal: Refusal::HubUnreachable { .. }
            }
        ),
        "{answer:?}"
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn an_oversized_hub_body_names_its_bytes_and_ceiling() -> Result<(), Box<dyn Error>> {
    let hub = hub(HubBehavior::Oversized).await?;
    let (_directory, _daemon, app) = paired_client(&hub).await?;
    let answer = tokio::time::timeout(READ_BUDGET, app.read_sessions_page(None)).await??;
    assert!(
        matches!(answer, Knowledge::Absent { refusal: Refusal::TooLarge {
        bytes, max_bytes, ..
    } } if bytes == orbit_core::MAX_FRAME_BYTES as u64 + 1 && max_bytes == orbit_core::MAX_FRAME_BYTES as u64)
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn an_invalid_hub_row_names_its_position_and_size() -> Result<(), Box<dyn Error>> {
    let hub = hub(HubBehavior::InvalidRow).await?;
    let (_directory, _daemon, app) = paired_client(&hub).await?;
    let answer = tokio::time::timeout(READ_BUDGET, app.read_sessions_page(None)).await??;
    let refusal = match answer {
        Knowledge::Absent { refusal } => serde_json::to_value(refusal)?,
        _ => panic!("expected refusal"),
    };
    assert_eq!(refusal["code"], "hub_session_invalid_row");
    assert_eq!(refusal["position"], 1);
    assert_eq!(refusal["rows"], 128);
    assert_eq!(refusal["bytes"], 0);
    assert_eq!(refusal["max_bytes"], 121);
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn a_wrong_hub_cursor_names_the_page_size() -> Result<(), Box<dyn Error>> {
    let hub = hub(HubBehavior::WrongCursor).await?;
    let (_directory, _daemon, app) = paired_client(&hub).await?;
    let answer = tokio::time::timeout(READ_BUDGET, app.read_sessions_page(None)).await??;
    let refusal = match answer {
        Knowledge::Absent { refusal } => serde_json::to_value(refusal)?,
        _ => panic!("expected refusal"),
    };
    assert_eq!(refusal["code"], "hub_sessions_cursor_mismatch");
    assert_eq!(refusal["rows"], 128);
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn a_stalled_hub_request_names_the_wait_and_deadline() -> Result<(), Box<dyn Error>> {
    let hub = hub(HubBehavior::Stalled).await?;
    let (_directory, _daemon, app) = paired_client(&hub).await?;
    let answer = tokio::time::timeout(READ_BUDGET, app.read_sessions_page(None)).await??;
    assert!(
        matches!(answer, Knowledge::Absent { refusal: Refusal::Timeout {
        waited_ms, deadline_ms: 8_000, ..
    } } if waited_ms >= 8_000)
    );
    Ok(())
}

/// At the app boundary: a conversation opened from the list
/// reaches the app as its messages, read from the hub through the daemon.
#[tokio::test(flavor = "multi_thread")]
async fn an_opened_conversation_reaches_the_app_with_its_messages() -> Result<(), Box<dyn Error>> {
    let hub = hub(HubBehavior::Paged).await?;
    let (_directory, _daemon, app) = paired_client(&hub).await?;
    let session = "00000000-0000-4000-8000-000000000001".to_owned();
    let answer =
        tokio::time::timeout(READ_BUDGET, app.read_session_messages_page(session, None)).await??;
    let Knowledge::Present { value, .. } = answer else {
        panic!("the conversation was not read: {answer:?}")
    };
    let texts = value
        .items
        .iter()
        .map(|message| message.text.as_str())
        .collect::<Vec<_>>();
    assert_eq!(texts, ["Hola", "Hola, Alina"]);
    let voice = value.items[0]
        .voice
        .as_ref()
        .ok_or("the voice note is missing")?;
    assert_eq!(voice.duration_ms, 1_500);
    assert_eq!(voice.transcript.as_deref(), Some("Hola"));
    assert!(!voice.audio_available, "this Mac never held that audio");
    assert_eq!(value.items[0].id, "voice-sessions");
    assert_eq!(value.total, 2);
    Ok(())
}
