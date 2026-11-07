//! What a device says it handles, and what the engine then asks of it.
//!
//! The engine runs in this process (`run_embedded`) with a helper link held
//! in memory, as `embedded_platform.rs` starts it, and a synthetic hub on a
//! loopback WebSocket that sends `device.request` frames, the way the hub
//! relays what an assistant asks, and records each `device.reply`. The
//! helper's side of the link is this test: after `hello` it says `ready` with
//! what the device declares, answers every order and reports each order's
//! name.
//!
//! A helper whose `ready` says nothing is a Mac that handles every group of
//! orders, as the helper bundled today. A device that lists its groups is
//! asked nothing outside them: what the assistant asks of a group it does not
//! list is refused with that cause, the device's kind, the order asked and
//! its group, and never reaches the helper. The permissions are synthetic:
//! the screen is granted, so only what the device declares can stop a screen
//! order, and Accessibility is not, so what the device declares must be
//! decided before the permission is.
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

/// What a cold engine gets under the whole suite to reach each step; the
/// bound only stops a hang (`embedded_platform.rs`).
const BUDGET: Duration = Duration::from_secs(60);

/// Room in each direction of the in-memory link, far above one frame.
const LINK_BYTES: usize = 256 * 1024;

/// How long a request waits before it is asked again while the engine has
/// not yet read what its helper reported (a permission still unknown).
const NOT_YET_PAUSE: Duration = Duration::from_millis(50);

const DEVICE_ID: &str = "00000000-0000-4000-8000-00000000d1ce";
const DEVICE_TOKEN: &str = "synthetic-device-token";

/// What the engine answers while it has not yet read its helper's report:
/// the same request is asked again, never taken as the answer.
const NOT_YET: [&str; 3] = ["permission_unknown", "helper_not_running", "recovering"];

/// One engine at a time in this process: each test starts its own.
static ONE_ENGINE: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// A helper link in memory: each `open` hands the engine one end of a fresh
/// duplex pipe and the test the other.
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

/// Secrets in a map, with the paired device's token already in it.
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

/// No one outside ever asks this engine to stop: it closes by `Shutdown`.
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

/// Today's `ready`, with what the device declares (`kind`, `capabilities`)
/// added to it; `{}` adds nothing.
fn ready_event(declared: &Value) -> Result<Value> {
    let mut event: Value = serde_json::from_str(include_str!(
        "../../../contracts/fixtures/native/events/ready.json"
    ))?;
    event["selected_text_available"] = json!(false);
    let fields = declared
        .as_object()
        .context("what a device declares is an object")?;
    for (name, value) in fields {
        event[name] = value.clone();
    }
    Ok(event)
}

/// Synthetic permissions: the screen granted, Accessibility not, the
/// calendar fully granted.
fn permissions() -> Value {
    json!({"microphone":"authorized","screen":true,"accessibility":false,
        "calendars":"full_access"})
}

fn state() -> Result<Value> {
    let mut state: Value = serde_json::from_str(include_str!(
        "../../../contracts/fixtures/native/results/hello.json"
    ))?;
    state["calendar"]["authorization"] = json!("full_access");
    state["permissions"] = permissions();
    Ok(state)
}

/// The helper's answer to one order. The window listing is refused with the
/// helper's own code, which is its answer; anything else not named here is
/// an empty success.
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

/// The helper's side of one link: answers every order, says `ready` (with
/// what the device declares) after answering `hello`, reports each order's
/// name, and ends when the engine closes its end.
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

/// The hub's side of the socket: `ready` for this device, then every frame
/// the test asks it to send; each `device.reply` comes back.
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

/// One ask of the assistant: the order the hub relays and its parameters.
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

/// What one engine was asked and did: each ask's final reply, in order, and
/// the name of every order it wrote to its helper while it ran.
struct Run {
    replies: Vec<Value>,
    orders: Vec<String>,
}

impl Run {
    fn asked_the_helper(&self, order: &str) -> bool {
        self.orders.iter().any(|seen| seen == order)
    }
}

/// The reply to `id`, skipping any other.
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

/// Sends `ask` until the engine answers it with something other than «not
/// yet» (it had not read its helper's report), and returns that answer.
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
/// `declared`, asks each of `asks` once it has read its helper, and closes it.
async fn run(declared: Value, asks: Vec<Ask>) -> Result<Run> {
    let local = tokio::task::LocalSet::new();
    local.run_until(run_on_this_thread(declared, asks)).await
}

async fn run_on_this_thread(declared: Value, asks: Vec<Ask>) -> Result<Run> {
    let directory = tempfile::tempdir()?;
    let paths = Paths::resolve(Some(directory.path().join("orbit")))?;
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
    Ok(Run {
        replies: answered,
        orders,
    })
}

/// The helper's orders an ask of each group would start (`native-v1.md`).
/// The computer's control is the engine's, not the helper's: none.
fn helper_orders_of(group: &str) -> &'static [&'static str] {
    match group {
        "screen" => &["screen_sources", "screenshot"],
        "selected_text" => &["selected_text"],
        "folders" => &[
            "assistant_folders",
            "assistant_files_list",
            "assistant_files_search",
            "assistant_files_read",
        ],
        "calendar" => &[
            "calendar_sources",
            "calendar_snapshot",
            "calendar_save",
            "calendar_delete",
        ],
        _ => &[],
    }
}

/// A day of the calendar, asked as the assistant asks it.
fn a_day() -> Value {
    json!({"start_ms": 1_800_000_000_000_i64, "end_ms": 1_800_086_400_000_i64})
}

