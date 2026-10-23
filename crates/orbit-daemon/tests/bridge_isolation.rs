//! A file read for Deimos that the helper holds for 15 s holds neither the
//! daemon's loop nor the app's orders, and a second request from Deimos is
//! answered at once while it waits.
//!
//! This runs the real daemon, its real socket and the production helper
//! launch (`--helper`) against two synthetic peers: a hub on a loopback
//! WebSocket that sends `device.request` frames and records each
//! `device.reply`, and an NDJSON helper that holds the folder resolution a
//! `files_read` starts with for [`HOLD`], as a slow disk or a folder on a
//! network volume would. Meanwhile the app asks the daemon for its state.
//!
//! What it reads is the daemon's own turn instrumentation (`daemon_turn_slow`,
//! `daemon_loop_stalled`), each order's answer time and each reply's. The
//! second request is answered `busy`: the bridge serves one request at a time,
//! and that is the behaviour this measures, not one it changes. Folders,
//! bookmarks and permissions are fixture data, not macOS evidence.
use anyhow::{Context, Result, bail, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use futures_util::{SinkExt, StreamExt};
use orbit_core::{assistant_files::Folder, protocol::Command as IpcCommand};
use orbit_daemon::config::{Paths, set_secret};
use orbit_store::Store;
use serde_json::{Value, json};
use std::{
    path::Path,
    process::Stdio,
    time::{Duration, Instant},
};
use tokio::{
    fs::File,
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::TcpListener,
    sync::{mpsc, oneshot},
};
use tokio_tungstenite::tungstenite::Message;

#[path = "support/helper_relay.rs"]
mod helper_relay;
use helper_relay::{Relay, STRICT_TURN_BOUND, kill_group, tokio_daemon};
#[path = "support/ipc.rs"]
mod ipc;

/// How long the helper takes to resolve the folder: under the daemon's own
/// 20 s bound for a folder request (`HELPER_REQUEST_TIMEOUT_MS`), so the
/// read is still in flight, not refused for its time, while it is held.
const HOLD: Duration = Duration::from_secs(15);

/// A state the daemon reaches as part of starting or of an order: the suite
/// gives a cold daemon this long under load elsewhere (`track_signal_lifecycle.rs`).
const STATE_BUDGET: Duration = Duration::from_secs(30);

const DEVICE_ID: &str = "00000000-0000-4000-8000-0000000b1d6e";
const FOLDER_ID: &str = "folder-proyectos";
const FOLDER_PATH: &str = "/Users/invitada/Proyectos";

async fn send(writer: &mut File, event: Value) -> Result<()> {
    let mut bytes = serde_json::to_vec(&event)?;
    ensure!(bytes.len() < 32_768, "synthetic event budget exceeded");
    bytes.push(b'\n');
    writer.write_all(&bytes).await?;
    writer.flush().await?;
    Ok(())
}

fn state() -> Result<Value> {
    helper_relay::state_result(json!({
        "microphone":"authorized","screen":true,"accessibility":false,
        "calendars":"denied"}))
}

fn folders() -> Value {
    json!({"folders":[{"state":"accessible","id":FOLDER_ID,"path":FOLDER_PATH}]})
}

/// The helper's side of the pipe. The folder resolution a request with a
/// `path` starts with is held for [`HOLD`] and then refused; the check the
/// daemon makes on its own, without a path, is answered at once.
async fn native_peer(
    mut writer: File,
    reader: File,
    held: oneshot::Sender<Instant>,
) -> Result<Option<Duration>> {
    send(&mut writer, helper_relay::ready_event()?).await?;
    let mut lines = BufReader::new(reader).lines();
    let mut held = Some(held);
    let mut hold: Option<(Value, Instant)> = None;
    let mut held_for = None;
    loop {
        let answer_at = hold.as_ref().map(|(_, since)| *since + HOLD);
        tokio::select! {
            () = tokio::time::sleep_until(answer_at.unwrap_or_else(Instant::now).into()),
                if answer_at.is_some() => {
                let Some((request, since)) = hold.take() else { continue };
                held_for = Some(since.elapsed());
                send(&mut writer, json!({"v":1,"event":"response","id":request["id"],
                    "ok":false,"error":{"code":"assistant_folder_unavailable",
                    "message":"La carpeta sintética no se abrió."}})).await?;
            }
            line = lines.next_line() => {
                let Some(line) = line? else { break };
                let request: Value = serde_json::from_str(&line)?;
                let result = match request["command"].as_str() {
                    Some("hello" | "heartbeat" | "status") => state()?,
                    Some("assistant_folders") if request.get("path").is_some() => {
                        let since = Instant::now();
                        if let Some(held) = held.take() {
                            let _ = held.send(since);
                        }
                        hold = Some((request, since));
                        continue;
                    }
                    Some("assistant_folders") => folders(),
                    Some(name @ ("privacy_stop" | "capture_stop")) => {
                        let mut result = state()?;
                        result["privacy_stopped"] = json!(name == "privacy_stop");
                        result["drain_complete"] = json!(true);
                        result
                    }
                    Some("playback_stop") => json!({}),
                    Some("shutdown") => json!({"drain_complete":true}),
                    other => bail!("native action outside this fixture: {other:?}"),
                };
                send(&mut writer, json!({"v":1,"event":"response","id":request["id"],
                    "ok":true,"result":result})).await?;
            }
        }
    }
    Ok(held_for)
}

/// The hub's side of the socket: `ready`, then every frame the test asks it
/// to send; each `device.reply` comes back with the time it arrived.
async fn hub_peer(
    listener: TcpListener,
    mut outgoing: mpsc::Receiver<Value>,
    replies: mpsc::Sender<(Instant, Value)>,
) -> Result<()> {
    let (tcp, _) = listener.accept().await?;
    let mut socket = tokio_tungstenite::accept_async(tcp).await?;
    let ready = json!({"v":1,"event":"ready","data":{"device_id":DEVICE_ID,"hub_protocol":1}});
    socket.send(Message::Text(ready.to_string().into())).await?;
    loop {
        tokio::select! {
            Some(frame) = outgoing.recv() => {
                socket.send(Message::Text(frame.to_string().into())).await?;
            }
            message = socket.next() => match message {
                Some(Ok(Message::Text(text))) => {
                    let frame: Value = serde_json::from_str(&text)?;
                    if frame["method"] == "device.reply" {
                        replies.send((Instant::now(), frame["params"].clone())).await?;
                    }
                }
                Some(Ok(Message::Close(_)) | Err(_)) | None => return Ok(()),
                Some(Ok(_)) => {}
            }
        }
    }
}

fn device_request(id: &str, capability: &str, parameters: Value) -> Value {
    json!({"v":1,"event":"device.request","data":{
        "request_id":id,"capability":capability,"parameters":parameters}})
}

/// A paired Mac with one folder for the assistant, before the daemon starts.
fn paired(path: &Path, port: u16) -> Result<()> {
    let paths = Paths::resolve(Some(path.to_path_buf()))?;
    paths.initialize()?;
    set_secret(&paths, "storage", &STANDARD.encode([0x42; 32]), true)?;
    set_secret(&paths, "device", "synthetic-device-token", true)?;
    let mut store = Store::open(&paths.store, [0x42; 32])?;
    store.save_assistant_folders(&[Folder {
        id: FOLDER_ID.into(),
        path: FOLDER_PATH.into(),
        bookmark_base64: STANDARD.encode(b"synthetic bookmark"),
    }])?;
    drop(store);
    let mut settings = paths.load_settings()?;
    settings.hub_url = Some(format!("http://127.0.0.1:{port}"));
    settings.device_id = Some(DEVICE_ID.into());
    paths.save_settings(&settings)?;
    Ok(())
}

async fn timed(path: &Path, command: IpcCommand) -> Result<Duration> {
    let started = Instant::now();
    ipc::request_async(path, command).await?;
    Ok(started.elapsed())
}

/// The next reply to `id`, and when it came.
async fn reply_to(
    replies: &mut mpsc::Receiver<(Instant, Value)>,
    id: &str,
    budget: Duration,
) -> Result<(Instant, Value)> {
    tokio::time::timeout(budget, async {
        while let Some((at, reply)) = replies.recv().await {
            if reply["request_id"] == id {
                return Ok((at, reply));
            }
        }
        bail!("the hub stopped before the reply to {id}")
    })
    .await
    .with_context(|| format!("no reply to {id} within {budget:?}"))?
}

/// What the daemon logged of its loop while this ran.
fn loop_lines(path: &Path) -> (Vec<Value>, Vec<Value>) {
    let lines = helper_relay::daemon_log(path);
    let pick = |event: &str| {
        lines
            .iter()
            .filter(|line| line["event"] == event)
            .map(|line| line["fields"].clone())
            .collect::<Vec<_>>()
    };
    (pick("daemon_turn_slow"), pick("daemon_loop_stalled"))
}

async fn scenario(path: &Path) -> Result<()> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    paired(path, listener.local_addr()?.port())?;
    let (outgoing, hub_outgoing) = mpsc::channel(4);
    let (hub_replies, mut replies) = mpsc::channel(16);
    let hub = tokio::spawn(hub_peer(listener, hub_outgoing, hub_replies));
    let relay = Relay::create(path, "helper")?;
    let child = tokio_daemon(path, &relay.script, None)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    let group = child.id().context("the daemon has no pid")?;
    let mut cleanup = Cleanup(Some(group));
    let mut child = child;
    let (reader, writer) = tokio::task::spawn_blocking(move || relay.connect()).await??;
    let (held, hold_started) = oneshot::channel();
    let peer = tokio::spawn(native_peer(
        File::from_std(writer),
        File::from_std(reader),
        held,
    ));
    wait_ready(path).await?;

    let asked_at = Instant::now();
    outgoing
        .send(device_request(
            "files-held",
            "files_read",
            json!({"path":format!("{FOLDER_PATH}/notas.txt")}),
        ))
        .await?;
    let since = tokio::time::timeout(STATE_BUDGET, hold_started)
        .await
        .context("the daemon never asked the helper for the folder")??;
    let answered_at = since + HOLD;
    let to_helper = since.duration_since(asked_at);

    // A second request from Deimos while the read is held.
    let second_at = Instant::now();
    outgoing
        .send(device_request(
            "windows-meanwhile",
            "screen_sources",
            json!({}),
        ))
        .await?;
    let (second_replied, second) =
        reply_to(&mut replies, "windows-meanwhile", STRICT_TURN_BOUND).await?;
    let second_took = second_replied.duration_since(second_at);
    ensure!(
        second["refusal"]["code"] == "busy",
        "the second request was not answered busy: {second}"
    );

    let mut orders = Vec::new();
    while Instant::now() + Duration::from_secs(1) < answered_at {
        orders.push(timed(path, IpcCommand::Status).await?);
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    let (files_replied, files) = reply_to(&mut replies, "files-held", HOLD + STATE_BUDGET).await?;
    ensure!(
        files_replied >= answered_at,
        "the file read was answered before the helper resolved its folder: {files}"
    );
    ensure!(files.get("refusal").is_some(), "{files}");

    ipc::request_async(path, IpcCommand::Shutdown).await?;
    let exit = child.wait().await?;
    // Reaped: its group id may be handed on, so it is never signalled again.
    cleanup.0 = None;
    ensure!(exit.success(), "the daemon did not close cleanly");
    let held_for = peer.await??.context("the held folder was never answered")?;
    drop(outgoing);
    hub.abort();

    let slowest = orders.iter().max().copied().unwrap_or_default();
    let (slow, stalled) = loop_lines(path);
    eprintln!(
        "bridge_isolation held_ms={} to_helper_ms={} second_reply_ms={} orders={} \
         slowest_order_ms={} files_reply_after_hold_ms={} slow_turns={} stalls={}",
        held_for.as_millis(),
        to_helper.as_millis(),
        second_took.as_millis(),
        orders.len(),
        slowest.as_millis(),
        files_replied.duration_since(answered_at).as_millis(),
        serde_json::to_string(&slow)?,
        serde_json::to_string(&stalled)?,
    );
    ensure!(held_for >= HOLD, "the folder was held only {held_for:?}");
    ensure!(
        orders.len() >= 30,
        "too few orders to tell: {}",
        orders.len()
    );
    ensure!(
        slowest < STRICT_TURN_BOUND,
        "an order waited {slowest:?} while the file read was held"
    );
    ensure!(
        second_took < STRICT_TURN_BOUND,
        "the second request waited {second_took:?} for its answer"
    );
    helper_relay::assert_no_held_turn(path)?;
    let bridge_named = |fields: &&Value| fields.to_string().contains("hub:");
    ensure!(
        !slow
            .iter()
            .chain(&stalled)
            .any(|fields| bridge_named(&fields)),
        "a loop turn was held by a hub request: {slow:?} {stalled:?}"
    );
    Ok(())
}

/// The daemon has its helper and its hub.
async fn wait_ready(path: &Path) -> Result<()> {
    let started = Instant::now();
    loop {
        if let Ok(state) = ipc::status_async(path).await
            && state["connection"] == "online"
            && state["capabilities"]["native_helper"] == true
        {
            return Ok(());
        }
        if started.elapsed() >= STATE_BUDGET {
            let state = ipc::status_async(path).await?;
            bail!("the daemon never had both its helper and its hub: {state}");
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

/// Failure cleanup only: the group is this test's daemon and its helper,
/// signalled only while the daemon is unreaped and its id still reserved.
struct Cleanup(Option<u32>);

impl Drop for Cleanup {
    fn drop(&mut self) {
        if let Some(group) = self.0 {
            kill_group(group);
        }
    }
}

#[tokio::test]
async fn a_file_read_held_fifteen_seconds_delays_neither_the_loop_nor_orders() -> Result<()> {
    let directory = tempfile::Builder::new()
        .prefix("orbit-bridge-")
        .tempdir_in("/tmp")?;
    tokio::time::timeout(HOLD + Duration::from_secs(90), scenario(directory.path()))
        .await
        .context("the scenario outlived its bound without one of its waits expiring first")??;
    Ok(())
}
