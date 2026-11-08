//! The engine built without its control client (`--no-default-features`):
//! what needs that client is not the device's to answer.
//!
//! The engine runs in this process (`run_embedded`) with a helper link held in
//! memory and a synthetic hub on a loopback WebSocket, as
//! `device_capabilities.rs` starts it. The permissions are all granted, so
//! nothing but the missing client can refuse an order: what the assistant
//! asks of the screen or of the computer's control is refused as a device
//! that does not handle it, naming the device and what was asked, and never
//! as an engine that is unavailable. What the helper answers on its own stays
//! its to answer.
//!
//! The whole file is compiled out when the client is built in: the gate runs
//! it with `cargo test -p orbit-daemon --no-default-features --test
//! engine_without_control`, and only then does it run.
#![cfg(not(feature = "control"))]

use anyhow::{Context, Result, bail, ensure};
use futures_util::{FutureExt, SinkExt, StreamExt, future::BoxFuture};
use orbit_core::protocol::Command;
use orbit_daemon::{
    config::Paths,
    daemon, ipc,
    platform::{HelperLink, HelperPipes, OwnerName, Platform, Secrets, StopSignals},
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncWrite, AsyncWriteExt, BufReader, DuplexStream},
    net::TcpListener,
    sync::{mpsc, oneshot},
};
use tokio_tungstenite::tungstenite::Message;

/// What a cold engine gets to reach each step: the bound only stops a hang,
/// so a broken engine fails here instead of holding the gate.
const BUDGET: Duration = Duration::from_secs(60);

/// How long the engine is left running after its asks when a test has to see
/// that it did nothing on its own: three of the main loop's one-second
/// maintenance ticks, which is where an engine that wrongly tries to start
/// would try. A wait only for the passing case: a wrong engine fails as soon
/// as its log says so.
const NOTHING_HAPPENS_FOR: Duration = Duration::from_secs(3);

/// How long a request waits before it is asked again while the engine has
/// not yet read what its helper reported.
const NOT_YET_PAUSE: Duration = Duration::from_millis(50);

/// Room in each direction of the in-memory link, far above one frame.
const LINK_BYTES: usize = 256 * 1024;

const DEVICE_ID: &str = "00000000-0000-4000-8000-00000000d1ce";
const DEVICE_TOKEN: &str = "synthetic-device-token";

/// What the engine answers while it has not yet read its helper's report:
/// the same request is asked again, never taken as the answer.
const NOT_YET: [&str; 3] = ["permission_unknown", "helper_not_running", "recovering"];

/// One engine at a time in this process: each test starts its own.
static ONE_ENGINE: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

struct MemoryLink {
    opened: mpsc::UnboundedSender<DuplexStream>,
}

impl HelperLink for MemoryLink {
    fn open(&mut self) -> Result<HelperPipes> {
        let (engine, helper) = tokio::io::duplex(LINK_BYTES);
        self.opened
            .send(helper)
            .map_err(|_| anyhow::anyhow!("the test's side of the link is gone"))?;
        let (reader, writer) = tokio::io::split(engine);
        Ok(HelperPipes {
            reader: Box::new(reader),
            writer: Box::new(writer),
            process: None,
        })
    }
}

struct MemorySecrets {
    saved: Mutex<BTreeMap<String, String>>,
}

impl MemorySecrets {
    fn paired() -> Self {
        Self {
            saved: Mutex::new(BTreeMap::from([(
                "device".to_owned(),
                DEVICE_TOKEN.to_owned(),
            )])),
        }
    }
}

impl Secrets for MemorySecrets {
    fn get(&self, account: &str) -> Result<Option<String>> {
        let saved = self
            .saved
            .lock()
            .map_err(|_| anyhow::anyhow!("secrets poisoned"))?;
        Ok(saved.get(account).cloned())
    }

    fn set(&self, account: &str, value: &str) -> Result<()> {
        self.saved
            .lock()
            .map_err(|_| anyhow::anyhow!("secrets poisoned"))?
            .insert(account.to_owned(), value.to_owned());
        Ok(())
    }
}

