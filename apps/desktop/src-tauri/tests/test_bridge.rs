#![cfg(feature = "pruebas")]
use orbit_desktop::test_bridge::{Bridge, View, ViewFuture};
use serde_json::{Value, json};
use std::{os::unix::fs::PermissionsExt, sync::Arc, time::Duration};
use tokio::{io::AsyncWriteExt, net::UnixStream};

struct FakeView;
impl View for FakeView {
    fn request(&self, request: Value) -> ViewFuture {
        Box::pin(async move {
            if request.get("name") == Some(&json!("Slow")) {
                std::future::pending::<()>().await;
            }
            match request["command"].as_str().unwrap_or_default() {
                "press" | "type" => {
                    let count = match request["name"].as_str() {
                        Some("Missing") => 0,
                        Some("Doubled") => 2,
                        _ => 1,
                    };
                    if count != 1 {
                        return json!({"ok":false,"cause":"match_count","count":count,
                            "name":request["name"],"role":request["role"]});
                    }
                    json!({"ok":true,"count":1})
                }
                "read" => {
                    json!({"ok":true,"elements":[{"role":"button","name":"Record","text":"Record"}]})
                }
                "menus" => {
                    json!({"ok":true,"tray":{"title":"00:12","rows":[{"name":"Mute","enabled":true}]},"application":[]})
                }
                "windows" => {
                    json!({"ok":true,"windows":[{"label":"main","window_id":11,"visible":true,"frame":{"x":0,"y":0,"width":800,"height":600}}]})
                }
                _ => json!({"ok":false,"cause":"unsupported_command"}),
            }
        })
    }
}
async fn call(path: &std::path::Path, request: &[u8]) -> anyhow::Result<Value> {
    let mut stream = UnixStream::connect(path).await?;
    stream.write_all(request).await?;
    let line = orbit_ipc::read_frame(tokio::io::BufReader::new(stream)).await?;
    Ok(serde_json::from_slice(&line)?)
}
#[tokio::test]
async fn commands_cross_the_owner_only_socket_and_a_slow_view_is_refused() -> anyhow::Result<()> {
    let directory = tempfile::tempdir()?;
    let bridge = Bridge::bind(
        directory.path(),
        Arc::new(FakeView),
        Duration::from_millis(50),
    )
    .await?;
    assert_eq!(
        std::fs::metadata(bridge.path())?.permissions().mode() & 0o777,
        0o600
    );
    for command in ["press", "type", "read", "menus", "windows"] {
        let request = format!(
            "{}\n",
            json!({"command":command,"window":"main","role":"button","name":"Record","text":"Hello"})
        );
        let reply = call(bridge.path(), request.as_bytes()).await?;
        assert_eq!(reply["ok"], true, "{reply}");
        match command {
            "read" => assert_eq!(reply["elements"][0]["name"], "Record"),
            "menus" => assert_eq!(reply["tray"]["title"], "00:12"),
            "windows" => assert_eq!(reply["windows"][0]["window_id"], 11),
            _ => assert_eq!(reply["count"], 1),
        }
    }
    for (name, count) in [("Missing", 0), ("Doubled", 2)] {
        let request = format!(
            "{}\n",
            json!({"command":"press","window":"main","role":"button","name":name})
        );
        let reply = call(bridge.path(), request.as_bytes()).await?;
        assert_eq!(reply["cause"], "match_count");
        assert_eq!(reply["name"], name);
        assert_eq!(reply["count"], count);
    }
    let start = std::time::Instant::now();
    let reply = call(
        bridge.path(),
        b"{\"command\":\"press\",\"window\":\"main\",\"role\":\"button\",\"name\":\"Slow\"}\n",
    )
    .await?;
    assert_eq!(reply["cause"], "view_timeout");
    assert_eq!(reply["bound_ms"], 50);
    assert!(start.elapsed() < Duration::from_secs(2));
    let path = bridge.path().to_owned();
    drop(bridge);
    assert!(!path.exists());
    Ok(())
}
#[tokio::test]
async fn oversized_and_invalid_requests_are_refused_without_dispatch() -> anyhow::Result<()> {
    let directory = tempfile::tempdir()?;
    let bridge = Bridge::bind(directory.path(), Arc::new(FakeView), Duration::from_secs(2)).await?;
    let request = vec![b'x'; orbit_core::limits::MAX_IPC_LINE_BYTES + 1];
    // Concurrent writing permits a refusal before the sender finishes its overlong line.
    let mut stream = UnixStream::connect(bridge.path()).await?;
    let (reader, mut writer) = stream.split();
    let (_, reply) = tokio::join!(
        writer.write_all(&request),
        orbit_ipc::read_frame(tokio::io::BufReader::new(reader))
    );
    let reply: Value = serde_json::from_slice(&reply?)?;
    assert_eq!(reply["ok"], false);
    assert!(reply["max_bytes"].as_u64().is_some());
    assert_eq!(reply["cause"], "request_too_large");
    assert_eq!(
        call(bridge.path(), b"{}\n").await?["cause"],
        "invalid_request"
    );
    Ok(())
}

