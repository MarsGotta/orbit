//! ⌘Q with a meeting recording: once its audio is stored, its last chunks and
//! its closure reach a hub that answers before Orbit leaves, and a hub that
//! does not answer cannot hold the exit past the closing budget. Synthetic
//! audio, helper and hub only.
use super::*;
use crate::daemon::audio::tests::{fixture, settle_seal_receipts};
use crate::daemon::capture_lane::test_support;
use crate::daemon::meeting_continuity_tests::{frame, start};
use crate::daemon::shutdown::Shutdown;
use anyhow::{Context, Result};
use orbit_core::protocol::Command;
use orbit_store::JobState;
use serde_json::{Value, json};
use std::time::{Duration, Instant};
use tokio::io::AsyncWriteExt;
use tokio::net::TcpListener;
use tokio::sync::mpsc;

const RECORDING: &str = "reunion-sintetica";

/// One request the synthetic hub received: its method and path, and the
/// closure's body when it is one.
#[derive(Debug)]
struct Received {
    method: String,
    path: String,
    body: Value,
}

/// A hub that answers every chunk and closure with its durable receipt.
async fn answering_hub(listener: TcpListener, seen: mpsc::UnboundedSender<Received>) -> Result<()> {
    loop {
        let (mut stream, request) = test_support::accept_put(&listener).await?;
        let text = String::from_utf8_lossy(&request).into_owned();
        let (head, body) = text
            .split_once("\r\n\r\n")
            .context("request without headers")?;
        let mut line = head.lines().next().context("no request line")?.split(' ');
        let method = line.next().context("no method")?.to_owned();
        let path = line.next().context("no path")?.to_owned();
        let receipt = if method == "PUT" {
            let sha256 = head
                .lines()
                .find_map(|line| {
                    line.to_lowercase()
                        .strip_prefix("x-orbit-sha256: ")
                        .map(str::to_owned)
                })
                .context("chunk without its digest")?;
            let chunk = path.trim_start_matches("/v1/chunks/");
            json!({"durable":true,"chunk_id":chunk,"sha256":sha256})
        } else {
            json!({"durable":true,"finalized":true,"recording_id":RECORDING})
        };
        let body = if method == "POST" {
            serde_json::from_str(body)?
        } else {
            Value::Null
        };
        seen.send(Received { method, path, body })?;
        let receipt = receipt.to_string();
        stream
            .write_all(
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{receipt}",
                    receipt.len()
                )
                .as_bytes(),
            )
            .await?;
    }
}

/// A meeting with two sealed one-second chunks the hub has not received,
/// then ⌘Q: the closing order, the helper's drained tail and its receipt.
/// Returns the closing as the loop holds it once nothing is left to store,
/// and the wall time just before the quit.
async fn quit_while_recording(
    runtime: &mut Runtime,
    helper: &mut mpsc::Receiver<Value>,
) -> Result<(Shutdown, i64)> {
    let stream = start(runtime, helper).await?;
    runtime.handle_native_event(frame(&stream, 0, 8_000)).await;
    runtime.handle_native_event(frame(&stream, 1, 8_000)).await;
    settle_seal_receipts(runtime).await?;
    let quit_at_ms = crate::daemon::now();
    let mut closing = Shutdown::default();
    // The order reports the tail still in memory; the loop goes on serving
    // the helper until it is stored.
    let _ = closing.command(Command::Shutdown, runtime).await;
    let stop = loop {
        let command = helper.recv().await.context("capture_stop absent")?;
        if command["command"] == "capture_stop" {
            break command;
        }
    };
    runtime.handle_native_event(frame(&stream, 2, 4_000)).await;
    runtime
        .handle_native_event(json!({"event":"response","id":stop["id"],"ok":true,
            "result":{"drain_complete":true}}))
        .await;
    settle_seal_receipts(runtime).await?;
    runtime.run_maintenance().await;
    assert!(
        closing.ready(runtime),
        "the fixture must reach the loop's exit gate: {:?}",
        runtime.has_any_pending()
    );
    Ok((closing, quit_at_ms))
}

fn finalize_state(runtime: &Runtime) -> Result<Option<JobState>> {
    Ok(runtime
        .reader
        .get_job(&format!("finalize-{RECORDING}"))?
        .map(|job| job.state))
}