struct NoSignals;

impl StopSignals for NoSignals {
    fn next(&mut self) -> BoxFuture<'_, &'static str> {
        std::future::pending().boxed()
    }
}

struct FixedOwner;

impl OwnerName for FixedOwner {
    fn read(&self) -> BoxFuture<'static, String> {
        async { "Ada Prueba".to_owned() }.boxed()
    }
}

fn permissions() -> Value {
    json!({"microphone":"authorized","screen":true,"accessibility":true,
        "calendars":"full_access"})
}

fn ready_event(declared: &Value) -> Result<Value> {
    let mut event: Value = serde_json::from_str(include_str!(
        "../../../contracts/fixtures/native/events/ready.json"
    ))?;
    event["selected_text_available"] = json!(false);
    for (name, value) in declared
        .as_object()
        .context("what a device declares is an object")?
    {
        event[name] = value.clone();
    }
    Ok(event)
}

fn state() -> Result<Value> {
    let mut state: Value = serde_json::from_str(include_str!(
        "../../../contracts/fixtures/native/results/hello.json"
    ))?;
    state["calendar"]["authorization"] = json!("full_access");
    state["permissions"] = permissions();
    Ok(state)
}

fn answer(request: &Value) -> Result<Value> {
    let id = &request["id"];
    let result = match request["command"].as_str() {
        Some("hello" | "heartbeat" | "status") => state()?,
        Some("permissions") => permissions(),
        Some("shutdown") => json!({"drain_complete": true}),
        Some(name @ ("privacy_stop" | "capture_stop")) => {
            let mut result = state()?;
            result["privacy_stopped"] = json!(name == "privacy_stop");
            result["drain_complete"] = json!(true);
            result
        }
        Some("assistant_folders") => json!({"folders": []}),
        Some("screen_sources") => {
            return Ok(json!({"v":1,"event":"response","id":id,"ok":false,
                "error":{"code":"screen_permission_required",
                    "message":"Pantalla sintética sin permiso."}}));
        }
        _ => json!({}),
    };
    Ok(json!({"v":1,"event":"response","id":id,"ok":true,"result":result}))
}

async fn send(writer: &mut (impl AsyncWrite + Unpin), frame: &Value) -> Result<()> {
    let mut bytes = serde_json::to_vec(frame)?;
    bytes.push(b'\n');
    writer.write_all(&bytes).await?;
    writer.flush().await?;
    Ok(())
}

async fn helper_peer(
    link: DuplexStream,
    declared: Value,
    seen: mpsc::UnboundedSender<String>,
) -> Result<()> {
    let (reader, mut writer) = tokio::io::split(link);
    let mut lines = BufReader::new(reader).lines();
    while let Some(line) = lines.next_line().await? {
        let request: Value = serde_json::from_str(&line)?;
        let name = request["command"]
            .as_str()
            .context("a frame from the engine without a command")?
            .to_owned();
        send(&mut writer, &answer(&request)?).await?;
        if name == "hello" {
            send(&mut writer, &ready_event(&declared)?).await?;
        }
        let _ = seen.send(name);
    }
    Ok(())
}

async fn hub_peer(
    listener: TcpListener,
    connected: oneshot::Sender<()>,
    mut outgoing: mpsc::Receiver<Value>,
    replies: mpsc::Sender<Value>,
) -> Result<()> {
    let (tcp, _) = listener.accept().await?;
    let mut socket = tokio_tungstenite::accept_async(tcp).await?;
    let ready = json!({"v":1,"event":"ready","data":{"device_id":DEVICE_ID,"hub_protocol":1}});
    socket.send(Message::Text(ready.to_string().into())).await?;
    let _ = connected.send(());
    loop {
        tokio::select! {
            Some(frame) = outgoing.recv() => {
                socket.send(Message::Text(frame.to_string().into())).await?;
            }
            message = socket.next() => match message {
                Some(Ok(Message::Text(text))) => {
                    let frame: Value = serde_json::from_str(&text)?;
                    if frame["method"] == "device.reply" {
                        replies.send(frame["params"].clone()).await?;
                    }
                }
                Some(Ok(Message::Close(_)) | Err(_)) | None => return Ok(()),
                Some(Ok(_)) => {}
            }
        }
    }
}

