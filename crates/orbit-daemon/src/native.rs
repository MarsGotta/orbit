//! Bounded NDJSON transport to the native host. No native effects at import time.

#[cfg(test)]
#[path = "native_tests.rs"]
mod tests;

use crate::ipc::read_frame_with_limit;
use anyhow::{Context, Result};
use orbit_core::limits::{MAX_HELPER_COMMAND_LINE_BYTES, MAX_HELPER_EVENT_LINE_BYTES};
use orbit_core::wire::native::{
    NativeAudioIdentityRef, NativeCommandHeader, NativeEvent, NativeEventHeader,
    NativeVersionHeader,
};
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

/// Ten seconds of two-track input fits with margin: the tap produces
/// ≈47 ev/s × 2 pistas and the bridge needs room to keep emitting while a
/// slow consumer drains. When it is full the bridge waits and stops reading
/// the helper's pipe: the backlog then waits in the helper, which keeps up to
/// five minutes of audio in order. This bounds memory here, never audio.
pub const NATIVE_EVENTS_CAPACITY: usize = 1000;

/// How long the daemon gives its helper to leave by itself once it has been
/// sent `shutdown` and its input has been closed, before killing it.
///
/// What it protects: the helper's own exit — releasing the global shortcuts,
/// persisting its privacy latch and writing its last diagnostic line — which a
/// `SIGKILL` would skip, and which a helper orphaned with the shortcuts still
/// registered made impossible for the next one.
///
/// Why this size: by the time it is asked, the helper has already drained its
/// audio (that happens inside the daemon's own [`crate::daemon::SHUTDOWN_BUDGET`]),
/// so what is left is an AppKit termination and one small file write. The
/// helper's own leave-anyway grace, `NativeQuitPolicy.orphanExitGrace` in
/// Swift, is two seconds; this gives it one more before the kill. Not measured
/// on the Mac yet: it is among what a reviewer verifies there.
///
/// What it is not: a wait for audio. A helper that is still capturing when this
/// starts has already missed the closure's budget and is killed on time.
pub const HELPER_EXIT_GRACE: Duration = Duration::from_secs(3);

/// Room over [`HELPER_EXIT_GRACE`] for the supervisor task to kill and reap a
/// helper that did not leave, before the daemon stops waiting for the task and
/// drops it, which kills the child without reaping it. A `SIGKILL` and a
/// `waitpid` take milliseconds; this bounds a hang, it is not a measured wait.
const HELPER_KILL_ROOM: Duration = Duration::from_secs(1);

/// Eight consecutive malformed frames declare the bridge dead. The exact
/// count is a contract with the helper, not a heuristic: any frame whose
/// bytes cannot be parsed or whose version is not 1 is malformed.
const MAX_MALFORMED_BEFORE_BRIDGE_DOWN: u32 = 8;

/// Diagnostic deduplication only: 128 names cover eight times native-v1's 13
/// event names. Each retained name and field is clipped to 64 Unicode scalars,
/// at most 256 UTF-8 bytes each, so keys stay below about 64 KiB. This does
/// not limit wire input or authorize an event; further distinct bad names are
/// still skipped.
const MAX_REPORTED_NATIVE_EVENTS: usize = 128;

/// Diagnostic deduplication only: limits the distinct unmatched response IDs
/// tracked in memory to protect against an endless stream of garbage responses.
/// 128 entries easily cover a burst of missing ACKs or delayed deliveries.
/// This ceiling only governs the one-time warning log; it does not drop or
/// suppress the delivery of the unpaired responses themselves to the daemon.
const MAX_REPORTED_UNPAIRED_RESPONSES: usize = 128;

/// Caps only the stream ID copied into a synthetic gap: native command IDs
/// already allow 128 bytes, while the current UUID-shaped stream IDs are 36.
/// It protects the gap queue from an untrusted long identity, not capture or
/// the helper's event-line ceiling; an overlong identity cuts with a cause.
const MAX_AUDIO_GAP_STREAM_ID_BYTES: usize = 128;

#[derive(Debug)]
struct MalformedAudio {
    field: String,
    found: &'static str,
    count: u32,
}

