//! Bounded NDJSON transport to the native host. No native effects at import time.

#[cfg(test)]
#[path = "native_tests.rs"]
mod tests;

mod events;
mod route;
mod supervisor;
mod wire;
use events::EventRelay;
pub(crate) use events::MalformedAudio;
#[cfg(test)]
use events::{
    MAX_REPORTED_NATIVE_EVENTS, MAX_REPORTED_UNPAIRED_RESPONSES, native_error_field,
    report_native_event_once, report_unpaired_response_once,
};
pub use route::{AudioItem, NativeRoute, RouteClosed, RoutedAudio, SEQ_FIELD};
use supervisor::close_helper;
pub use supervisor::{
    HELPER_EXIT_GRACE, HelperEnd, NATIVE_EVENTS_CAPACITY, SUPERVISOR_THREAD_NAME, Supervisor, run,
};
pub use wire::{command, encode_command_line, wav};

use anyhow::{Context, Result};
use orbit_core::limits::{MAX_HELPER_COMMAND_LINE_BYTES, MAX_HELPER_EVENT_LINE_BYTES};
use orbit_core::wire::native::{NativeCommandHeader, NativeEvent, NativeEventHeader};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    process::Stdio,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::{
    io::{AsyncBufRead, AsyncWrite, AsyncWriteExt, BufReader},
    process::{Child, Command},
    sync::{Mutex, mpsc, oneshot, watch},
    time::Instant,
};

/// One command written to the helper, heartbeat included. A helper whose pipe
/// is full for this long is not reading, and the bridge is declared down.
const WRITE_TIMEOUT: Duration = Duration::from_secs(2);

pub(super) async fn run_helper_bridge(
    path: &std::path::Path,
    commands: &mut mpsc::Receiver<Value>,
    events: mpsc::Sender<Value>,
    stop: &mut watch::Receiver<bool>,
) -> Result<HelperEnd> {
    let mut child = Command::new(path)
        .arg("--stdio")
        .arg("--managed")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .kill_on_drop(true)
        .spawn()?;
    let mut reader = BufReader::new(child.stdout.take().context("stdout del helper ausente")?);
    let mut writer = child.stdin.take().context("stdin del helper ausente")?;
    match bridge_inner(&mut reader, &mut writer, commands, events, stop).await {
        // Asked to stop, or nobody left to send commands: end it in order.
        Ok(()) => Ok(close_helper(child, reader, writer).await),
        Err(error) => {
            let _ = child.kill().await;
            Err(error)
        }
    }
}

/// The bridge over caller-given pipes, never stopped from outside: the host
/// pipe fixtures of `native_tests.rs` drive it directly.
#[cfg(test)]
pub(crate) async fn bridge<R, W>(
    reader: R,
    writer: W,
    commands: mpsc::Receiver<Value>,
    events: mpsc::Sender<Value>,
) -> Result<()>
where
    R: tokio::io::AsyncBufRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let mut commands = commands;
    let (_never, mut stop) = watch::channel(false);
    bridge_inner(reader, writer, &mut commands, events, &mut stop).await
}

/// Relays frames both ways until the helper's side fails, the command channel
/// closes or `stop` turns true; the last two are the orderly ends (`Ok`).
async fn bridge_inner<R, W>(
    mut reader: R,
    mut writer: W,
    commands: &mut mpsc::Receiver<Value>,
    events: mpsc::Sender<Value>,
    stop: &mut watch::Receiver<bool>,
) -> Result<()>
where
    R: tokio::io::AsyncBufRead + Unpin,
    W: AsyncWrite + Unpin,
{
    // Read and write loops are independent. Cancelling read_frame on every
    // heartbeat would discard a partial JSON frame already consumed from stdin.
    // The bridge sends hello and heartbeat itself. Keep the same bounded
    // in-flight command index as the daemon so their replies have an owner.
    let pending_commands = Arc::new(Mutex::new(BTreeMap::<String, String>::new()));
    let mut relay = EventRelay::new(pending_commands.clone(), events);
    let read = relay.run(&mut reader);
    let send = async {
        let hello = command("hello", json!({}))?;
        record_bridge_command(&pending_commands, &hello).await?;
        tokio::time::timeout(WRITE_TIMEOUT, write(&mut writer, hello)).await??;
        let mut heartbeat = tokio::time::interval(std::time::Duration::from_secs(2));
        heartbeat.tick().await;
        loop {
            let value = tokio::select! {
                incoming = commands.recv() => match incoming {
                    Some(value) => value,
                    None => return Ok(()),
                },
                _ = heartbeat.tick() => command("heartbeat", json!({}))?,
            };
            record_bridge_command(&pending_commands, &value).await?;
            tokio::time::timeout(WRITE_TIMEOUT, write(&mut writer, value)).await??;
        }
    };
    tokio::select! {
        result = read => result,
        result = send => result,
        _ = stop.wait_for(|stopped| *stopped) => Ok(()),
    }
}

async fn record_bridge_command(
    pending: &Mutex<BTreeMap<String, String>>,
    value: &Value,
) -> Result<()> {
    let header = NativeCommandHeader::from_value(value)?;
    let mut pending = pending.lock().await;
    retain_pending_command(
        &mut pending,
        header.id.to_owned(),
        header.command.to_owned(),
    );
    Ok(())
}

/// Remember which order an id belongs to, bounded by the same diagnostic
/// budget as the daemon's `pending_native`.
///
/// Heartbeats are a probe the bridge writes every 2 s on its own. An
/// unanswered probe must not be what pushes out a command the daemon still
/// expects: that reply would arrive unnamed. The probe is forgotten first.
/// When every slot is a real command, the lexicographically first id goes,
/// the same rule as `pending_native`. This is not a wire or helper limit. A
/// reply to a forgotten id is forwarded without `_bridge_command`, with no
/// guessed type; only the daemon's capture registry can still name it.
fn retain_pending_command(pending: &mut BTreeMap<String, String>, id: String, command: String) {
    if pending.len() >= crate::daemon::MAX_PENDING_NATIVE && !pending.contains_key(&id) {
        let evicted = pending
            .iter()
            .find(|(_, name)| name.as_str() == "heartbeat")
            .map(|(evicted_id, _)| evicted_id.clone())
            .or_else(|| pending.keys().next().cloned());
        if let Some(evicted) = evicted {
            pending.remove(&evicted);
        }
    }
    pending.insert(id, command);
}

async fn write<W: AsyncWrite + Unpin>(writer: &mut W, value: Value) -> Result<()> {
    let bytes = encode_command_line(&value)?;
    writer.write_all(&bytes).await?;
    writer.flush().await?;
    Ok(())
}