struct Ask {
    capability: &'static str,
    parameters: Value,
}

fn ask(capability: &'static str, parameters: Value) -> Ask {
    Ask {
        capability,
        parameters,
    }
}

struct Run {
    replies: Vec<Value>,
    orders: Vec<String>,
    log: String,
}

impl Run {
    fn asked_the_helper(&self, order: &str) -> bool {
        self.orders.iter().any(|seen| seen == order)
    }
}

async fn reply_to(replies: &mut mpsc::Receiver<Value>, id: &str) -> Result<Value> {
    tokio::time::timeout(BUDGET, async {
        while let Some(reply) = replies.recv().await {
            if reply["request_id"] == id {
                return Ok(reply);
            }
        }
        bail!("the hub stopped before the reply to {id}")
    })
    .await
    .with_context(|| format!("no reply to {id} within {BUDGET:?}"))?
}

async fn settled_reply(
    outgoing: &mpsc::Sender<Value>,
    replies: &mut mpsc::Receiver<Value>,
    ask: &Ask,
    serial: &mut u32,
) -> Result<Value> {
    let started = tokio::time::Instant::now();
    loop {
        *serial += 1;
        let id = format!("{}-{serial}", ask.capability);
        outgoing
            .send(json!({"v":1,"event":"device.request","data":{
                "request_id":id,"capability":ask.capability,"parameters":ask.parameters}}))
            .await?;
        let reply = reply_to(replies, &id).await?;
        let code = reply["refusal"]["code"].as_str().unwrap_or_default();
        if !NOT_YET.contains(&code) {
            return Ok(reply);
        }
        ensure!(
            started.elapsed() < BUDGET,
            "{} was still answered «{code}» after {BUDGET:?}: {reply}",
            ask.capability
        );
        tokio::time::sleep(NOT_YET_PAUSE).await;
    }
}

/// Starts an engine paired with a synthetic hub whose helper declares
/// `declared`, asks each of `asks` once it has read its helper, leaves it
/// running for `linger` (a test of something it must not do by itself needs
/// to give it the time) and closes it.
async fn run_for(declared: Value, asks: Vec<Ask>, linger: Duration) -> Result<Run> {
    let local = tokio::task::LocalSet::new();
    local
        .run_until(run_on_this_thread(declared, asks, linger))
        .await
}

async fn run(declared: Value, asks: Vec<Ask>) -> Result<Run> {
    run_for(declared, asks, Duration::ZERO).await
}

