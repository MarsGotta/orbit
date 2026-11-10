//! A plugin call at the app boundary: from the app through the in-process
//! daemon (no socket, so none of its line limits) to the hub's
//! `POST /v1/plugins/{id}/call`, and back. Each of the three outcomes comes
//! back as an answer, with the refusal's cause and numbers whole; only a
//! failure of the local channel itself is an error. The hub is a loopback
//! fixture; every name and value is invented.
#![allow(clippy::unwrap_used)]
use orbit_app_core::DaemonClient;
use orbit_core::{
    plugin::{PLUGIN_CALL_MAX_BYTES, PluginCall, PluginOutcome},
    protocol::Command,
    topics::Knowledge,
};
use orbit_ipc::Paths;
use serde_json::{Map, Value, json};
use std::{
    error::Error,
    sync::{Arc, Mutex},
    time::Duration,
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

const DEVICE_ID: &str = "00000000-0000-4000-8000-000000000647";
const TOKEN: &str = "synthetic-plugin-token";
// Bounds a stuck fixture, not product latency (the agents fixture beside
// this one measured 3,969 ms worst cold start under load).
const CALL_BUDGET: Duration = Duration::from_secs(30);
// A daemon recovery was measured at 36.4 s worst on a loaded disk; this
// guard detects a jammed fixture.
const RECOVERY_JAM_GUARD: Duration = Duration::from_secs(120);

/// What the hub fixture saw of one plugin request.
#[derive(Debug, Clone)]
struct Seen {
    method: String,
    path: String,
    authorization: Option<String>,
    body: Vec<u8>,
}

struct TestHub {
    url: String,
    task: JoinHandle<()>,
    seen: Arc<Mutex<Vec<Seen>>>,
}

impl Drop for TestHub {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn hub() -> Result<TestHub, Box<dyn Error>> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let url = format!("http://{}", listener.local_addr()?);
    let seen = Arc::new(Mutex::new(Vec::new()));
    let recorded = Arc::clone(&seen);
    let task = tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let recorded = Arc::clone(&recorded);
            tokio::spawn(async move {
                let _ = answer(stream, recorded).await;
            });
        }
    });
    Ok(TestHub { url, task, seen })
}

fn refusal_body() -> Value {
    json!({"code": "plugin_call_unknown", "message": "The plugin has no such call.",
        "op": "op_invented_647",
        "refusal": {"code": "plugin_call_unknown", "retry": {"kind": "after_fix"}}})
}

async fn answer(
    mut stream: TcpStream,
    seen: Arc<Mutex<Vec<Seen>>>,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    let mut bytes = Vec::new();
    let mut chunk = [0_u8; 16 * 1024];
    let header_end = loop {
        if let Some(index) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
            break index + 4;
        }
        let count = stream.read(&mut chunk).await?;
        // The synthetic requests have sub-2 KiB headers; this guards fixture
        // memory if a client loops, it is not a product cap.
        if count == 0 || bytes.len() + count > 64 * 1024 {
            return Ok(());
        }
        bytes.extend_from_slice(&chunk[..count]);
    };
    let head = String::from_utf8_lossy(&bytes[..header_end]).into_owned();
    let mut start = head.split_whitespace();
    let method = start.next().unwrap_or("").to_owned();
    let path = start.next().unwrap_or("").to_owned();
    if path == "/v1/ws" {
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
    let authorization = head.lines().find_map(|line| {
        line.to_ascii_lowercase()
            .strip_prefix("authorization:")
            .map(|value| value.trim().to_owned())
    });
    let body = bytes[header_end..].to_vec();
    let (status, reply) = if path == "/v1/pair" {
        ("200 OK", json!({"device_id": DEVICE_ID, "token": TOKEN}))
    } else if path.starts_with("/v1/plugins/") {
        let call: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
        seen.lock().unwrap().push(Seen {
            method,
            path,
            authorization,
            body,
        });
        if call["name"] == "config" {
            (
                "200 OK",
                json!({"value": {"configured": true, "path": "/invented/Garden",
                "pages": [1, 2.5, null, "dos"]}}),
            )
        } else {
            ("404 Not Found", refusal_body())
        }
    } else {
        ("404 Not Found", json!({"detail": "Not Found"}))
    };
    let encoded = serde_json::to_vec(&reply)?;
    let header = format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        encoded.len()
    );
    stream.write_all(header.as_bytes()).await?;
    stream.write_all(&encoded).await?;
    Ok(())
}

async fn started_client(
    directory: &tempfile::TempDir,
) -> Result<(TestDaemon, DaemonClient), Box<dyn Error>> {
    let paths = Paths::resolve(Some(directory.path().to_path_buf()))?;
    let daemon = TestDaemon::start(directory.path(), None)?;
    daemon.ready(RECOVERY_JAM_GUARD).await?;
    let app = DaemonClient::connect(paths, daemon.client.clone());
    Ok((daemon, app))
}

async fn paired_client(
    hub: &TestHub,
) -> Result<(tempfile::TempDir, TestDaemon, DaemonClient), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let (daemon, app) = started_client(&directory).await?;
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
            device_name: "Plugin fixture".into(),
        },
    )
    .await?;
    assert!(response.ok, "pair refused: {:?}", response.refusal);
    // Waits on the hub's `ready`; the guard only catches a fixture that
    // never connects.
    tokio::time::timeout(CALL_BUDGET, async {
        loop {
            let frame = subscription.next_topic().await?;
            if matches!(frame.data, orbit_core::topics::TopicData::Status(Knowledge::Present { value, .. }) if value.hub == orbit_core::Connection::Online) {
                return Ok::<_, Box<dyn Error>>(());
            }
        }
    })
    .await??;
    Ok((directory, daemon, app))
}