impl std::fmt::Display for MalformedAudio {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "native_audio_invalid event=audio field={} type={} count={}/{}",
            self.field, self.found, self.count, MAX_MALFORMED_BEFORE_BRIDGE_DOWN
        )
    }
}

impl std::error::Error for MalformedAudio {}

/// One command written to the helper, heartbeat included. A helper whose pipe
/// is full for this long is not reading, and the bridge is declared down.
const WRITE_TIMEOUT: Duration = Duration::from_secs(2);

/// Bounded backoff sequence (1, 2, 4, 8, 16 s) — every value sits under
/// a 30 s ceiling; a single recovery consumes one slot.
const RELAUNCH_BACKOFFS: [Duration; 5] = [
    Duration::from_secs(1),
    Duration::from_secs(2),
    Duration::from_secs(4),
    Duration::from_secs(8),
    Duration::from_secs(16),
];

/// Five consecutive failures (each with the same generation of attempts
/// after the last `ready`) exhaust the supervisor's patience.
const MAX_RELAUNCH_ATTEMPTS: u32 = 5;

/// Internal channel between the bridge and the events tap. Sized so the
/// bridge can keep forwarding while the daemon is briefly busy.
const RELAUNCH_BRIDGE_TAP_CAPACITY: usize = 1024;

/// How the helper's supervision ended, for the daemon's log: codes and flags,
/// never content.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HelperEnd {
    /// It left by itself within its grace. `asked` says whether `shutdown`
    /// reached it before its input closed.
    Exited {
        code: Option<i32>,
        signal: Option<i32>,
        asked: bool,
    },
    /// It had not left when its grace ran out and was killed.
    Killed { asked: bool },
    /// No helper was running when the stop came: between relaunches.
    NotRunning,
    /// The supervisor had already given up relaunching, or could not start it.
    GaveUp,
    /// The supervisor task did not finish within its bound and was dropped,
    /// which kills a child without reaping it.
    Lost,
}

impl HelperEnd {
    /// The fields of the daemon's `helper_stopped` line.
    pub fn fields(&self) -> Vec<(&'static str, String)> {
        let flag = |value: bool| value.to_string();
        match *self {
            Self::Exited {
                code,
                signal,
                asked,
            } => {
                let mut fields = vec![("how", "exited".to_owned()), ("asked", flag(asked))];
                if let Some(code) = code {
                    fields.push(("code", code.to_string()));
                }
                if let Some(signal) = signal {
                    fields.push(("signal", signal.to_string()));
                }
                fields
            }
            Self::Killed { asked } => vec![
                ("how", "killed".to_owned()),
                ("asked", flag(asked)),
                ("grace_ms", HELPER_EXIT_GRACE.as_millis().to_string()),
            ],
            Self::NotRunning => vec![("how", "not_running".to_owned())],
            Self::GaveUp => vec![("how", "gave_up".to_owned())],
            Self::Lost => vec![("how", "lost".to_owned())],
        }
    }
}

/// The thread the helper's supervision runs on, by this name in a crash
/// report and in `ps -T`.
pub const SUPERVISOR_THREAD_NAME: &str = "orbit-helper-bridge";

/// The helper's supervision as the daemon holds it: how it ended and the one
/// signal that ends it in order.
///
/// It runs on a thread of its own, with its own runtime, not on the daemon's.
/// The heartbeat is what tells the helper the daemon is alive. On the
/// daemon's runtime, work that held all of its threads (a blocking call, a
/// slow disk flush) would hold the beat as well, and the helper would read a
/// busy daemon as a dead one; on 2026-10-22 the helper missed the beat for
/// over 6 s and cut a meeting. Pipe reads and writes, the beat and relaunches
/// now depend only on this thread.
pub struct Supervisor {
    end: oneshot::Receiver<HelperEnd>,
    stop: watch::Sender<bool>,
}