async fn run_on_this_thread(declared: Value, asks: Vec<Ask>, linger: Duration) -> Result<Run> {
    let directory = tempfile::tempdir()?;
    let root = directory.path().join("orbit");
    let paths = Paths::resolve(Some(root.clone()))?;
    paths.initialize()?;
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let mut settings = paths.load_settings()?;
    settings.hub_url = Some(format!(
        "http://127.0.0.1:{}",
        listener.local_addr()?.port()
    ));
    settings.device_id = Some(DEVICE_ID.into());
    paths.save_settings(&settings)?;

    let (connected, hub_connected) = oneshot::channel();
    let (outgoing, hub_outgoing) = mpsc::channel(4);
    let (hub_replies, mut replies) = mpsc::channel(16);
    let hub = tokio::spawn(hub_peer(listener, connected, hub_outgoing, hub_replies));

    let (opened_tx, mut opened) = mpsc::unbounded_channel();
    let platform = Platform {
        helper: Some(Box::new(MemoryLink { opened: opened_tx })),
        secrets: Arc::new(MemorySecrets::paired()),
        signals: Box::new(NoSignals),
        owner: Arc::new(FixedOwner),
    };
    let (handed, client) = oneshot::channel();
    let (ended, end) = tokio::sync::watch::channel(None);
    let engine = tokio::task::spawn_local(async move {
        let result = daemon::run_embedded(paths, true, platform, handed, end).await;
        ended.send_replace(Some(match &result {
            Ok(()) => ipc::DaemonEnded::Closed,
            Err(error) => ipc::DaemonEnded::Failed(format!("{error:#}")),
        }));
        result
    });
    let client = tokio::time::timeout(BUDGET, client)
        .await
        .context("the engine never handed its client")??;
    let link = tokio::time::timeout(BUDGET, opened.recv())
        .await
        .context("the engine never opened its helper link")?
        .context("the link was dropped unopened")?;
    let (seen_tx, mut seen) = mpsc::unbounded_channel();
    let peer = tokio::task::spawn_local(helper_peer(link, declared, seen_tx));

    // `status` is what the engine asks a helper whose `ready` it has read.
    let mut orders = Vec::new();
    tokio::time::timeout(BUDGET, async {
        while let Some(name) = seen.recv().await {
            let read_ready = name == "status";
            orders.push(name);
            if read_ready {
                return Ok(());
            }
        }
        bail!("the helper's side of the link ended before the engine read its ready")
    })
    .await
    .context("the engine never asked its helper for status")??;
    tokio::time::timeout(BUDGET, async {
        loop {
            let status = client
                .request_command(Command::Status)
                .await?
                .status
                .context("status absent")?;
            if status.recovery == orbit_core::RecoveryStatus::Ready {
                return Ok::<_, anyhow::Error>(());
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .context("the engine never answered Status as ready")??;
    tokio::time::timeout(BUDGET, hub_connected)
        .await
        .context("the engine never connected to its hub")??;

    let mut answered = Vec::new();
    let mut serial = 0;
    for ask in &asks {
        answered.push(settled_reply(&outgoing, &mut replies, ask, &mut serial).await?);
    }
    tokio::time::sleep(linger).await;

    client.request_command(Command::Shutdown).await?;
    tokio::time::timeout(BUDGET, engine)
        .await
        .context("the engine did not end after Shutdown")???;
    tokio::time::timeout(BUDGET, peer)
        .await
        .context("the engine left its end of the link open")???;
    while let Ok(name) = seen.try_recv() {
        orders.push(name);
    }
    drop(outgoing);
    hub.abort();
    let log = std::fs::read_to_string(root.join("logs/daemon.log")).unwrap_or_default();
    Ok(Run {
        replies: answered,
        orders,
        log,
    })
}

/// The helper's orders a screen or control order would start: the window
/// listing and the capture. The computer's control has none: it is the engine.
const SCREEN_ORDERS: [&str; 2] = ["screen_sources", "screenshot"];

fn refused(kind: &str, capability: &str, group: &str) -> Value {
    json!({"code": "device_capability_missing", "kind": kind, "capability": capability,
        "group": group, "retry": {"kind": "never"}})
}

/// Every wrong refusal, not only the first: `(asked, reply, expected)`.
fn wrong_refusals<'a>(
    run: &Run,
    expected: impl IntoIterator<Item = (&'a str, Value)>,
) -> Vec<String> {
    run.replies
        .iter()
        .zip(expected)
        .filter(|(reply, (_, want))| reply["refusal"] != *want)
        .map(|(reply, (asked, want))| {
            format!(
                "asked {asked}, the engine answered {reply}, not {want}; it wrote {:?}",
                run.orders
            )
        })
        .collect()
}

fn no_screen_order_reached_the_helper(run: &Run) -> Vec<String> {
    SCREEN_ORDERS
        .iter()
        .filter(|order| run.asked_the_helper(order))
        .map(|order| format!("the helper was sent {order}; it wrote {:?}", run.orders))
        .collect()
}

/// A tool call the engine would validate, were there one.
fn a_click() -> Value {
    json!({"action": {"tool": "click", "arguments": {"x": 1, "y": 2}}})
}

#[tokio::test(flavor = "current_thread")]
async fn what_needs_the_engine_is_refused_as_a_device_that_does_not_handle_it() -> Result<()> {
    let _one = ONE_ENGINE.lock().await;
    // Today's helper says nothing of itself: a Mac that handles every group.
    // The client's absence is what decides, whatever the helper would do.
    let ran = run(
        json!({}),
        vec![
            ask("screenshot", json!({})),
            ask("screenshot", json!({"full_screen": true})),
            ask("screen_sources", json!({})),
            ask("control_tools", json!({})),
            ask("perform_action", a_click()),
        ],
    )
    .await?;
    let mut wrong = wrong_refusals(
        &ran,
        [
            (
                "a window screenshot",
                refused("mac", "screenshot", "screen"),
            ),
            ("a full screen", refused("mac", "screenshot", "screen")),
            ("the windows", refused("mac", "screen_sources", "screen")),
            ("the tools", refused("mac", "control_tools", "control")),
            ("an action", refused("mac", "perform_action", "control")),
        ],
    );
    wrong.extend(no_screen_order_reached_the_helper(&ran));
    ensure!(wrong.is_empty(), "{}", wrong.join("\n"));
    Ok(())
}

#[tokio::test(flavor = "current_thread")]
async fn a_device_that_says_it_handles_the_screen_is_refused_it_with_its_own_kind() -> Result<()> {
    let _one = ONE_ENGINE.lock().await;
    // What the device declares does not put back what the build left out,
    // and the device named is the one that asked.
    let ran = run(
        json!({"kind": "phone", "capabilities": ["screen", "control", "calendar"]}),
        vec![
            ask("screenshot", json!({})),
            ask("screen_sources", json!({})),
            ask("control_tools", json!({})),
            ask("perform_action", a_click()),
        ],
    )
    .await?;
    let mut wrong = wrong_refusals(
        &ran,
        [
            ("a screenshot", refused("phone", "screenshot", "screen")),
            ("the windows", refused("phone", "screen_sources", "screen")),
            ("the tools", refused("phone", "control_tools", "control")),
            ("an action", refused("phone", "perform_action", "control")),
        ],
    );
    wrong.extend(no_screen_order_reached_the_helper(&ran));
    ensure!(wrong.is_empty(), "{}", wrong.join("\n"));
    Ok(())
}

#[tokio::test(flavor = "current_thread")]
async fn what_the_helper_answers_on_its_own_is_still_asked_of_it() -> Result<()> {
    let _one = ONE_ENGINE.lock().await;
    let a_day = json!({"start_ms": 1_800_000_000_000_i64, "end_ms": 1_800_086_400_000_i64});
    let ran = run(
        json!({}),
        vec![ask("selected_text", json!({})), ask("calendar_read", a_day)],
    )
    .await?;
    let mut wrong = Vec::new();
    for (reply, (asked, order)) in ran.replies.iter().zip([
        ("the selected text", "screen_sources"),
        ("the calendar", "calendar_snapshot"),
    ]) {
        if reply["refusal"]["code"] == "device_capability_missing" {
            wrong.push(format!("{asked} was refused as a device: {reply}"));
        }
        if !ran.asked_the_helper(order) {
            wrong.push(format!(
                "{asked} did not reach the helper's {order}; the engine wrote {:?} and answered {reply}",
                ran.orders
            ));
        }
    }
    ensure!(wrong.is_empty(), "{}", wrong.join("\n"));
    Ok(())
}

#[tokio::test(flavor = "current_thread")]
async fn no_engine_is_started_though_every_permission_is_granted() -> Result<()> {
    let _one = ONE_ENGINE.lock().await;
    // A build without the engine has nothing to start: with every permission
    // granted it must not try, fail and retry every few seconds.
    let ran = run_for(
        json!({}),
        vec![ask("control_tools", json!({}))],
        NOTHING_HAPPENS_FOR,
    )
    .await?;
    ensure!(
        !ran.log.contains("engine_failed"),
        "the engine tried to start its control client without having one: {}",
        ran.log
            .lines()
            .filter(|line| line.contains("engine_failed"))
            .collect::<Vec<_>>()
            .join("\n")
    );
    Ok(())
}