fn call(name: &str, input: Value) -> PluginCall {
    let Value::Object(input) = input else {
        panic!("an input is an object");
    };
    PluginCall {
        name: name.into(),
        input,
    }
}

/// A call whose body, as the device writes it, is `bytes` long.
fn call_of(bytes: usize) -> Result<PluginCall, Box<dyn Error>> {
    let empty = serde_json::to_vec(&call("read", json!({"text": ""})))?.len();
    let call = call("read", json!({"text": "a".repeat(bytes - empty)}));
    assert_eq!(serde_json::to_vec(&call)?.len(), bytes);
    Ok(call)
}

#[tokio::test(flavor = "multi_thread")]
async fn each_outcome_of_a_call_reaches_the_app_whole() -> Result<(), Box<dyn Error>> {
    let hub = hub().await?;
    let (_directory, _daemon, app) = paired_client(&hub).await?;

    // Answered: the hub's `value`, untouched.
    let answered = tokio::time::timeout(
        CALL_BUDGET,
        app.plugin_call("vault".into(), call("config", json!({}))),
    )
    .await??;
    assert_eq!(
        serde_json::to_value(&answered)?,
        json!({"outcome": "answered", "value": {"configured": true,
            "path": "/invented/Garden", "pages": [1, 2.5, null, "dos"]}})
    );
    {
        let seen = hub.seen.lock().unwrap();
        assert_eq!(seen.len(), 1, "one call is one request");
        assert_eq!(seen[0].method, "POST");
        assert_eq!(seen[0].path, "/v1/plugins/vault/call");
        assert_eq!(
            seen[0].authorization.as_deref(),
            Some("bearer synthetic-plugin-token")
        );
        let sent: Value = serde_json::from_slice(&seen[0].body)?;
        assert_eq!(sent, json!({"name": "config", "input": {}}));
    }
    assert!(!serde_json::to_string(&answered)?.contains(TOKEN));

    // Refused: the hub's status and body as it wrote them, `op` included.
    let refused = tokio::time::timeout(
        CALL_BUDGET,
        app.plugin_call("vault".into(), call("missing", json!({"page": 0}))),
    )
    .await??;
    let Value::Object(body) = refusal_body() else {
        unreachable!()
    };
    assert_eq!(refused, PluginOutcome::Refused { status: 404, body });
    assert_eq!(
        serde_json::to_value(&refused)?,
        json!({"outcome": "refused", "status": 404, "body": refusal_body()})
    );
    assert_eq!(hub.seen.lock().unwrap().len(), 2);

    // Failed, by the device: in the process nothing bounds the order before
    // the device does, so a call one byte over its limit reaches the device's
    // own refusal, with its numbers, and never the hub.
    let over = call_of(PLUGIN_CALL_MAX_BYTES + 1)?;
    let failed = tokio::time::timeout(CALL_BUDGET, app.plugin_call("vault".into(), over)).await??;
    assert_eq!(
        serde_json::to_value(&failed)?,
        json!({"outcome": "failed", "refusal": {"code": "plugin_call_too_large",
            "bytes": 1_048_577, "max_bytes": 1_048_576, "retry": {"kind": "after_fix"}}})
    );
    // A malformed id: the device's refusal names the identifier's shape.
    let malformed = tokio::time::timeout(
        CALL_BUDGET,
        app.plugin_call("Vault".into(), call("config", json!({}))),
    )
    .await??;
    assert_eq!(
        serde_json::to_value(&malformed)?,
        json!({"outcome": "failed", "refusal": {"code": "invalid_parameter",
            "parameter": "identifier", "expected": {"kind": "wire_type", "name": "plugin_id"},
            "retry": {"kind": "after_fix"}}})
    );
    assert_eq!(hub.seen.lock().unwrap().len(), 2, "no request for either");

    // At its limit the whole call goes, once.
    let at = call_of(PLUGIN_CALL_MAX_BYTES)?;
    let written = serde_json::to_vec(&at)?;
    let at_limit = tokio::time::timeout(CALL_BUDGET, app.plugin_call("vault".into(), at)).await??;
    assert!(matches!(
        at_limit,
        PluginOutcome::Refused { status: 404, .. }
    ));
    let seen = hub.seen.lock().unwrap();
    assert_eq!(seen.len(), 3);
    assert_eq!(seen[2].body.len(), written.len());
    assert_eq!(
        serde_json::from_slice::<Value>(&seen[2].body)?,
        serde_json::from_slice::<Value>(&written)?
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn the_devices_refusal_is_an_answer_and_a_closed_channel_an_error()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let (daemon, app) = started_client(&directory).await?;
    let unpaired = tokio::time::timeout(
        CALL_BUDGET,
        app.plugin_call("vault".into(), call("config", json!({}))),
    )
    .await??;
    assert_eq!(
        serde_json::to_value(&unpaired)?,
        json!({"outcome": "failed", "refusal": {"code": "not_paired", "retry": {"kind": "after_fix"}}})
    );
    daemon.shutdown(RECOVERY_JAM_GUARD).await?;
    let closed = tokio::time::timeout(
        CALL_BUDGET,
        app.plugin_call(
            "vault".into(),
            call("config", json!({"a": Map::<String, Value>::new()})),
        ),
    )
    .await?;
    match closed {
        Err(error) => assert_eq!(error.code, "daemon_error"),
        Ok(outcome) => panic!("a closed channel is no outcome: {outcome:?}"),
    }
    Ok(())
}