#[tokio::test]
async fn command_q_while_recording_delivers_the_last_chunks_and_the_closure_before_leaving()
-> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut helper) = fixture(directory.path())?;
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    runtime.settings.hub_url = Some(format!("http://{}", listener.local_addr()?));
    runtime.link.token = Some("synthetic-not-a-credential".into());
    let (seen, mut received) = mpsc::unbounded_channel();
    let hub = tokio::spawn(answering_hub(listener, seen));
    let (closing, quit_at_ms) = quit_while_recording(&mut runtime, &mut helper).await?;
    let deadline = closing.deadline.context("the closure latched no budget")?;
    runtime.deliver_before_exit(deadline).await;
    hub.abort();
    let mut requests = Vec::new();
    while let Ok(request) = received.try_recv() {
        requests.push(request);
    }
    let puts: Vec<_> = requests.iter().filter(|r| r.method == "PUT").collect();
    assert_eq!(
        puts.len(),
        3,
        "every chunk, the drained tail included: {requests:?}"
    );
    let closure = requests
        .last()
        .filter(|r| r.method == "POST" && r.path == format!("/v1/recordings/{RECORDING}/finalize"))
        .with_context(|| format!("the hub received no closure, after the chunks: {requests:?}"))?;
    assert_eq!(closure.body["gaps"], 0, "{:?}", closure.body);
    assert_eq!(
        closure.body["tracks"][0]["chunk_count"], 3,
        "{:?}",
        closure.body
    );
    let finalized_at_ms = closure.body["finalized_at_ms"]
        .as_i64()
        .context("closure without its end")?;
    assert!(
        finalized_at_ms >= quit_at_ms,
        "the meeting ended at the quit, not before it: {finalized_at_ms} < {quit_at_ms}"
    );
    assert_eq!(finalize_state(&runtime)?, Some(JobState::Succeeded));
    Ok(())
}

#[tokio::test]
async fn a_hub_that_does_not_answer_leaves_the_closure_queued_within_the_budget() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut helper) = fixture(directory.path())?;
    // A hub that takes the connection and never answers.
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    runtime.settings.hub_url = Some(format!("http://{}", listener.local_addr()?));
    runtime.link.token = Some("synthetic-not-a-credential".into());
    let (_closing, _) = quit_while_recording(&mut runtime, &mut helper).await?;
    // Two seconds: the local closure (a store write) fits in it on a loaded
    // Mac, where 300 ms once did not; the silent hub then holds the rest.
    let deadline = Instant::now() + Duration::from_secs(2);
    // A jam guard, not a measured budget: the delivery must stop at the
    // deadline, far under the hub client's own thirty-second timeout.
    tokio::time::timeout(
        Duration::from_secs(10),
        runtime.deliver_before_exit(deadline),
    )
    .await
    .context("the delivery outlived the closing budget")?;
    assert_eq!(
        finalize_state(&runtime)?,
        Some(JobState::Queued),
        "the closure stays on this Mac for the next start"
    );
    drop(listener);
    Ok(())
}

#[tokio::test]
async fn after_the_privacy_stop_nothing_goes_up_on_the_way_out() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut helper) = fixture(directory.path())?;
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    runtime.settings.hub_url = Some(format!("http://{}", listener.local_addr()?));
    runtime.link.token = Some("synthetic-not-a-credential".into());
    let stream = start(&mut runtime, &mut helper).await?;
    runtime.handle_native_event(frame(&stream, 0, 8_000)).await;
    settle_seal_receipts(&mut runtime).await?;
    runtime.command(Command::PrivacyStop).await?;
    assert!(runtime.state.privacy_stopped());
    runtime
        .deliver_before_exit(Instant::now() + Duration::from_secs(5))
        .await;
    // A connection attempt completes in the listener's backlog without an
    // accept, so one made by the delivery would be waiting here.
    assert!(
        futures_util::FutureExt::now_or_never(listener.accept()).is_none(),
        "a privacy-stopped Mac reached the hub while closing"
    );
    Ok(())
}

/// A hub out of reach would hold every quit for the rest of the budget (its
/// connect timeout is ten seconds): the way out only tries a hub that was
/// online when the quit came, and says why it did not.
#[tokio::test]
async fn a_quit_while_the_hub_was_offline_does_not_wait_for_it() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut helper) = fixture(directory.path())?;
    crate::daemon::turn::said::trace(&mut runtime, directory.path());
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    runtime.settings.hub_url = Some(format!("http://{}", listener.local_addr()?));
    runtime.link.token = Some("synthetic-not-a-credential".into());
    let stream = start(&mut runtime, &mut helper).await?;
    runtime.handle_native_event(frame(&stream, 0, 8_000)).await;
    settle_seal_receipts(&mut runtime).await?;
    crate::daemon::authority_fixture::set(
        &mut runtime.state,
        "connection",
        json!(orbit_core::Connection::Offline),
    );
    let mut closing = Shutdown::default();
    let _ = closing.command(Command::Shutdown, &mut runtime).await;
    runtime
        .deliver_before_exit(Instant::now() + Duration::from_secs(5))
        .await;
    assert!(
        futures_util::FutureExt::now_or_never(listener.accept()).is_none(),
        "the way out tried a hub that was offline at the quit"
    );
    let log = std::fs::read_to_string(directory.path().join("logs/daemon.log"))?;
    assert!(log.contains("hub_offline"), "{log}");
    Ok(())
}
