//! The state the daemon keeps and serves. The saved part, [`DurableState`],
//! is one part per domain (`transition/`); the views beside it are
//! presentation only.
use serde::{Deserialize, Serialize};
use std::ops::Deref;

mod capabilities;
mod capture;
mod conversation;
mod durable;
mod durable_wire;

pub use capabilities::Capabilities;
pub use capture::*;
pub use conversation::*;
pub use durable::DurableState;

pub const PROTOCOL_VERSION: u8 = 1;
pub const MAX_TEXT_BYTES: usize = 32 * 1024;

/// Legacy spelling kept for tests and external clients; production uses resource names.
pub const MAX_FRAME_BYTES: usize = crate::limits::MAX_IPC_LINE_BYTES;
pub const MAX_VOICE_NOTE_BYTES: usize = 8 * 1024 * 1024;

/// Bounds the human recording title copied into the final manifest, and
/// the name the user gives a meeting afterwards (`notes::meeting_title`):
/// the same title on the hub (`MAX_RECORDING_TITLE_CHARACTERS`). 256 Unicode
/// scalar values matches Pydantic's `str` length and the hub intake; this is
/// not a byte or transcript limit.
pub const MAX_MANIFEST_TITLE_CHARACTERS: usize = 256;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Connection {
    #[default]
    Offline,
    Connecting,
    Online,
    Revoked,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountSummary {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hub_url: Option<SummaryText>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_name: Option<SummaryText>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_id_short: Option<SummaryText>,
}

/// Heap-backed text keeps these additive presentation summaries from inflating
/// every cloned actor frame while preserving their JSON string contract.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
#[allow(clippy::box_collection)] // Thin pointer keeps State below its enforced 4 KiB bound.
pub struct SummaryText(Box<String>);

impl From<&str> for SummaryText {
    fn from(value: &str) -> Self {
        Self(Box::new(value.to_owned()))
    }
}

impl From<String> for SummaryText {
    fn from(value: String) -> Self {
        Self(Box::new(value))
    }
}

impl Deref for SummaryText {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        self.0.as_str()
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionState {
    Granted,
    Denied,
    /// The helper checked a macOS preflight and got false; TCC did not say
    /// whether the person denied it or has not decided yet.
    NotGranted,
    NotDetermined,
    #[default]
    Unknown,
}

impl PermissionState {
    pub(crate) fn is_unknown(&self) -> bool {
        *self == Self::Unknown
    }
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Views {
    #[serde(default)]
    pub source_inventory: Option<crate::sources::WindowInventory>,
    #[serde(default)]
    pub source_request: Option<crate::sources::SourceProgress>,
    #[serde(default)]
    pub source_receipt: Option<crate::sources::SourceReceipt>,

    /// Startup recovery is an observed phase, not an absent/default state.
    #[serde(default, skip_serializing_if = "RecoveryStatus::is_unknown")]
    pub recovery: RecoveryStatus,
    /// Wall-clock start of the native audio drain currently visible to IPC.
    /// This is presentation state only: a restarted daemon cannot resume an
    /// in-memory helper command from a persisted timestamp.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub draining_since_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<AccountSummary>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub audio_sources: Vec<AudioSource>,
    /// Why the helper's inventory of applications with audio is not shown,
    /// with its numbers. Absent while the list is fine or has
    /// not been read: an empty list with no reason is "none running", and
    /// one with a reason is "could not be verified", never the same.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio_sources_unavailable: Option<crate::refusal::Refusal>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recording_signal: Option<crate::audio_signal::RecordingSignalView>,
    /// The last voice note the daemon did not send for being too short,
    /// until the next note or message. Presentation only: a restart forgets it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub short_voice_note: Option<crate::voice::ShortVoiceNote>,
}

/// Durable authority is read-only outside orbit-core; see [`DurableState`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    #[serde(flatten)]
    pub durable: DurableState,
    #[serde(flatten)]
    pub views: Views,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ts",
    ts(export_to = "../../apps/desktop/src/bridge/generated/")
)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryStep {
    Chunks,
    UploadJobs,
    Snapshot,
    Environment,
    Operations,
    /// «Al detectar una llamada», read from the encrypted store.
    CallPreference,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum RecoveryStatus {
    #[default]
    Unknown,
    Recovering {
        step: RecoveryStep,
        elapsed_ms: u64,
    },
    Ready,
}

impl RecoveryStatus {
    pub fn is_unknown(&self) -> bool {
        matches!(self, Self::Unknown)
    }
}

fn is_zero_u64(value: &u64) -> bool {
    *value == 0
}

fn is_false(value: &bool) -> bool {
    !*value
}

impl From<DurableState> for State {
    fn from(durable: DurableState) -> Self {
        Self {
            durable,
            views: Views::default(),
        }
    }
}

impl State {
    pub fn generation(&self) -> u64 {
        self.durable.generation()
    }
    pub fn privacy_stopped(&self) -> bool {
        self.durable.privacy_stopped()
    }
    pub fn privacy_stop_by(&self) -> Option<crate::privacy_stop::StopRequester> {
        self.durable.privacy_stop_by()
    }
    pub fn connection(&self) -> Connection {
        self.durable.connection()
    }
    pub fn session_id(&self) -> Option<String> {
        self.durable.session_id()
    }
    /// The open conversation's agent; `None` is the hub's default.
    pub fn agent_id(&self) -> Option<&str> {
        self.durable.agent_id()
    }
    pub fn capabilities(&self) -> &Capabilities {
        self.durable.capabilities()
    }
    /// Voice processing and calendar count are observations, not grants of
    /// authority. The four permissions and helper presence remain decision-only.
    pub fn set_voice_processing_supported(&mut self, value: bool) {
        self.durable.capabilities.voice_processing_supported = value;
    }
    pub fn set_voice_processing_available(&mut self, value: bool) {
        self.durable.capabilities.voice_processing_available = value;
    }
    pub fn set_playback_reference_available(&mut self, value: bool) {
        self.durable.capabilities.playback_reference_available = value;
    }
    pub fn set_calendar_count(&mut self, count: Option<u32>) {
        self.durable.capabilities.calendars = count;
    }
    /// Whether a native helper is connected at all, so «nobody has asked
    /// yet» and «there is nobody to ask» stay distinguishable. The helper
    /// reports capabilities with every status, so a missing report means no
    /// helper; this reads that fact and never asks the system itself.
    pub fn helper_connected(&self) -> bool {
        self.durable.capabilities.native_helper
    }

    pub fn durable(&self) -> DurableState {
        self.durable.clone()
    }

    pub fn set_notice(&mut self, notice: impl Into<crate::notice::Notice>) {
        self.durable.set_notice(notice);
    }
    pub fn notice_code(&self) -> Option<crate::notice::NoticeCode> {
        self.durable.notice_code()
    }
    pub fn recovered(self) -> Self {
        self.durable.recovered().into()
    }
}

#[cfg(test)]
mod tests;
