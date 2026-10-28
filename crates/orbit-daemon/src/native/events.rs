//! The helper's events, read off its stdout and relayed to the daemon in
//! order: each frame checked against native-v1, a reply named by the order
//! it answers, and a malformed audio frame turned into a gap or a cut.
use super::*;
use crate::ipc::read_frame_with_limit;
use orbit_core::wire::native::{NativeAudioIdentityRef, NativeVersionHeader};

/// Eight consecutive malformed frames declare the bridge dead. The exact
/// count is a contract with the helper, not a heuristic: any frame whose
/// bytes cannot be parsed or whose version is not 1 is malformed.
const MAX_MALFORMED_BEFORE_BRIDGE_DOWN: u32 = 8;

/// Diagnostic deduplication only: 128 names cover eight times native-v1's 13
/// event names. Each retained name and field is clipped to 64 Unicode scalars,
/// at most 256 UTF-8 bytes each, so keys stay below about 64 KiB. This does
/// not limit wire input or authorize an event; further distinct bad names are
/// still skipped.
pub(super) const MAX_REPORTED_NATIVE_EVENTS: usize = 128;

/// Diagnostic deduplication only: limits the distinct unmatched response IDs
/// tracked in memory to protect against an endless stream of garbage responses.
/// 128 entries easily cover a burst of missing ACKs or delayed deliveries.
/// This ceiling only governs the one-time warning log; it does not drop or
/// suppress the delivery of the unpaired responses themselves to the daemon.
pub(super) const MAX_REPORTED_UNPAIRED_RESPONSES: usize = 128;

/// Caps only the stream ID copied into a synthetic gap: native command IDs
/// already allow 128 bytes, while the current UUID-shaped stream IDs are 36.
/// It protects the gap queue from an untrusted long identity, not capture or
/// the helper's event-line ceiling; an overlong identity cuts with a cause.
const MAX_AUDIO_GAP_STREAM_ID_BYTES: usize = 128;

#[derive(Debug)]
pub(crate) struct MalformedAudio {
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

/// What the read side of the bridge keeps between frames.
pub(super) struct EventRelay {
    consecutive_malformed: u32,
    reported_native_events: BTreeSet<(String, String, &'static str)>,
    reported_unpaired_responses: BTreeSet<String>,
    /// The orders written to the helper, by id, shared with the write side.
    pending_commands: Arc<Mutex<BTreeMap<String, String>>>,
    events: mpsc::Sender<Value>,
}

impl EventRelay {
    pub(super) fn new(
        pending_commands: Arc<Mutex<BTreeMap<String, String>>>,
        events: mpsc::Sender<Value>,
    ) -> Self {
        Self {
            consecutive_malformed: 0,
            reported_native_events: BTreeSet::new(),
            reported_unpaired_responses: BTreeSet::new(),
            pending_commands,
            events,
        }
    }

    /// Reads frames until the pipe fails, the helper is declared down or
    /// the daemon stops listening.
    pub(super) async fn run<R>(&mut self, reader: &mut R) -> Result<()>
    where
        R: tokio::io::AsyncBufRead + Unpin,
    {
        loop {
            let bytes = read_frame_with_limit(&mut *reader, MAX_HELPER_EVENT_LINE_BYTES).await?;
            self.relay(&bytes).await?;
        }
    }

