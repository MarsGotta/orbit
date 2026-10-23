//! Bounded NDJSON transport to the native host. No native effects at import time.

#[cfg(test)]
#[path = "native_tests.rs"]
mod tests;

mod route;
mod supervisor;
mod wire;
pub use route::{AudioItem, NativeRoute, RouteClosed, RoutedAudio, SEQ_FIELD};
use supervisor::close_helper;
pub use supervisor::{
    HELPER_EXIT_GRACE, HelperEnd, NATIVE_EVENTS_CAPACITY, SUPERVISOR_THREAD_NAME, Supervisor, run,
};
pub use wire::{command, encode_command_line, wav};

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
pub(super) struct MalformedAudio {
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