impl Supervisor {
    pub fn spawn(
        path: Option<PathBuf>,
        commands: mpsc::Receiver<Value>,
        events: mpsc::Sender<Value>,
    ) -> Self {
        let (stop, stopped) = watch::channel(false);
        let (ended, end) = oneshot::channel();
        let announce = events.clone();
        let launched = std::thread::Builder::new()
            .name(SUPERVISOR_THREAD_NAME.into())
            .spawn(move || {
                let end = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(runtime) => runtime.block_on(supervise(path, commands, events, stopped)),
                    Err(_) => {
                        if events
                            .blocking_send(json!({"event":"helper_disconnected"}))
                            .is_err()
                        {
                            eprintln!("helper_disconnected_receiver_closed");
                        }
                        HelperEnd::GaveUp
                    }
                };
                // Nobody waits for it when the daemon did not ask to stop.
                let _ = ended.send(end);
            });
        if launched.is_err()
            && announce
                .try_send(json!({"event":"helper_disconnected"}))
                .is_err()
        {
            eprintln!("helper_disconnected_not_announced");
        }
        Self { end, stop }
    }

    /// Ends the supervision in order: the helper is sent `shutdown`, its input
    /// is closed and it gets [`HELPER_EXIT_GRACE`] to leave before it is killed.
    /// `events` is the daemon's end of the event channel, drained meanwhile so
    /// a full channel cannot hold the helper's last frames — or this stop.
    pub async fn stop(self, events: &mut mpsc::Receiver<Value>) -> HelperEnd {
        self.stop.send_replace(true);
        let mut end = self.end;
        let bound = tokio::time::sleep(HELPER_EXIT_GRACE + HELPER_KILL_ROOM);
        tokio::pin!(bound);
        let mut draining = true;
        loop {
            tokio::select! {
                ended = &mut end => return ended.unwrap_or(HelperEnd::Lost),
                event = events.recv(), if draining => draining = event.is_some(),
                // A thread cannot be aborted: the supervision ends on its own
                // within its bounded closure, and its child goes with the
                // process at the latest.
                () = &mut bound => return HelperEnd::Lost,
            }
        }
    }
}

/// The supervision's whole life on its own thread: `run`, and the fall it
/// announces when the relaunches run out.
async fn supervise(
    path: Option<PathBuf>,
    commands: mpsc::Receiver<Value>,
    events: mpsc::Sender<Value>,
    stopped: watch::Receiver<bool>,
) -> HelperEnd {
    let stopping = stopped.clone();
    match run(path, commands, events.clone(), stopped).await {
        Ok(end) => end,
        Err(_) => {
            // The daemon latches the capture on this; a supervision
            // that ends because it was asked to has nothing to report.
            if !*stopping.borrow()
                && events
                    .send(json!({"event":"helper_disconnected"}))
                    .await
                    .is_err()
            {
                eprintln!("helper_disconnected_receiver_closed");
            }
            HelperEnd::GaveUp
        }
    }
}

