//! The capture task as a test runs it: the same task production spawns, on
//! a fixture's lane, store and gate, and a channel the test feeds as the
//! bridge would.
use super::task::{CaptureTask, TaskParts};
use crate::daemon::Runtime;
use crate::native::{AudioItem, RoutedAudio};
use anyhow::{Context, Result, bail};
use orbit_store::JobState;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

/// A jam guard for waits on the task, not a measured budget: the task's own
/// retries back off for seconds, and the suite runs under load.
pub(in crate::daemon) const TASK_JAM: Duration = Duration::from_secs(60);

/// Spawns the capture task on `runtime`'s lane, allowed to upload as the
/// runtime's state says, and returns the bridge's end of its channel.
pub(in crate::daemon) fn spawn_task(
    runtime: &Runtime,
) -> Result<(CaptureTask, mpsc::Sender<RoutedAudio>)> {
    spawn_task_ticking(runtime, super::task::CAPTURE_TICK)
}

/// The same with its own clock at `tick`.
pub(in crate::daemon) fn spawn_task_ticking(
    runtime: &Runtime,
    tick: Duration,
) -> Result<(CaptureTask, mpsc::Sender<RoutedAudio>)> {
    let (audio_tx, audio) = mpsc::channel(crate::native::NATIVE_EVENTS_CAPACITY);
    runtime.capture.lane.lock().upload = runtime.upload_view();
    let parts = TaskParts {
        lane: runtime.capture.lane.clone(),
        audio,
        port: runtime.store_actor.capture_port(),
        store: runtime.paths.store.clone(),
        diagnostics: Some(crate::diagnostics::DiagnosticLog::in_root(
            &runtime.paths.root,
        )),
        reader: crate::daemon::store_actor::open_reader(&runtime.paths.store, [0x42; 32])?,
        gate: runtime.work.retry_gate.clone(),
        root: runtime.paths.root.clone(),
        tick,
        supervision: runtime.supervision.clone(),
    };
    Ok((CaptureTask::spawn(parts)?, audio_tx))
}

/// One frame as the bridge hands it to the task, at its place `seq`.
pub(in crate::daemon) fn frame(seq: u64, value: serde_json::Value) -> RoutedAudio {
    RoutedAudio {
        seq,
        item: AudioItem::Frame(value),
    }
}

/// Waits until the upload job `id` is in `state`.
pub(in crate::daemon) async fn job_reaches(
    runtime: &Runtime,
    id: &str,
    state: JobState,
) -> Result<()> {
    let started = Instant::now();
    loop {
        let job = runtime.reader.get_job(id)?.context("job lost")?;
        if job.state == state {
            return Ok(());
        }
        if started.elapsed() > TASK_JAM {
            bail!(
                "{id} stayed {:?}, never {state:?}, within {TASK_JAM:?}",
                job.state
            );
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// The next chunk PUT a synthetic hub receives: its connection and the
/// request's bytes, headers and body.
pub(in crate::daemon) async fn accept_put(
    listener: &tokio::net::TcpListener,
) -> Result<(tokio::net::TcpStream, Vec<u8>)> {
    use tokio::io::AsyncReadExt;
    let (mut stream, _) = tokio::time::timeout(TASK_JAM, listener.accept()).await??;
    let mut bytes = Vec::new();
    let mut byte = [0];
    while !bytes.ends_with(b"\r\n\r\n") {
        stream.read_exact(&mut byte).await?;
        bytes.push(byte[0]);
    }
    let headers = String::from_utf8(bytes.clone())?;
    let length: usize = headers
        .lines()
        .find_map(|line| {
            line.to_lowercase()
                .strip_prefix("content-length: ")
                .map(str::to_owned)
        })
        .context("content length missing")?
        .parse()?;
    let header_len = bytes.len();
    bytes.resize(header_len + length, 0);
    stream.read_exact(&mut bytes[header_len..]).await?;
    Ok((stream, bytes))
}

/// The hub's durable receipt for the chunk `id` with `sha256`.
pub(in crate::daemon) async fn acknowledge(
    stream: &mut tokio::net::TcpStream,
    id: &str,
    sha256: &str,
) -> Result<()> {
    use tokio::io::AsyncWriteExt;
    let body = serde_json::json!({"durable":true,"chunk_id":id,"sha256":sha256}).to_string();
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(response.as_bytes()).await?;
    Ok(())
}
