//! What the device checks of a plugin call before the hub sees it: the shape
//! of the plugin's `id` and the call's size (`contracts/plugin-v1.md`, «Call»
//! and «Limits»). A call it refuses never opens a connection; one it admits
//! is sent once, as written, to `POST /v1/plugins/{id}/call`.
use anyhow::{Context, Result};
use orbit_core::plugin::PluginCall;
use orbit_daemon::hub_notes::NoteHub;
use serde_json::{Value, json};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

const PLUGIN_CALL_MAX_BYTES: usize = 1_048_576;
/// Long enough for a device that does connect to have done so: the call
/// under test has already returned when this wait starts.
const NO_CONNECTION_WAIT: Duration = Duration::from_millis(300);
/// The device waits 8 s for the hub; a test that waits longer has failed.
const TEST_WAIT: Duration = Duration::from_secs(20);

async fn hub_at(listener: &TcpListener) -> Result<NoteHub> {
    Ok(NoteHub {
        base: format!("http://{}/", listener.local_addr()?).parse()?,
        token: "synthetic-device-token".into(),
        op: orbit_ipc::new_op(),
    })
}

fn call(name: &str, input: Value) -> Result<PluginCall> {
    Ok(serde_json::from_value(
        json!({"name": name, "input": input}),
    )?)
}

/// A call whose body, as the device writes it, is exactly `bytes` long.
fn call_of(bytes: usize) -> Result<PluginCall> {
    let empty = serde_json::to_vec(&call("save", json!({"text": ""}))?)?.len();
    let text = "x".repeat(bytes.checked_sub(empty).context("too small")?);
    let call = call("save", json!({"text": text}))?;
    assert_eq!(serde_json::to_vec(&call)?.len(), bytes);
    Ok(call)
}

async fn outcome(hub: &NoteHub, plugin: &str, call: &PluginCall) -> Result<Value> {
    let outcome =
        tokio::time::timeout(TEST_WAIT, orbit_daemon::hub_plugin::call(hub, plugin, call))
            .await
            .context("the device did not answer the call")?;
    Ok(serde_json::to_value(outcome)?)
}

/// The fake hub's own end, bounded: a device that never sent the call
/// fails the test instead of leaving it waiting.
async fn served<T>(server: tokio::task::JoinHandle<Result<T>>) -> Result<T> {
    tokio::time::timeout(TEST_WAIT, server)
        .await
        .context("the hub never received the call")??
}

async fn never_connected(listener: &TcpListener) -> bool {
    tokio::time::timeout(NO_CONNECTION_WAIT, listener.accept())
        .await
        .is_err()
}

/// One HTTP request, read whole: its request line, headers and body.
async fn read_request(socket: &mut TcpStream) -> Result<(String, Vec<String>, Vec<u8>)> {
    let mut raw = Vec::new();
    let mut piece = [0_u8; 65_536];
    let head_end = loop {
        let read = socket.read(&mut piece).await?;
        anyhow::ensure!(read > 0, "the request ended before its headers");
        raw.extend_from_slice(&piece[..read]);
        if let Some(end) = raw.windows(4).position(|window| window == b"\r\n\r\n") {
            break end;
        }
    };
    let head = String::from_utf8(raw[..head_end].to_vec())?;
    let mut lines = head.split("\r\n").map(str::to_owned);
    let request_line = lines.next().context("no request line")?;
    let headers: Vec<String> = lines.collect();
    let length: usize = headers
        .iter()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse().ok())?
        })
        .context("no content-length")?;
    let mut body = raw[head_end + 4..].to_vec();
    while body.len() < length {
        let read = socket.read(&mut piece).await?;
        anyhow::ensure!(read > 0, "the request body was cut");
        body.extend_from_slice(&piece[..read]);
    }
    Ok((request_line, headers, body))
}

async fn answer(socket: &mut TcpStream, status: &str, body: &str) -> Result<()> {
    socket
        .write_all(
            format!(
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .as_bytes(),
        )
        .await?;
    Ok(())
}

#[tokio::test]
async fn a_call_over_its_limit_never_reaches_the_hub() -> Result<()> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let hub = hub_at(&listener).await?;
    let wire = outcome(&hub, "vault", &call_of(PLUGIN_CALL_MAX_BYTES + 1)?).await?;
    assert_eq!(
        wire,
        json!({"outcome": "failed", "refusal": {
            "code": "plugin_call_too_large",
            "bytes": PLUGIN_CALL_MAX_BYTES + 1,
            "max_bytes": PLUGIN_CALL_MAX_BYTES,
            "retry": {"kind": "after_fix"},
        }})
    );
    assert!(
        never_connected(&listener).await,
        "the refused call reached the hub"
    );
    Ok(())
}

