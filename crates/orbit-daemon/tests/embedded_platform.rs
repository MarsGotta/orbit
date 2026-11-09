//! The engine started with everything the machine would give it handed in
//! instead: a helper link held in memory, with no process behind it, the
//! secrets in a map, a stop that never comes and a made-up owner.
//!
//! The helper's side of the link is this test. It sees `hello` as the first
//! frame, says `ready`, and is asked `status` back: the engine reads and
//! writes the link as it does a helper's pipes. The store is opened with the
//! key the engine saved through the given secrets, and no key file of its
//! own appears in the data directory.
use anyhow::{Context, Result, bail, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use futures_util::{FutureExt, future::BoxFuture};
use orbit_core::protocol::Command;
use orbit_daemon::{
    config::Paths,
    daemon, ipc,
    platform::{HelperLink, HelperPipes, OwnerName, Platform, Secrets, StopSignals},
};
use orbit_store::Store;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncWrite, AsyncWriteExt, BufReader, DuplexStream},
    sync::mpsc,
};

/// What a cold engine gets under the whole suite to reach each step; the
/// bound only stops a hang.
const BUDGET: Duration = Duration::from_secs(60);

/// Room in each direction of the in-memory link, far above one frame.
const LINK_BYTES: usize = 256 * 1024;

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

/// Secrets kept in a map the test can read afterwards.
#[derive(Default)]
struct MemorySecrets {
    saved: Mutex<BTreeMap<String, String>>,
}

impl MemorySecrets {
    fn saved(&self, account: &str) -> Option<String> {
        self.saved.lock().ok()?.get(account).cloned()
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

fn ready_event() -> Result<Value> {
    let mut event: Value = serde_json::from_str(include_str!(
        "../../../contracts/fixtures/native/events/ready.json"
    ))?;
    event["selected_text_available"] = json!(false);
    Ok(event)
}

fn hello_result() -> Result<Value> {
    Ok(serde_json::from_str(include_str!(
        "../../../contracts/fixtures/native/results/hello.json"
    ))?)
}

/// The helper's answer to one order, with the shapes the fake helpers of the
/// suite use; anything else is answered with an empty success.
fn answer(request: &Value) -> Result<Value> {
    let result = match request["command"].as_str() {
        Some("hello" | "heartbeat" | "status") => hello_result()?,
        Some("shutdown") => json!({"drain_complete": true}),
        Some(name @ ("privacy_stop" | "capture_stop")) => {
            let mut result = hello_result()?;
            result["privacy_stopped"] = json!(name == "privacy_stop");
            result["drain_complete"] = json!(true);
            result
        }
        Some("assistant_folders") => json!({"folders": []}),
        _ => json!({}),
    };
    Ok(json!({"v":1,"event":"response","id":request["id"],"ok":true,"result":result}))
}

async fn send(writer: &mut (impl AsyncWrite + Unpin), frame: &Value) -> Result<()> {
    let mut bytes = serde_json::to_vec(frame)?;
    bytes.push(b'\n');
    writer.write_all(&bytes).await?;
    writer.flush().await?;
    Ok(())
}

/// The helper's side of one link: answers every order, says `ready` after
/// answering `hello`, reports each order's name, and ends when the engine
/// closes its end.
async fn helper_peer(link: DuplexStream, seen: mpsc::UnboundedSender<String>) -> Result<()> {
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
            send(&mut writer, &ready_event()?).await?;
        }
        let _ = seen.send(name);
    }
    Ok(())
}

async fn next_order(seen: &mut mpsc::UnboundedReceiver<String>) -> Result<String> {
    tokio::time::timeout(BUDGET, seen.recv())
        .await
        .context("the engine wrote nothing more to its helper link")?
        .context("the helper's side of the link ended")
}

#[tokio::test(flavor = "current_thread")]
async fn the_engine_talks_to_a_helper_link_in_memory_and_keeps_its_secrets_where_it_is_told()
-> Result<()> {
    tokio::task::LocalSet::new()
        .run_until(async {
            let directory = tempfile::tempdir()?;
            let paths = Paths::resolve(Some(directory.path().join("orbit")))?;
            let (opened_tx, mut opened) = mpsc::unbounded_channel();
            let secrets = Arc::new(MemorySecrets::default());
            let platform = Platform {
                helper: Some(Box::new(MemoryLink { opened: opened_tx })),
                secrets: secrets.clone(),
                signals: Box::new(NoSignals),
                owner: Arc::new(FixedOwner),
            };
            let (connected, client) = tokio::sync::oneshot::channel();
            let (ended, end) = tokio::sync::watch::channel(None);
            let daemon_paths = paths.clone();
            let engine = tokio::task::spawn_local(async move {
                let result =
                    daemon::run_embedded(daemon_paths, true, platform, connected, end).await;
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
            let peer = tokio::task::spawn_local(helper_peer(link, seen_tx));

            assert_eq!(
                next_order(&mut seen).await?,
                "hello",
                "the first frame on the link must be the engine's hello"
            );
            // `status` is what the engine asks a helper that said `ready`:
            // the link carried the event in and the order out.
            loop {
                if next_order(&mut seen).await? == "status" {
                    break;
                }
            }

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

            client.request_command(Command::Shutdown).await?;
            tokio::time::timeout(BUDGET, engine)
                .await
                .context("the engine did not end after Shutdown")???;
            let mut asked_to_leave = false;
            while let Ok(name) = seen.try_recv() {
                asked_to_leave |= name == "shutdown";
            }
            ensure!(
                asked_to_leave,
                "the engine closed without asking its helper link to shut down"
            );
            tokio::time::timeout(BUDGET, peer)
                .await
                .context("the engine left its end of the link open")???;
            ensure!(
                opened.try_recv().is_err(),
                "a link that never failed was opened a second time"
            );

            let Some(saved) = secrets.saved("storage") else {
                bail!("the store key was not saved through the given secrets");
            };
            let key: [u8; 32] = STANDARD
                .decode(saved)?
                .try_into()
                .map_err(|_| anyhow::anyhow!("the saved store key is not 32 bytes"))?;
            Store::open_existing(&paths.store, key)
                .context("the store does not open with the key the engine saved")?;
            for account in ["storage", "device"] {
                let file = paths.root.join(format!("development-{account}.key"));
                ensure!(
                    !file.exists(),
                    "the engine wrote its own {account} key file beside the given secrets"
                );
            }
            Ok(())
        })
        .await
}
