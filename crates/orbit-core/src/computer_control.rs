//! Pure controller for the explicit computer-control vertical. A decoded
//! command conveys no authority: the actor owns the connection, the
//! identifiers and the clock.
use serde::{Deserialize, Serialize};

pub const MAX_CONTROL_HISTORY: usize = 256;
/// Bounds the origin label in each durable history entry. The two current
/// labels occupy 5 and 26 UTF-8 bytes; 64 allows a future transport name while
/// keeping an untrusted origin from inflating every repeated history view.
/// This is not a limit on tool arguments or conversation content.
pub const MAX_CONTROL_CHANNEL_BYTES: usize = 64;
/// The engine declares its own catalogue; Orbit only bounds how much of it it
/// will carry. Nothing here names a tool, so a newer engine reaches Deimos
/// without a change in Orbit.
pub const MAX_ENGINE_TOOLS: usize = 256;
pub const MAX_TOOL_NAME_BYTES: usize = 128;
pub use crate::limits::MAX_ACTION_BYTES;
/// The whole catalogue, with every input schema, as the engine hands it over.
/// Why this number: the engine on a development Mac answers `tools/list` with
/// 147 909 bytes (measured 2026-09-23), so 256 KiB leaves a newer engine roughly a 1.7x
/// margin to grow tools without a change here, and still refuses a runaway
/// catalogue well before the 1 MiB MCP frame the transport accepts. It is
/// deliberately not the screen-observation bound: a catalogue is the engine's
/// fixed self-description, it is read once per connection, and the connected
/// engine is asked for it when somebody wants it instead of it riding in the
/// state Orbit republishes every tick.
pub const MAX_ENGINE_CATALOGUE_BYTES: usize = 256 * 1024;

/// One tool exactly as the engine declared it. Orbit forwards the name and the
/// schema it was given; it never invents, renames or filters an entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EngineTool {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub input_schema: serde_json::Value,
}

impl EngineTool {
    /// Whether this is a tool at all — whether it has a name Orbit can forward.
    /// Nothing about how much the engine wrote about it is judged here.
    ///
    /// There used to be a 4 KiB bound on the description, and a tool
    /// that exceeded it was dropped from the catalogue in silence: no notice to
    /// the user, none to the assistant, nothing in a log. The catalogue is already
    /// bounded where it arrives (`MAX_ENGINE_CATALOGUE_BYTES`) and where it
    /// travels (`device_bridge::catalogue_page`), and that second bound keeps
    /// the name of anything that will not fit and says why.
    /// A tool the engine describes at length therefore reaches whoever asked.
    pub fn is_valid(&self) -> bool {
        valid_tool_name(&self.name)
    }
}

/// A request to run one engine tool. The arguments travel as the engine's own
/// JSON object: validating their shape here would recreate the hand-written
/// vocabulary that was removed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolCall {
    pub tool: String,
    #[serde(default)]
    pub arguments: serde_json::Value,
}

impl ToolCall {
    pub fn validate(&self) -> Result<(), String> {
        if !valid_tool_name(&self.tool) {
            return Err("El nombre de la herramienta del motor no es válido".into());
        }
        if !self.arguments.is_object() {
            return Err("Los argumentos de la herramienta del motor deben ser un objeto".into());
        }
        let bytes = serde_json::to_vec(self)
            .map_err(|_| "No se pudo serializar la acción del motor".to_owned())?
            .len();
        if bytes > crate::limits::MAX_ACTION_BYTES {
            return Err(format!(
                "Acción del motor de {bytes} bytes; máximo {}",
                crate::limits::MAX_ACTION_BYTES
            ));
        }
        Ok(())
    }
}

fn valid_tool_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_TOOL_NAME_BYTES
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
}

/// How one action ended, as the local record keeps it. There is no state for
/// «waiting for somebody's approval» — nothing on this path waits for one, and
/// the word once told the user a page had not opened when it had.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ControlOutcome {
    Performed,
    Cancelled,
    Unknown,
    Failed,
}

/// Reads an outcome from a record that an earlier version may have written.
/// A word this version does not know — the retired approval state among them,
/// which that version used for an action already on its way — is `Unknown`:
/// an end nobody saw. Refusing the word would make the whole record, and with
/// it every new action, unreadable on the Mac that kept it.
fn lenient_outcome<'de, D>(deserializer: D) -> Result<ControlOutcome, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let word = String::deserialize(deserializer)?;
    Ok(serde_json::from_value(serde_json::Value::String(word)).unwrap_or(ControlOutcome::Unknown))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
// Not `deny_unknown_fields`: entries recorded with the retired frictionless
// marker keep their place in the record rather than invalidating the file.
pub struct ControlHistoryEntry {
    pub operation_id: String,
    pub requested: String,
    pub application: String,
    #[serde(default = "unknown_channel")]
    pub channel: String,
    pub at_ms: i64,
    #[serde(deserialize_with = "lenient_outcome")]
    pub outcome: ControlOutcome,
}

impl ControlHistoryEntry {
    pub fn new(
        operation_id: &str,
        requested: &str,
        application: &str,
        at_ms: i64,
        outcome: ControlOutcome,
    ) -> Result<Self, &'static str> {
        let value = Self {
            operation_id: operation_id.into(),
            requested: requested.into(),
            application: application.into(),
            channel: "local".into(),
            at_ms,
            outcome,
        };
        if value.is_valid() {
            Ok(value)
        } else {
            Err("invalid control history entry")
        }
    }

    pub fn is_valid(&self) -> bool {
        bounded_plain(&self.operation_id, 128)
            && bounded_plain(&self.requested, MAX_TOOL_NAME_BYTES)
            && bounded_plain(&self.application, 256)
            && bounded_plain(&self.channel, MAX_CONTROL_CHANNEL_BYTES)
            && self.at_ms >= 0
    }

    pub fn from_channel(mut self, channel: &str) -> Result<Self, &'static str> {
        self.channel = channel.into();
        if self.is_valid() {
            Ok(self)
        } else {
            Err("invalid control history channel")
        }
    }
}

fn unknown_channel() -> String {
    "desconocido".into()
}

impl ControlOutcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Performed => "performed",
            Self::Cancelled => "cancelled",
            Self::Unknown => "unknown",
            Self::Failed => "failed",
        }
    }
}

/// The key under which the published control view carries the engine's
/// status, beside `control_history`. Named once, for writer and reader.
pub const ENGINE_STATUS_KEY: &str = "engine";
/// The key under which the published control view carries the local record.
pub const CONTROL_HISTORY_KEY: &str = "control_history";

fn bounded_plain(value: &str, maximum: usize) -> bool {
    !value.is_empty() && value.len() <= maximum && !value.chars().any(char::is_control)
}

pub mod engine;

#[cfg(test)]
mod tests;
