//! The daemon names what held its loop: `daemon_turn_slow` carries the turn's
//! source and the store wait inside it, by call site. The hold is real: the
//! test takes SQLite's single writer slot, as another process holding the
//! database would, and an order that saves something waits for it on the loop.
use anyhow::{Context, Result, bail};
use base64::{Engine, engine::general_purpose::STANDARD};
use orbit_core::{call_detection::CallPreference, protocol::Command};
use orbit_daemon::{
    config::{Paths, set_secret},
    daemon, ipc,
};
use orbit_store::Store;
use serde_json::Value;
use std::time::{Duration, Instant};

/// How long the test holds the writer: past the writer's own 2 s busy
/// timeout, so the save gives up after 2 s whatever the machine's load, and
/// that turn is over the daemon's 1 s slow-turn threshold either way.
const HOLD: Duration = Duration::from_secs(3);

/// A jam guard for the daemon's start and the order, not a measured budget:
/// the suite gives a cold daemon up to 120 s under load elsewhere
/// (`tests/startup.rs`).
const JAM: Duration = Duration::from_secs(120);

fn fixture() -> Result<(tempfile::TempDir, Paths)> {
    let directory = tempfile::tempdir()?;
    let paths = Paths::resolve(Some(directory.path().join("orbit")))?;
    paths.initialize()?;
    set_secret(&paths, "storage", &STANDARD.encode([0x42; 32]), true)?;
    drop(Store::open(&paths.store, [0x42; 32])?);
    Ok((directory, paths))
}

async fn ready(paths: &Paths, daemon: &mut tokio::task::JoinHandle<Result<()>>) -> Result<()> {
    let started = Instant::now();
    loop {
        if tokio::net::UnixStream::connect(&paths.socket).await.is_ok() {
            let response = ipc::request(paths, Command::Status).await?;
            let state = serde_json::to_value(response.status.context("status had no state")?)?;
            if state["recovery"]["state"] == "ready" {
                return Ok(());
            }
        }
        tokio::select! {
            outcome = &mut *daemon => {
                outcome.context("the daemon stopped before it was ready")??;
                bail!("the daemon stopped before it was ready");
            }
            () = tokio::time::sleep(Duration::from_millis(10)) => {}
        }
        if started.elapsed() > JAM {
            bail!("the daemon was not ready within {JAM:?}");
        }
    }
}

fn logged(paths: &Paths, event: &str) -> Vec<Value> {
    std::fs::read_to_string(paths.root.join("logs/daemon.log"))
        .unwrap_or_default()
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(|line| line["event"] == event)
        .map(|line| line["fields"].clone())
        .collect()
}

#[tokio::test(flavor = "current_thread")]
async fn a_turn_held_by_the_store_writer_is_named_with_its_order_and_call_site() -> Result<()> {
    tokio::task::LocalSet::new()
        .run_until(async {
            let (_directory, paths) = fixture()?;
            let daemon_paths = paths.clone();
            let mut daemon =
                tokio::task::spawn_local(async move { daemon::run(daemon_paths, true).await });
            ready(&paths, &mut daemon).await?;
            assert!(logged(&paths, "daemon_turn_slow").is_empty());

            // Another connection takes SQLite's writer slot and gives it back
            // after `HOLD`, from a thread of its own.
            let probe = rusqlite::Connection::open(paths.store.join("orbit.sqlite3"))?;
            probe.execute_batch("BEGIN IMMEDIATE")?;
            let release = std::thread::spawn(move || -> Result<()> {
                std::thread::sleep(HOLD);
                probe.execute_batch("ROLLBACK")?;
                Ok(())
            });
            let saved = ipc::request(
                &paths,
                Command::CallPreference {
                    preference: CallPreference::Nothing,
                },
            )
            .await;
            release
                .join()
                .map_err(|_| anyhow::anyhow!("the writer holder panicked"))??;
            // Refused after the writer's busy timeout, or saved if the writer
            // came back first: the turn's length is what this test reads.
            drop(saved);
            ipc::request(&paths, Command::Shutdown).await?;
            tokio::time::timeout(JAM, daemon)
                .await
                .context("the daemon did not close")???;

            let slow = logged(&paths, "daemon_turn_slow");
            let order = slow
                .iter()
                .find(|fields| fields["source"] == "command:call_preference")
                .with_context(|| format!("the held order was not named: {slow:#?}"))?;
            assert!(
                order["wait"]
                    .as_str()
                    .is_some_and(|wait| wait.starts_with("store@daemon/call_detection.rs:")),
                "the wait must name its call site: {order:#}"
            );
            let took: u64 = order["took_ms"].as_str().context("took_ms")?.parse()?;
            let waited: u64 = order["wait_ms"].as_str().context("wait_ms")?.parse()?;
            // The hold began before the order was sent and outlasted the
            // writer's 2 s busy timeout: the order waited past the threshold.
            assert!(waited >= 1000 && took >= waited, "{order:#}");
            assert_eq!(order["step"], "handler", "{order:#}");
            assert_eq!(order["recording"], "false", "{order:#}");
            // Codes and numbers only, with the turn's op as every line has it.
            let fields = order.as_object().context("fields")?;
            let mut names: Vec<_> = fields.keys().map(String::as_str).collect();
            names.sort_unstable();
            assert_eq!(
                names,
                [
                    "op",
                    "recording",
                    "source",
                    "step",
                    "step_ms",
                    "took_ms",
                    "wait",
                    "wait_ms"
                ]
            );
            Ok(())
        })
        .await
}
