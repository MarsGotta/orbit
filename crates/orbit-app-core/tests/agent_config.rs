//! Ajustes › Agentes at the app boundary: each call goes from the
//! app through the in-process daemon to its hub route, with the device's
//! credential, and the hub's answer or refusal comes back whole. The hub is a
//! loopback fixture; every name and value is invented.
#![allow(clippy::unwrap_used)]
use orbit_app_core::DaemonClient;
use orbit_core::{
    agent_config::{AccessGroups, AgentCall, AgentOutcome, RefusalNumber},
    protocol::Command,
    refusal::Refusal,
    topics::Knowledge,
};
use orbit_ipc::Paths;
use serde_json::{Value, json};
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

const DEVICE_ID: &str = "00000000-0000-4000-8000-000000000311";
const TOKEN: &str = "synthetic-agents-token";
// The sessions fixture beside this one measured 3,969 ms worst cold start
// under load; thirty seconds bounds a stuck fixture, not product latency.
const CALL_BUDGET: Duration = Duration::from_secs(30);
// A daemon recovery was measured at 36.4 s worst on a loaded disk. This 120 s
// guard detects a jammed fixture; it is not a daemon startup target.
const RECOVERY_JAM_GUARD: Duration = Duration::from_secs(120);

/// What the hub fixture saw of one agent request.
#[derive(Debug, Clone)]
struct Seen {
    method: String,
    path: String,
    authorization: Option<String>,
    body: String,
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

async fn answer(
    mut stream: TcpStream,
    seen: Arc<Mutex<Vec<Seen>>>,
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
    let body = String::from_utf8_lossy(&bytes[header_end..]).into_owned();
    let (status, reply) = if path == "/v1/pair" {
        ("200 OK", json!({"device_id": DEVICE_ID, "token": TOKEN}))
    } else if path.starts_with("/v1/vault/") {
        seen.lock().unwrap().push(Seen {
            method: method.clone(),
            path: path.clone(),
            authorization,
            body: body.clone(),
        });
        vault_reply(&path, &body)
    } else if path.starts_with("/v1/agents/") || path.starts_with("/v1/settings/") {
        seen.lock().unwrap().push(Seen {
            method: method.clone(),
            path: path.clone(),
            authorization,
            body: body.clone(),
        });
        agent_reply(&method, &path, &body)
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

fn vault_reply(path: &str, body: &str) -> (&'static str, Value) {
    if path.ends_with("config") {
        return (
            "200 OK",
            json!({"configured":true,"kind":"folder","path":"/invented/Garden",
            "has_token":false,"link_style":"markdown","maintenance_model":"fixture/model",
            "maintenance_enabled":false}),
        );
    }
    let request: Value = serde_json::from_str(body).unwrap_or(Value::Null);
    if request["path"] == "large.md" {
        return (
            "413 Payload Too Large",
            json!({"code":"vault_note_too_large",
            "message":"La nota supera el máximo indicado.",
            "refusal":{"code":"vault_note_too_large","bytes":262145,"max_bytes":262144}}),
        );
    }
    (
        "200 OK",
        json!({"path":"personas/ada.md","frontmatter":{"type":"persona","title":"Ada"},
        "body":"A gardener.","raw":"# Ada\nA gardener.","outlinks":[],"synthesized":false,
        "citations":[],"tasks":[],"dependencies":[],"queries":[],"history":[]}),
    )
}

#[tokio::test(flavor = "multi_thread")]
async fn vault_calls_cross_app_actor_http_and_back_without_echoing_credentials()
-> Result<(), Box<dyn Error>> {
    use orbit_core::vault::{VaultCall, VaultOutcome};
    let hub = hub().await?;
    let (_directory, _daemon, app) = paired_client(&hub).await?;
    let configured = tokio::time::timeout(CALL_BUDGET, app.vault(VaultCall::Config)).await??;
    assert!(!serde_json::to_string(&configured)?.contains(TOKEN));
    assert!(
        matches!(configured, VaultOutcome::Config { configuration } if configuration.maintenance_model == "fixture/model")
    );
    let call: VaultCall = serde_json::from_value(
        json!({"call":"read","request":{"operation":"read","path":"personas/ada.md"}}),
    )?;
    let read = tokio::time::timeout(CALL_BUDGET, app.vault(call)).await??;
    assert!(matches!(read, VaultOutcome::Note { note } if note.body == "A gardener."));
    let call: VaultCall = serde_json::from_value(
        json!({"call":"read","request":{"operation":"read","path":"large.md"}}),
    )?;
    let refused = tokio::time::timeout(CALL_BUDGET, app.vault(call)).await??;
    assert!(
        matches!(refused, VaultOutcome::Refused { code, numbers, .. } if code == "vault_note_too_large" && numbers["bytes"] == 262145 && numbers["max_bytes"] == 262144)
    );
    let seen = hub.seen.lock().unwrap();
    assert_eq!(seen.len(), 3);
    assert!(
        seen.iter().all(
            |request| request.authorization.as_deref() == Some("bearer synthetic-agents-token")
        )
    );
    Ok(())
}

fn agent_reply(method: &str, path: &str, body: &str) -> (&'static str, Value) {
    match (method, path) {
        (_, path) if path.starts_with("/v1/settings/ai") => (
            "200 OK",
            json!({"providers": [{"id":"cedar","name":"Cedro","kind":"local","base_url":"http://127.0.0.1:8080/v1","key_configured":true,"models":[{"id":"cedar","capabilities":["text","tools"]}]}],"functions":[{"id":"assistants","provider_id":"cedar","model":"cedar","capability":"tools","privacy":"El texto se queda en tu hub."}]}),
        ),
        ("GET" | "PUT", "/v1/settings/transcription") => {
            ("200 OK", json!({ "model": "mai", "key_configured": true }))
        }
        ("GET", "/v1/agents/vega/config") => (
            "200 OK",
            json!({
                "agent_id": "vega", "kind": "openai",
                "personality": {"text": "Sé breve.", "editable": true, "max_bytes": 65536},
                "access": {"groups": {"screen": true, "control": false, "calendar": true,
                                      "meetings": true, "files": true, "notes": true}, "editable": true},
                "skills": {"editable": true, "max_skills": 2, "max_skill_bytes": 32768,
                           "items": [{"name": "resumen-diario",
                                      "description": "Resume el día.", "enabled": true}]}
            }),
        ),
        ("PUT", "/v1/agents/vega/access") => ("200 OK", json!({"ok": true})),
        // The library is full: the third skill is refused with its numbers.
        ("PUT", "/v1/agents/vega/skills/notas-de-viaje") if body.contains("content") => (
            "409 Conflict",
            json!({"code": "too_many_skills", "message": "Hay 2 skills; el máximo es 2",
                   "refusal": {"code": "too_many_skills", "count": 2, "max": 2,
                               "retry": {"kind": "after_fix"}}}),
        ),
        _ => (
            "404 Not Found",
            json!({"code": "agent_not_found", "message": "No existe",
                   "refusal": {"code": "agent_not_found", "agent_id": "orion",
                               "agent_ids": "vega", "retry": {"kind": "after_fix"}}}),
        ),
    }
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
            device_name: "Agents fixture".into(),
        },
    )
    .await?;
    assert!(response.ok, "pair refused: {:?}", response.refusal);
    // Waits on the event (the hub's `ready`), not on a deadline; the guard
    // only catches a fixture that never connects.
    tokio::time::timeout(CALL_BUDGET, async {
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
async fn the_config_an_order_and_a_refusal_cross_the_mac_whole() -> Result<(), Box<dyn Error>> {
    let hub = hub().await?;
    let (_directory, _daemon, app) = paired_client(&hub).await?;

    let read = AgentCall::Config {
        agent_id: "vega".into(),
    };
    let answer = tokio::time::timeout(CALL_BUDGET, app.agent(read.clone())).await??;
    assert_eq!(answer.call, read);
    let AgentOutcome::Config { config } = answer.outcome else {
        panic!("expected the config, got {:?}", answer.outcome)
    };
    assert_eq!(config.kind, "openai");
    assert!(!config.access.groups.control);

    let order = AgentCall::Access {
        agent_id: "vega".into(),
        groups: AccessGroups {
            control: true,
            ..config.access.groups
        },
    };
    let answer = tokio::time::timeout(CALL_BUDGET, app.agent(order)).await??;
    assert_eq!(answer.outcome, AgentOutcome::Done);

    let save = AgentCall::SkillSave {
        agent_id: "vega".into(),
        name: "notas-de-viaje".into(),
        content: "---\nname: notas-de-viaje\ndescription: Prepara un viaje.\n---\n".into(),
    };
    let answer = tokio::time::timeout(CALL_BUDGET, app.agent(save)).await??;
    let AgentOutcome::Refused { refusal } = answer.outcome else {
        panic!("expected the hub's refusal, got {:?}", answer.outcome)
    };
    assert_eq!(refusal.code, "too_many_skills");
    assert_eq!(refusal.status, 409);
    assert_eq!(
        refusal.numbers.get("count"),
        Some(&RefusalNumber::Integer(2))
    );
    assert_eq!(refusal.numbers.get("max"), Some(&RefusalNumber::Integer(2)));

    let seen = hub.seen.lock().unwrap().clone();
    let routes = seen
        .iter()
        .map(|seen| format!("{} {}", seen.method, seen.path))
        .collect::<Vec<_>>();
    assert_eq!(
        routes,
        [
            "GET /v1/agents/vega/config",
            "PUT /v1/agents/vega/access",
            "PUT /v1/agents/vega/skills/notas-de-viaje",
        ]
    );
    for request in &seen {
        assert_eq!(
            request.authorization.as_deref(),
            Some("bearer synthetic-agents-token")
        );
    }
    assert_eq!(
        serde_json::from_str::<Value>(&seen[1].body)?,
        json!({"groups": {"screen": true, "control": true, "calendar": true,
                          "meetings": true, "files": true, "notes": true}})
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unpaired_mac_says_so_without_asking_any_hub() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let paths = Paths::resolve(Some(directory.path().to_path_buf()))?;
    let daemon = TestDaemon::start(directory.path(), None)?;
    daemon.ready(RECOVERY_JAM_GUARD).await?;
    let app = DaemonClient::connect(paths, daemon.client.clone());
    let answer = tokio::time::timeout(
        CALL_BUDGET,
        app.agent(AgentCall::Config {
            agent_id: "vega".into(),
        }),
    )
    .await??;
    assert_eq!(
        answer.outcome,
        AgentOutcome::Failed {
            refusal: Refusal::NotPaired
        }
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn transcription_settings_cross_the_mac_and_the_key_never_returns()
-> Result<(), Box<dyn Error>> {
    let hub = hub().await?;
    let (_directory, _daemon, app) = paired_client(&hub).await?;
    let call: AgentCall = serde_json::from_value(json!({
        "call": "transcription_save", "model": "mai", "api_key": "invented-provider-secret"
    }))?;
    let answer = tokio::time::timeout(CALL_BUDGET, app.agent(call)).await??;
    assert!(matches!(
        answer.outcome,
        AgentOutcome::TranscriptionSettings { .. }
    ));
    assert!(!serde_json::to_string(&answer)?.contains("invented-provider-secret"));
    assert!(!format!("{answer:?}").contains("invented-provider-secret"));
    let requests = hub.seen.lock().unwrap();
    let request = requests.last().unwrap();
    assert_eq!(request.method, "PUT");
    assert_eq!(request.path, "/v1/settings/transcription");
    assert_eq!(
        request.authorization.as_deref(),
        Some("bearer synthetic-agents-token")
    );
    assert_eq!(
        serde_json::from_str::<Value>(&request.body)?["api_key"],
        "invented-provider-secret"
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn ai_settings_cross_the_mac_without_echoing_provider_credentials()
-> Result<(), Box<dyn Error>> {
    let hub = hub().await?;
    let (_directory, _daemon, app) = paired_client(&hub).await?;
    let call: AgentCall = serde_json::from_value(json!({
        "call":"ai_provider_save", "provider_id":"cedar", "provider": {
            "name":"Cedro", "kind":"local", "base_url":"http://127.0.0.1:8080/v1", "api_key":"invented-ai-provider-secret"
        }
    }))?;
    assert!(!format!("{call:?}").contains("invented-ai-provider-secret"));
    let answer = tokio::time::timeout(CALL_BUDGET, app.agent(call)).await??;
    assert!(matches!(answer.outcome, AgentOutcome::AiSettings { .. }));
    assert!(!serde_json::to_string(&answer)?.contains("invented-ai-provider-secret"));
    let read = tokio::time::timeout(CALL_BUDGET, app.agent(AgentCall::AiSettings)).await??;
    assert!(matches!(read.outcome, AgentOutcome::AiSettings { .. }));
    let call = serde_json::from_value(
        json!({"call":"ai_function_save","function":"assistants","choice":{"provider_id":"cedar","model":"cedar"}}),
    )?;
    let changed = tokio::time::timeout(CALL_BUDGET, app.agent(call)).await??;
    assert!(matches!(changed.outcome, AgentOutcome::AiSettings { .. }));
    let seen = hub.seen.lock().unwrap();
    assert_eq!(
        seen.iter().map(|r| r.path.as_str()).collect::<Vec<_>>(),
        [
            "/v1/settings/ai/providers/cedar",
            "/v1/settings/ai",
            "/v1/settings/ai/functions/assistants"
        ]
    );
    assert_eq!(
        serde_json::from_str::<Value>(&seen[0].body)?["api_key"],
        "invented-ai-provider-secret"
    );
    assert!(
        seen.iter()
            .all(|r| r.authorization.as_deref() == Some("bearer synthetic-agents-token"))
    );
    Ok(())
}