#[test]
fn pruebas_executable_really_contains_the_bridge_markers() -> anyhow::Result<()> {
    let executable = std::fs::read(std::env::var("CARGO_BIN_EXE_orbit-desktop")?)?;
    for marker in [
        b"orbit-test-bridge-v1".as_slice(),
        b"orbit-test-bridge.sock".as_slice(),
    ] {
        assert!(executable.windows(marker.len()).any(|part| part == marker));
    }
    Ok(())
}

struct LargeView;
impl View for LargeView {
    fn request(&self, _: Value) -> ViewFuture {
        Box::pin(async {
            json!({"ok":true,"text":"x".repeat(orbit_core::limits::MAX_IPC_LINE_BYTES)})
        })
    }
}
#[tokio::test]
async fn large_replies_are_refused_with_the_observed_size() -> anyhow::Result<()> {
    let directory = tempfile::tempdir()?;
    let bridge = Bridge::bind(
        directory.path(),
        Arc::new(LargeView),
        Duration::from_secs(2),
    )
    .await?;
    let reply = call(
        bridge.path(),
        b"{\"command\":\"read\",\"window\":\"main\"}\n",
    )
    .await?;
    assert_eq!(reply["cause"], "reply_too_large");
    assert!(reply["bytes"].as_u64() > reply["max_bytes"].as_u64());
    Ok(())
}

#[tokio::test]
async fn occupied_paths_are_preserved_and_a_live_bridge_is_never_replaced() -> anyhow::Result<()> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("orbit-test-bridge.sock");
    std::fs::write(&path, "fixture")?;
    assert!(
        Bridge::bind(directory.path(), Arc::new(FakeView), Duration::from_secs(2))
            .await
            .is_err()
    );
    assert_eq!(std::fs::read_to_string(&path)?, "fixture");
    std::fs::remove_file(path)?;
    let first = Bridge::bind(directory.path(), Arc::new(FakeView), Duration::from_secs(2)).await?;
    assert!(
        Bridge::bind(directory.path(), Arc::new(FakeView), Duration::from_secs(2))
            .await
            .is_err()
    );
    assert_eq!(
        call(first.path(), b"{\"command\":\"menus\"}\n").await?["ok"],
        true
    );
    Ok(())
}

#[tokio::test]
async fn stopping_twice_never_unlinks_the_next_instances_socket() -> anyhow::Result<()> {
    let directory = tempfile::tempdir()?;
    let first = Bridge::bind(directory.path(), Arc::new(FakeView), Duration::from_secs(2)).await?;
    first.stop();
    let second = Bridge::bind(directory.path(), Arc::new(FakeView), Duration::from_secs(2)).await?;
    drop(first);
    assert_eq!(
        call(second.path(), b"{\"command\":\"menus\"}\n").await?["ok"],
        true
    );
    Ok(())
}

struct WaitingView(tokio::sync::mpsc::Sender<()>);
impl View for WaitingView {
    fn request(&self, _: Value) -> ViewFuture {
        let entered = self.0.clone();
        Box::pin(async move {
            let _ = entered.send(()).await;
            std::future::pending().await
        })
    }
}
#[tokio::test]
async fn full_view_capacity_refuses_the_next_peer_without_queuing_it() -> anyhow::Result<()> {
    let directory = tempfile::tempdir()?;
    let (entered, mut events) = tokio::sync::mpsc::channel(4);
    let bridge = Bridge::bind(
        directory.path(),
        Arc::new(WaitingView(entered)),
        Duration::from_secs(10),
    )
    .await?;
    let mut clients = Vec::new();
    for _ in 0..4 {
        let mut client = UnixStream::connect(bridge.path()).await?;
        client.write_all(b"{\"command\":\"menus\"}\n").await?;
        tokio::time::timeout(Duration::from_secs(2), events.recv())
            .await?
            .ok_or_else(|| anyhow::anyhow!("view never entered"))?;
        clients.push(client);
    }
    let reply = tokio::time::timeout(
        Duration::from_secs(2),
        call(bridge.path(), b"{\"command\":\"menus\"}\n"),
    )
    .await??;
    assert_eq!(reply["cause"], "bridge_busy");
    assert_eq!(reply["in_flight"], 4);
    assert_eq!(reply["max_connections"], 4);
    drop(clients);
    Ok(())
}

struct InvalidView;
impl View for InvalidView {
    fn request(&self, _: Value) -> ViewFuture {
        Box::pin(async { json!([]) })
    }
}
#[tokio::test]
async fn invalid_view_results_are_refusals_rather_than_panics() -> anyhow::Result<()> {
    let directory = tempfile::tempdir()?;
    let bridge = Bridge::bind(
        directory.path(),
        Arc::new(InvalidView),
        Duration::from_secs(2),
    )
    .await?;
    assert_eq!(
        call(bridge.path(), b"{\"command\":\"menus\"}\n").await?["cause"],
        "invalid_view_reply"
    );
    Ok(())
}