#[tokio::test]
async fn a_call_at_its_limit_is_sent_once_as_written() -> Result<()> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let hub = hub_at(&listener).await?;
    let sent = call_of(PLUGIN_CALL_MAX_BYTES)?;
    let expected = serde_json::to_value(&sent)?;
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await?;
        let request = read_request(&mut socket).await?;
        answer(
            &mut socket,
            "200 OK",
            r#"{"value":{"saved":true,"count":3}}"#,
        )
        .await?;
        // Nothing comes again: the device never repeats a call.
        let again = tokio::time::timeout(NO_CONNECTION_WAIT, listener.accept()).await;
        Result::<_>::Ok((request, again.is_err()))
    });
    let wire = outcome(&hub, "vault", &sent).await?;
    let ((request_line, headers, body), once) = served(server).await?;
    assert_eq!(request_line, "POST /v1/plugins/vault/call HTTP/1.1");
    assert!(
        headers
            .iter()
            .any(|line| line.eq_ignore_ascii_case("authorization: Bearer synthetic-device-token")),
        "{headers:?}"
    );
    assert_eq!(body.len(), PLUGIN_CALL_MAX_BYTES);
    assert_eq!(serde_json::from_slice::<Value>(&body)?, expected);
    assert_eq!(
        wire,
        json!({"outcome": "answered", "value": {"saved": true, "count": 3}})
    );
    assert!(once, "the device sent the call again");
    Ok(())
}

#[tokio::test]
async fn a_malformed_plugin_id_never_reaches_the_hub() -> Result<()> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let hub = hub_at(&listener).await?;
    let too_long = "a".repeat(33);
    for plugin in [
        "",
        too_long.as_str(),
        "Vault",
        "reading-list",
        "a/b",
        "a%2Fb",
        "notas_é",
        " vault",
        "vault\n",
    ] {
        let wire = outcome(&hub, plugin, &call("config", json!({}))?).await?;
        assert_eq!(
            wire,
            json!({"outcome": "failed", "refusal": {
                "code": "invalid_parameter",
                "parameter": "identifier",
                "expected": {"kind": "wire_type", "name": "plugin_id"},
                "retry": {"kind": "after_fix"},
            }}),
            "{plugin:?}"
        );
    }
    assert!(
        never_connected(&listener).await,
        "a malformed id reached the hub"
    );
    Ok(())
}

#[tokio::test]
async fn the_longest_well_formed_id_is_sent() -> Result<()> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let hub = hub_at(&listener).await?;
    let plugin = format!("{}_9", "z".repeat(30));
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await?;
        let (request_line, _, _) = read_request(&mut socket).await?;
        answer(&mut socket, "200 OK", r#"{"value":null}"#).await?;
        Result::<_>::Ok(request_line)
    });
    let wire = outcome(&hub, &plugin, &call("config", json!({}))?).await?;
    assert_eq!(
        served(server).await?,
        format!("POST /v1/plugins/{plugin}/call HTTP/1.1")
    );
    assert_eq!(wire, json!({"outcome": "answered", "value": null}));
    Ok(())
}

#[tokio::test]
async fn a_cut_answer_is_named_with_what_had_arrived() -> Result<()> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let hub = hub_at(&listener).await?;
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await?;
        read_request(&mut socket).await?;
        // Promises 100 bytes, sends 11 and closes.
        socket
            .write_all(
                b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 100\r\n\r\n{\"value\":[1",
            )
            .await?;
        socket.shutdown().await?;
        Result::<()>::Ok(())
    });
    let wire = outcome(&hub, "vault", &call("config", json!({}))?).await?;
    served(server).await?;
    assert_eq!(
        wire,
        json!({"outcome": "failed", "refusal": {
            "code": "hub_reply_invalid",
            "field": "body",
            "received_bytes": 11,
            "retry": {"kind": "check_outcome"},
        }})
    );
    Ok(())
}