/// Supervises the helper until `stop` turns true, the command channel closes
/// or the relaunches run out. `stop` is how the daemon ends it in order.
pub async fn run(
    path: Option<PathBuf>,
    commands: mpsc::Receiver<Value>,
    events: mpsc::Sender<Value>,
    mut stop: watch::Receiver<bool>,
) -> Result<HelperEnd> {
    let path = path.context("El helper nativo no está configurado")?;
    if !path.is_absolute() {
        anyhow::bail!("El helper debe tener una ruta absoluta");
    }

    let mut commands = commands;
    let mut consecutive_failures: u32 = 0;
    loop {
        if *stop.borrow() {
            return Ok(HelperEnd::NotRunning);
        }
        let saw_ready = Arc::new(AtomicBool::new(false));
        let (bridge_tx, mut bridge_rx): (mpsc::Sender<Value>, mpsc::Receiver<Value>) =
            mpsc::channel(RELAUNCH_BRIDGE_TAP_CAPACITY);
        let bridge_tx_for_bridge = bridge_tx.clone();

        let tap_events = events.clone();
        let tap_saw_ready = saw_ready.clone();
        let tap = tokio::spawn(async move {
            while let Some(value) = bridge_rx.recv().await {
                if NativeEventHeader::from_value(&value).is_ok_and(|header| header.event == "ready")
                {
                    tap_saw_ready.store(true, Ordering::SeqCst);
                }
                if tap_events.send(value).await.is_err() {
                    break;
                }
            }
        });

        let bridge_result =
            run_helper_bridge(&path, &mut commands, bridge_tx_for_bridge, &mut stop).await;

        // Closing the original sender (the bridge task already dropped its
        // clone) is what signals the tap to finish.
        drop(bridge_tx);
        let _ = tap.await;

        match bridge_result {
            Ok(end) => return Ok(end),
            Err(bridge_error) => {
                let malformed_audio = bridge_error
                    .downcast_ref::<MalformedAudio>()
                    .map(ToString::to_string);
                let saw_ready = saw_ready.load(Ordering::SeqCst);
                if saw_ready {
                    // The previous cycle observed at least one ready event,
                    // so any crash that followed it starts a fresh generation.
                    consecutive_failures = 0;
                } else {
                    consecutive_failures = consecutive_failures.saturating_add(1);
                }

                if saw_ready || consecutive_failures == 1 {
                    // The daemon needs to learn the helper is gone so it can
                    // latch the capture. Every crash that closes a generation
                    // announces it: a helper that was already ready (the real
                    // case of a crash after startup) and the first crash of a
                    // generation that never got that far. Repeated crashes without a
                    // ready in between do not re-announce; the supervisor will
                    // give up after five.
                    let disconnected = if let Some(cause) = malformed_audio {
                        json!({"event":"helper_disconnected","cause":cause})
                    } else {
                        json!({"event":"helper_disconnected"})
                    };
                    if events.send(disconnected).await.is_err() {
                        eprintln!("helper_disconnected_receiver_closed");
                    }
                }

                if consecutive_failures >= MAX_RELAUNCH_ATTEMPTS {
                    return Err(anyhow::anyhow!(
                        "El helper nativo se cayó {} veces consecutivas sin volver a anunciar ready",
                        MAX_RELAUNCH_ATTEMPTS
                    ));
                }

                let backoff = RELAUNCH_BACKOFFS[consecutive_failures as usize];
                // A stop during the wait ends the supervision: nothing new is
                // launched into a daemon that is closing.
                tokio::select! {
                    () = tokio::time::sleep(backoff) => {}
                    _ = stop.wait_for(|stopped| *stopped) => return Ok(HelperEnd::NotRunning),
                }
            }
        }
    }
}