#[tokio::test(flavor = "current_thread")]
async fn what_a_device_does_not_list_is_refused_with_its_cause_and_never_reaches_its_helper()
-> Result<()> {
    let _one = ONE_ENGINE.lock().await;
    let cases = [
        (
            json!({"kind": "phone", "capabilities": ["calendar"]}),
            "phone",
            vec![
                (
                    ask("screenshot", json!({"application": "Editor"})),
                    "screen",
                ),
                (ask("screen_sources", json!({})), "screen"),
                (ask("selected_text", json!({})), "selected_text"),
                (ask("files_list", json!({})), "folders"),
                (ask("control_tools", json!({})), "control"),
            ],
        ),
        // An empty list is a device that handles nothing, not one that said
        // nothing.
        (
            json!({"kind": "phone", "capabilities": []}),
            "phone",
            vec![
                (ask("screenshot", json!({})), "screen"),
                (ask("calendar_read", a_day()), "calendar"),
            ],
        ),
        // Without `kind`, the device is a Mac, whatever it lists.
        (
            json!({"capabilities": ["calendar"]}),
            "mac",
            vec![(ask("screenshot", json!({})), "screen")],
        ),
    ];
    // Every case is run and every wrong answer said, not only the first.
    let mut wrong = Vec::new();
    for (declared, kind, asks) in cases {
        let (asks, groups): (Vec<Ask>, Vec<&str>) = asks.into_iter().unzip();
        let capabilities: Vec<&str> = asks.iter().map(|ask| ask.capability).collect();
        let ran = run(declared.clone(), asks).await?;
        for ((reply, capability), group) in ran.replies.iter().zip(&capabilities).zip(&groups) {
            let expected = json!({"code": "device_capability_missing", "kind": kind,
                "capability": capability, "group": group, "retry": {"kind": "never"}});
            if reply["refusal"] != expected {
                wrong.push(format!(
                    "a device that declared {declared} was asked {capability} and answered \
                     {reply}, not {expected}"
                ));
            }
        }
        // Nothing of a group it does not list, whoever started it: the
        // assistant or the engine on its own.
        let missing: std::collections::BTreeSet<&str> = groups.iter().copied().collect();
        for group in missing {
            for order in helper_orders_of(group) {
                if ran.asked_the_helper(order) {
                    wrong.push(format!(
                        "a device that declared {declared} was sent {order}, of a group it \
                         does not list ({group}); the engine wrote {:?}",
                        ran.orders
                    ));
                }
            }
        }
    }
    ensure!(wrong.is_empty(), "{}", wrong.join("\n"));
    Ok(())
}

#[tokio::test(flavor = "current_thread")]
async fn a_device_that_lists_no_groups_is_asked_as_todays_mac_is() -> Result<()> {
    let _one = ONE_ENGINE.lock().await;
    // Today's helper says neither; a phone that lists nothing handles every
    // group, as the hub reads a device that says nothing at pairing.
    for declared in [json!({}), json!({"kind": "phone"})] {
        let ran = run(
            declared.clone(),
            vec![ask("screenshot", json!({})), ask("calendar_read", a_day())],
        )
        .await?;
        for (reply, order) in ran
            .replies
            .iter()
            .zip(["screen_sources", "calendar_snapshot"])
        {
            ensure!(
                reply["refusal"]["code"] != "device_capability_missing",
                "a device that declared {declared} was refused what starts with {order}: {reply}"
            );
            ensure!(
                ran.asked_the_helper(order),
                "a device that declared {declared} was not sent {order}; the engine wrote {:?} \
                 and answered {reply}",
                ran.orders
            );
        }
    }
    Ok(())
}

#[tokio::test(flavor = "current_thread")]
async fn a_phone_is_asked_for_the_groups_it_lists_and_refused_the_others() -> Result<()> {
    let _one = ONE_ENGINE.lock().await;
    // The screen is not the selected text: an agent's access groups put both
    // under «screen» (Settings › Assistants), a device's groups do not.
    let declared = json!({"kind": "phone", "capabilities": ["screen"]});
    let ran = run(
        declared.clone(),
        vec![
            ask("screenshot", json!({})),
            ask("selected_text", json!({})),
        ],
    )
    .await?;
    let (screen, text) = (&ran.replies[0], &ran.replies[1]);
    let mut wrong = Vec::new();
    if screen["refusal"]["code"] == "device_capability_missing" {
        wrong.push(format!(
            "a phone that lists the screen was refused it: {screen}"
        ));
    }
    if !ran.asked_the_helper("screen_sources") {
        wrong.push(format!(
            "a phone that lists the screen was not asked for its windows; the engine wrote {:?}",
            ran.orders
        ));
    }
    let refused = json!({"code": "device_capability_missing", "kind": "phone",
        "capability": "selected_text", "group": "selected_text", "retry": {"kind": "never"}});
    if text["refusal"] != refused {
        wrong.push(format!(
            "a phone that does not list the selected text answered {text}, not {refused}"
        ));
    }
    // Nor the calendar it does not list, which the engine reads on its own
    // once its helper is ready, without anyone asking.
    for order in helper_orders_of("selected_text")
        .iter()
        .chain(helper_orders_of("calendar"))
    {
        if ran.asked_the_helper(order) {
            wrong.push(format!(
                "a phone that lists only the screen was sent {order}; the engine wrote {:?}",
                ran.orders
            ));
        }
    }
    ensure!(wrong.is_empty(), "{}", wrong.join("\n"));
    Ok(())
}