    /// One frame: dropped when it is not native-v1, said once when its
    /// event is unknown or invalid, and otherwise sent on to the daemon.
    async fn relay(&mut self, bytes: &[u8]) -> Result<()> {
        let mut value: Value = match serde_json::from_slice(bytes) {
            Ok(value) => value,
            Err(_) => return self.malformed(),
        };
        if !NativeVersionHeader::from_value(&value).is_ok_and(|header| header.v == 1) {
            return self.malformed();
        }
        let Ok(header) = NativeEventHeader::from_value(&value) else {
            if report_native_event_once(
                &mut self.reported_native_events,
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
            return Ok(());
        };
        let event_name = header.event;
        if !NativeEvent::names().contains(&event_name) {
            if let Some((name, _)) = report_native_event_once(
                &mut self.reported_native_events,
                event_name,
                "",
                "unknown",
            ) {
                eprintln!("native_event_unknown event={name}");
            }
            return Ok(());
        }
        let mut decoder = serde_json::Deserializer::from_slice(bytes);
        let typed: NativeEvent = match serde_path_to_error::deserialize(&mut decoder) {
            Ok(event) => event,
            Err(error) => return self.invalid_event(&value, event_name, &error).await,
        };
        self.consecutive_malformed = 0;
        if let NativeEvent::Response { ref id, .. } = typed {
            self.name_response(&mut value, id).await;
        }
        // Every frame waits for the consumer, audio included. A daemon
        // that stops reading for a while (an 11 s stall cut a meeting on
        // 2026-10-22) is not a dead link: while this waits it stops
        // reading the pipe, the helper keeps the backlog in order and
        // counts in a `gap` only what does not fit in its own room. The
        // backlog then arrives whole, never as a gap or a lost helper.
        if self.events.send(value).await.is_err() {
            anyhow::bail!("Canal de eventos cerrado");
        }
        Ok(())
    }

    /// A frame that is not native-v1. Eight in a row declare the bridge down.
    fn malformed(&mut self) -> Result<()> {
        self.consecutive_malformed = self.consecutive_malformed.saturating_add(1);
        if self.consecutive_malformed >= MAX_MALFORMED_BEFORE_BRIDGE_DOWN {
            anyhow::bail!(
                "Bridge declared down after {} consecutive malformed frames",
                self.consecutive_malformed
            );
        }
        Ok(())
    }

    /// A known event whose fields do not decode: said once by its field,
    /// and, for audio, a gap or a cut.
    async fn invalid_event(
        &mut self,
        value: &Value,
        event_name: &str,
        error: &serde_path_to_error::Error<serde_json::Error>,
    ) -> Result<()> {
        let (field, found) = native_error_field(value, error);
        if let Some((name, field_name)) =
            report_native_event_once(&mut self.reported_native_events, event_name, &field, found)
        {
            eprintln!("native_event_invalid event={name} field={field_name} type={found}");
        }
        if event_name == "audio" {
            self.audio_gap(value, field, found).await?;
        }
        Ok(())
    }

    /// A malformed audio frame becomes a gap on its track. With no
    /// trustworthy track identity a gap cannot be attributed: it cuts at
    /// once and carries the decode cause to the visible helper-loss notice.
    async fn audio_gap(&mut self, value: &Value, field: String, found: &'static str) -> Result<()> {
        let identity = match NativeAudioIdentityRef::from_value(value) {
            Ok(identity) => identity,
            Err(_) => {
                return Err(MalformedAudio {
                    field,
                    found,
                    count: self.consecutive_malformed.saturating_add(1),
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
                count: self.consecutive_malformed.saturating_add(1),
            }
            .into());
        }
        let cause = format!("native_audio_invalid event=audio field={field} type={found}");
        let gap = json!({
            "v": orbit_core::wire::native::NATIVE_PROTOCOL,
            "event": "gap",
            "source": identity.source,
            "stream_id": identity.stream_id,
            "reason": cause.clone(),
            "frames": 1,
        });
        if self.events.send(gap).await.is_err() {
            anyhow::bail!("Canal de eventos cerrado");
        }
        self.consecutive_malformed = self.consecutive_malformed.saturating_add(1);
        if self.consecutive_malformed >= MAX_MALFORMED_BEFORE_BRIDGE_DOWN {
            return Err(MalformedAudio {
                field,
                found,
                count: self.consecutive_malformed,
            }
            .into());
        }
        Ok(())
    }

    /// A helper field cannot name its own order. Only an ID we actually
    /// wrote supplies the internal correlation hint.
    async fn name_response(&mut self, value: &mut Value, id: &str) {
        if let Some(object) = value.as_object_mut() {
            object.remove("_bridge_command");
        }
        let command = self.pending_commands.lock().await.remove(id);
        if let Some(command) = command {
            if let Some(object) = value.as_object_mut() {
                object.insert("_bridge_command".into(), Value::String(command));
            }
        } else if let Some(id) =
            report_unpaired_response_once(&mut self.reported_unpaired_responses, id)
        {
            eprintln!("native_response_unpaired id={id}");
        }
    }
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

pub(super) fn native_error_field(
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
    let mut candidates = Vec::new();
    locate_invalid_values(value, &detail, "", "", &mut candidates);
    if candidates.len() > 1
        && let Some(accused) = accused_by_repair(value, &detail, &candidates)
    {
        return accused;
    }
    if let Some((field, _, found_type)) = candidates.into_iter().next() {
        return (field, found_type);
    }
    (
        error.path().to_string(),
        native_field_type(value, error.path()),
    )
}

/// Every scalar of `value` that serde's message `detail` names, with its
/// dotted path, its JSON pointer and its type.
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

/// Serde loses the path inside an internally tagged enum. On a malformed
/// frame only, try each same-valued field with the expected type so equal
/// numbers cannot accuse the wrong field: the first whose repair decodes.
fn accused_by_repair(
    value: &Value,
    detail: &str,
    candidates: &[(String, String, &'static str)],
) -> Option<(String, &'static str)> {
    let repair = if detail.contains("expected a boolean") {
        Value::Bool(false)
    } else if detail.contains("expected a string") {
        Value::String(String::new())
    } else {
        return None;
    };
    for (field, pointer, found_type) in candidates {
        let mut repaired = value.clone();
        if let Some(candidate) = repaired.pointer_mut(pointer) {
            *candidate = repair.clone();
            if serde_json::from_value::<NativeEvent>(repaired).is_ok() {
                return Some((field.clone(), found_type));
            }
        }
    }
    None
}

pub(super) fn report_native_event_once(
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

pub(super) fn report_unpaired_response_once(
    reported: &mut BTreeSet<String>,
    id: &str,
) -> Option<String> {
    let id: String = id.chars().take(128).collect();
    if reported.len() < MAX_REPORTED_UNPAIRED_RESPONSES && reported.insert(id.clone()) {
        Some(id)
    } else {
        None
    }
}