async fn run_helper_bridge(
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

/// Ends a helper in order: `shutdown`, then its input closed, then
/// [`HELPER_EXIT_GRACE`] for it to leave while its output keeps being read, and
/// only then `SIGKILL`. The child is always reaped before this returns.
async fn close_helper<R, W>(mut child: Child, reader: R, mut writer: W) -> HelperEnd
where
    R: AsyncBufRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let deadline = Instant::now() + HELPER_EXIT_GRACE;
    let asked = match command("shutdown", json!({})) {
        Ok(command) => tokio::time::timeout_at(
            deadline.min(Instant::now() + WRITE_TIMEOUT),
            write(&mut writer, command),
        )
        .await
        .is_ok_and(|written| written.is_ok()),
        Err(error) => {
            eprintln!("native_shutdown_command_invalid: {error}");
            false
        }
    };
    // End of input: the helper's other signal to leave, and the only one
    // a helper that missed `shutdown` can still get.
    drop(writer);
    // Its last frames — the shutdown receipt, a final state — are read and
    // dropped, so a full pipe cannot keep it from leaving.
    let mut reader = reader;
    let mut sink = tokio::io::sink();
    let drain = tokio::io::copy_buf(&mut reader, &mut sink);
    tokio::pin!(drain);
    let mut drained = false;
    let left = tokio::time::timeout_at(deadline, async {
        loop {
            tokio::select! {
                status = child.wait() => return status,
                _ = &mut drain, if !drained => drained = true,
            }
        }
    })
    .await;
    match left {
        Ok(Ok(status)) => {
            use std::os::unix::process::ExitStatusExt;
            HelperEnd::Exited {
                code: status.code(),
                signal: status.signal(),
                asked,
            }
        }
        _ => {
            let _ = child.kill().await;
            HelperEnd::Killed { asked }
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

fn native_field_type(value: &Value, path: &serde_path_to_error::Path) -> &'static str {
    use serde_path_to_error::Segment;
    let mut field = Some(value);
    for segment in path {
        field = match segment {
            Segment::Map { key } => field.and_then(|value| value.get(key)),
            Segment::Seq { index } => field.and_then(|value| value.get(*index)),
            Segment::Enum { .. } => field,
            Segment::Unknown => None,
        };
    }
    match field {
        Some(Value::Null) => "null",
        Some(Value::Bool(_)) => "boolean",
        Some(Value::Number(_)) => "number",
        Some(Value::String(_)) => "string",
        Some(Value::Array(_)) => "array",
        Some(Value::Object(_)) => "object",
        None => "absent",
    }
}

fn native_error_field(
    value: &Value,
    error: &serde_path_to_error::Error<serde_json::Error>,
) -> (String, &'static str) {
    let detail = error.inner().to_string();
    if let Some(missing) = detail
        .strip_prefix("missing field `")
        .and_then(|rest| rest.split_once('`'))
    {
        return (missing.0.to_owned(), "absent");
    }
    fn locate_invalid_values(
        value: &Value,
        detail: &str,
        prefix: &str,
        pointer: &str,
        found: &mut Vec<(String, String, &'static str)>,
    ) {
        match value {
            Value::Object(fields) => fields.iter().for_each(|(name, field)| {
                let path = if prefix.is_empty() {
                    name.to_owned()
                } else {
                    format!("{prefix}.{name}")
                };
                let escaped = name.replace('~', "~0").replace('/', "~1");
                locate_invalid_values(field, detail, &path, &format!("{pointer}/{escaped}"), found);
            }),
            Value::Array(fields) => fields.iter().enumerate().for_each(|(index, field)| {
                locate_invalid_values(
                    field,
                    detail,
                    &format!("{prefix}[{index}]"),
                    &format!("{pointer}/{index}"),
                    found,
                );
            }),
            Value::String(text) if detail.contains(&format!("string {text:?}")) => {
                found.push((prefix.to_owned(), pointer.to_owned(), "string"));
            }
            Value::Number(number)
                if detail.contains(&format!("integer `{number}`"))
                    || detail.contains(&format!("floating point `{number}`")) =>
            {
                found.push((prefix.to_owned(), pointer.to_owned(), "number"));
            }
            Value::Bool(boolean) if detail.contains(&format!("boolean `{boolean}`")) => {
                found.push((prefix.to_owned(), pointer.to_owned(), "boolean"));
            }
            // A scalar that serde's message does not name is not the culprit.
            Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
        }
    }
    let mut candidates = Vec::new();
    locate_invalid_values(value, &detail, "", "", &mut candidates);
    if candidates.len() > 1 {
        let repair = if detail.contains("expected a boolean") {
            Some(Value::Bool(false))
        } else if detail.contains("expected a string") {
            Some(Value::String(String::new()))
        } else {
            None
        };
        if let Some(repair) = repair {
            // Serde loses the path inside an internally tagged enum. On a
            // malformed frame only, try each same-valued field with the
            // expected type so equal numbers cannot accuse the wrong field.
            for (field, pointer, found_type) in &candidates {
                let mut repaired = value.clone();
                if let Some(candidate) = repaired.pointer_mut(pointer) {
                    *candidate = repair.clone();
                    if serde_json::from_value::<NativeEvent>(repaired).is_ok() {
                        return (field.clone(), found_type);
                    }
                }
            }
        }
    }
    if let Some((field, _, found_type)) = candidates.into_iter().next() {
        return (field, found_type);
    }
    (
        error.path().to_string(),
        native_field_type(value, error.path()),
    )
}

fn report_native_event_once(
    reported: &mut BTreeSet<(String, String, &'static str)>,
    event: &str,
    field: &str,
    found: &'static str,
) -> Option<(String, String)> {
    let event: String = event.chars().take(64).collect();
    let field: String = field.chars().take(64).collect();
    if reported.len() < MAX_REPORTED_NATIVE_EVENTS
        && reported.insert((event.clone(), field.clone(), found))
    {
        Some((event, field))
    } else {
        None
    }
}

fn report_unpaired_response_once(reported: &mut BTreeSet<String>, id: &str) -> Option<String> {
    let id: String = id.chars().take(128).collect();
    if reported.len() < MAX_REPORTED_UNPAIRED_RESPONSES && reported.insert(id.clone()) {
        Some(id)
    } else {
        None
    }
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
    let mut consecutive_malformed: u32 = 0;
    let mut reported_native_events = BTreeSet::new();
    let mut reported_unpaired_responses = BTreeSet::new();
    // The bridge sends hello and heartbeat itself. Keep the same bounded
    // in-flight command index as the daemon so their replies have an owner.
    let pending_commands = Arc::new(Mutex::new(BTreeMap::<String, String>::new()));
    let reading_commands = pending_commands.clone();
    let read = async {
        loop {
            let bytes = read_frame_with_limit(&mut reader, MAX_HELPER_EVENT_LINE_BYTES).await?;
            let mut value: Value = match serde_json::from_slice(&bytes) {
                Ok(value) => value,
                Err(_) => {
                    consecutive_malformed = consecutive_malformed.saturating_add(1);
                    if consecutive_malformed >= MAX_MALFORMED_BEFORE_BRIDGE_DOWN {
                        anyhow::bail!(
                            "Puente declarado caído por {} frames malformados consecutivos",
                            consecutive_malformed
                        );
                    }
                    continue;
                }
            };
            if !NativeVersionHeader::from_value(&value).is_ok_and(|header| header.v == 1) {
                consecutive_malformed = consecutive_malformed.saturating_add(1);
                if consecutive_malformed >= MAX_MALFORMED_BEFORE_BRIDGE_DOWN {
                    anyhow::bail!(
                        "Puente declarado caído por {} frames malformados consecutivos",
                        consecutive_malformed
                    );
                }
                continue;
            }
            let Ok(header) = NativeEventHeader::from_value(&value) else {
                if report_native_event_once(
                    &mut reported_native_events,
                    "<missing>",
                    "event",
                    "absent_or_non_string",
                )
                .is_some()
                {
                    eprintln!(
                        "native_event_invalid event=<missing> field=event type=absent_or_non_string"
                    );
                }
                continue;
            };
            let event_name = header.event;
            if !NativeEvent::names().contains(&event_name) {
                if let Some((name, _)) =
                    report_native_event_once(&mut reported_native_events, event_name, "", "unknown")
                {
                    eprintln!("native_event_unknown event={name}");
                }
                continue;
            }
            let mut decoder = serde_json::Deserializer::from_slice(&bytes);
            let typed: NativeEvent = match serde_path_to_error::deserialize(&mut decoder) {
                Ok(event) => event,
                Err(error) => {
                    let (field, found) = native_error_field(&value, &error);
                    if let Some((name, field_name)) = report_native_event_once(
                        &mut reported_native_events,
                        event_name,
                        &field,
                        found,
                    ) {
                        eprintln!(
                            "native_event_invalid event={name} field={field_name} type={found}"
                        );
                    }
                    if event_name == "audio" {
                        // With no trustworthy track identity a gap cannot be
                        // attributed. Cut immediately and carry the decode
                        // cause to the visible helper-loss notice.
                        let identity = match NativeAudioIdentityRef::from_value(&value) {
                            Ok(identity) => identity,
                            Err(_) => {
                                return Err(MalformedAudio {
                                    field,
                                    found,
                                    count: consecutive_malformed.saturating_add(1),
                                }
                                .into());
                            }
                        };
                        if !matches!(identity.source, "mic_local" | "remote_app")
                            || identity.stream_id.is_empty()
                            || identity.stream_id.len() > MAX_AUDIO_GAP_STREAM_ID_BYTES
                        {
                            return Err(MalformedAudio {
                                field: "source_or_stream_id".to_owned(),
                                found: "invalid_identity",
                                count: consecutive_malformed.saturating_add(1),
                            }
                            .into());
                        }
                        let cause =
                            format!("native_audio_invalid event=audio field={field} type={found}");
                        let gap = json!({
                            "v": orbit_core::wire::native::NATIVE_PROTOCOL,
                            "event": "gap",
                            "source": identity.source,
                            "stream_id": identity.stream_id,
                            "reason": cause.clone(),
                            "frames": 1,
                        });
                        if events.send(gap).await.is_err() {
                            anyhow::bail!("Canal de eventos cerrado");
                        }
                        consecutive_malformed = consecutive_malformed.saturating_add(1);
                        if consecutive_malformed >= MAX_MALFORMED_BEFORE_BRIDGE_DOWN {
                            return Err(MalformedAudio {
                                field,
                                found,
                                count: consecutive_malformed,
                            }
                            .into());
                        }
                    }
                    continue;
                }
            };
            consecutive_malformed = 0;

            if let NativeEvent::Response { ref id, .. } = typed {
                // A helper field cannot name its own order. Only an ID we
                // actually wrote supplies the internal correlation hint.
                if let Some(object) = value.as_object_mut() {
                    object.remove("_bridge_command");
                }
                let command = reading_commands.lock().await.remove(id);
                if let Some(command) = command {
                    if let Some(object) = value.as_object_mut() {
                        object.insert("_bridge_command".into(), Value::String(command));
                    }
                } else {
                    if let Some(id) =
                        report_unpaired_response_once(&mut reported_unpaired_responses, id)
                    {
                        eprintln!("native_response_unpaired id={id}");
                    }
                }
            }

            // Every frame waits for the consumer, audio included. A daemon
            // that stops reading for a while (an 11 s stall cut a meeting on
            // 2026-10-22) is not a dead link: while this waits it stops
            // reading the pipe, the helper keeps the backlog in order and
            // counts in a `gap` only what does not fit in its own room. The
            // backlog then arrives whole, never as a gap or a lost helper.
            if events.send(value).await.is_err() {
                anyhow::bail!("Canal de eventos cerrado");
            }
        }
    };
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

/// Encodes the exact line sent to the helper, including its newline. The
/// helper's 6 MiB stdin contract bounds this allocation before any write.
pub fn encode_command_line(value: &Value) -> Result<Vec<u8>> {
    let mut bytes = serde_json::to_vec(value)?;
    bytes.push(b'\n');
    if bytes.len() > MAX_HELPER_COMMAND_LINE_BYTES {
        anyhow::bail!(
            "Comando al helper de {} bytes; máximo {MAX_HELPER_COMMAND_LINE_BYTES}",
            bytes.len()
        );
    }
    Ok(bytes)
}

pub fn command(name: &str, fields: Value) -> Result<Value> {
    let mut fields = match fields {
        Value::Object(fields) => fields,
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) | Value::Array(_) => {
            anyhow::bail!("Los campos de la orden nativa deben ser un objeto JSON")
        }
    };
    let id = match fields.remove("id") {
        Some(Value::String(id)) => id,
        Some(_) => anyhow::bail!("El ID de la orden nativa debe ser texto"),
        None => uuid::Uuid::new_v4().to_string(),
    };
    let frame =
        orbit_core::wire::native::NativeCommand::from_parts(id, name, Value::Object(fields))
            .map_err(anyhow::Error::msg)?;
    serde_json::to_value(frame).context("No se pudo serializar la orden nativa")
}

/// Encode an observed PCM segment with its actual rate. No invented timestamps.
pub fn wav(pcm: &[u8], sample_rate: u32, channels: u16) -> Result<Vec<u8>> {
    if channels == 0
        || channels > 2
        || !(8_000..=192_000).contains(&sample_rate)
        || pcm.len() > 32 * 1024 * 1024
        || !pcm.len().is_multiple_of(usize::from(channels) * 2)
    {
        anyhow::bail!("Formato PCM no válido");
    }
    let len = u32::try_from(pcm.len())?;
    let mut out = Vec::with_capacity(pcm.len() + 44);
    out.extend(b"RIFF");
    out.extend((len + 36).to_le_bytes());
    out.extend(b"WAVEfmt ");
    out.extend(16u32.to_le_bytes());
    out.extend(1u16.to_le_bytes());
    out.extend(channels.to_le_bytes());
    out.extend(sample_rate.to_le_bytes());
    out.extend((sample_rate * u32::from(channels) * 2).to_le_bytes());
    out.extend((channels * 2).to_le_bytes());
    out.extend(16u16.to_le_bytes());
    out.extend(b"data");
    out.extend(len.to_le_bytes());
    out.extend(pcm);
    Ok(out)
}
