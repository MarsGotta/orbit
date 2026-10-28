//! The payload of each event the hub sends, as its `data`.
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReadyData {
    /// The default agent's name, kept for Macs that do not read `agents`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assistant_name: Option<String>,
    /// The hub's agents in its order; absent from a 0.9.0 hub, which has
    /// one, named by `assistant_name`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agents: Option<Vec<HubAgent>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_agent_id: Option<String>,
    /// The services an agent can be added with from Orbit (hub-v1
    /// «Assistants from Orbit»), possibly none; absent from a hub without
    /// those routes (0.10.1 and before).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub addable_agents: Option<Vec<String>>,
    /// The hub's attachment ceilings; absent from a 0.9.0 hub, which has no
    /// attachments.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attachments: Option<crate::attachments::AttachmentLimits>,
    /// What the hub's storage uses and has left; absent from a 0.10.1 hub.
    /// One the Mac cannot read is left out rather than refusing the `ready`
    /// that brings it: the link matters more than the warning.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "lenient_storage"
    )]
    pub storage: Option<HubStorage>,
    /// The hub deletes recordings (`DELETE /v1/recordings/{id}`); absent
    /// from a 0.11 hub, to which the Mac sends no delete.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recording_delete: Option<bool>,
    /// Read-only live-turn observation; absent on older hubs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chat_observe: Option<bool>,
    #[serde(default)]
    pub device_id: Option<String>,
    #[serde(default)]
    pub hub_protocol: Option<u64>,
    #[serde(default)]
    pub hub_version: Option<String>,
    #[serde(default)]
    pub device_version: Option<String>,
}
/// The hub's storage (`ready.storage`, `storage.status`): what the database
/// uses, its limit, what is left, what sets the limit (`disk` or `quota`)
/// and whether the hub counts that as nearly full.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HubStorage {
    pub used_bytes: u64,
    pub max_bytes: u64,
    pub remaining_bytes: u64,
    pub limit: String,
    pub nearly_full: bool,
}

fn lenient_storage<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<HubStorage>, D::Error> {
    let value = Value::deserialize(deserializer)?;
    Ok(serde_json::from_value(value).ok())
}

/// One agent as `ready` and `agents.changed` list it. The Mac reads `id`,
/// `name`, whether `capabilities` has `voice`, and its profile and account
/// (hub-v1 «Assistants from Orbit»), each absent from a hub before them;
/// `kind` and the rest travel for the contract.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HubAgent {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub capabilities: Vec<String>,
    /// What answers: `hermes`, `openai`, `claude-code`, `codex`, or one this
    /// Mac does not know yet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// `c1` to `c4`, or one this Mac does not know yet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    /// Added from Orbit, so Orbit can remove it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub removable: Option<bool>,
    /// `signed_in`, `signed_out`, `expired` or `unknown`; only an agent
    /// Orbit can sign in has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<String>,
    /// The account's address and the plan's name, when the agent's adapter said them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_email: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan: Option<String>,
    /// Present while the agent's plan is at its limit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<HubAgentLimit>,
    /// The plan's use, when the provider reported it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<crate::agents::AgentUsage>,
}
/// An agent at its plan's limit: when it resets, when the provider said.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HubAgentLimit {
    #[serde(default)]
    pub until_ms: Option<i64>,
}
/// The hub's agents changed (a profile, one added or removed, an account):
/// the whole list again, as `ready` gives it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentsChangedData {
    pub agents: Vec<HubAgent>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_agent_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub addable_agents: Option<Vec<String>>,
}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct EmptyData {}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionData {
    pub session_id: String,
}
/// Something of one of this device's recordings changed on the hub
/// (a group transcribed or failed, its note, its speakers' names, its private
/// notes). The Mac reads that recording again; the event carries no content.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecordingChangedData {
    pub recording_id: String,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeltaData {
    #[serde(default)]
    pub session_id: Option<String>,
    pub text: String,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CompletedData {
    #[serde(default)]
    pub session_id: Option<String>,
    pub text: String,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub persisted: Option<bool>,
    #[serde(default)]
    pub warning: Option<String>,
}
/// One step of the agent's work in a turn (hub-v1 «What the agent does»).
/// `kind` and `phase` stay text here: a value a newer hub adds is the
/// daemon's to skip, not a frame to refuse.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ItemData {
    pub session_id: String,
    pub item_id: String,
    pub kind: String,
    pub phase: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub steps: Option<Vec<ItemStep>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cause: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cause_code: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ItemStep {
    pub text: String,
    pub status: String,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StatusData {
    pub session_id: String,
    pub kind: String,
    pub text: String,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InterruptedData {
    pub session_id: String,
    pub status: String,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PendingInteraction {
    pub request_id: String,
    pub kind: String,
    pub state: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub question: Option<String>,
    #[serde(default)]
    pub expires_at: Option<u64>,
    #[serde(default)]
    pub choices: Option<Vec<String>>,
    #[serde(default)]
    pub questions: Option<Vec<Value>>,
    /// A command that deletes (`orbit_core::Deletion::from_wire`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deletes: Option<Value>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SnapshotSession {
    pub session_id: String,
    #[serde(default)]
    pub turn_id: Option<String>,
    pub status: String,
    pub pending: Vec<PendingInteraction>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn: Option<crate::Turn>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SnapshotData {
    pub revision: u64,
    pub sessions: Vec<SnapshotSession>,
    #[serde(default)]
    pub omitted: Option<bool>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RequiredData {
    pub session_id: String,
    pub request_id: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub question: Option<String>,
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub state: Option<String>,
    #[serde(default)]
    pub expires_at: Option<u64>,
    #[serde(default)]
    pub choices: Option<Vec<String>>,
    #[serde(default)]
    pub questions: Option<Vec<Value>>,
    /// A command that deletes (`orbit_core::Deletion::from_wire`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deletes: Option<Value>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InteractionData {
    pub session_id: String,
    pub request_id: String,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub applied: Option<bool>,
    #[serde(default)]
    pub resolved: Option<u64>,
    #[serde(default)]
    pub remaining: Option<Vec<String>>,
    #[serde(default)]
    pub stale_interaction: Option<bool>,
    #[serde(default)]
    pub turn_id: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ErrorData {
    #[serde(default)]
    pub code: Option<String>,
    pub message: String,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    /// Boxed: a refusal's fields would make every hub event as large as it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refusal: Option<Box<RefusalData>>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RefusalData {
    pub code: Option<String>,
    pub retry: RetryDisposition,
    #[serde(default)]
    pub operation: Option<String>,
    #[serde(default)]
    pub resource: Option<String>,
    #[serde(default)]
    pub bytes: Option<u64>,
    #[serde(default)]
    pub max_bytes: Option<u64>,
    #[serde(default)]
    pub retry_after_ms: Option<u64>,
    #[serde(default)]
    pub last_seen_ms: Option<u64>,
    #[serde(default)]
    pub waited_ms: Option<u64>,
    #[serde(default)]
    pub deadline_ms: Option<u64>,
    /// An attachment refusal's counts: the message's files and their
    /// ceiling, and what the conversation already holds (hub-v1
    /// «Attachments»).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub files: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_files: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_bytes: Option<u64>,
    /// Why a turn ended without an answer (hub-v1: the generic agent's
    /// `assistant_*` and an ACP agent's `agent_*` causes): the provider's
    /// HTTP status, the deadline that ran out, the context that did not fit,
    /// the model requests a turn took, the agent's own error code, exit
    /// status or stop reason.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_status: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connect_timeout_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub history_chars: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_requests: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub acp_code: Option<i64>,
    /// JSON-RPC error code from an official assistant tool. The older
    /// optional field above remains readable for already deployed hubs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rpc_code: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_status: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop_reason: Option<String>,
    /// When an agent's plan comes back from its limit
    /// (`agent_limit_reached`), when its official tool said.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resets_at_ms: Option<u64>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RetryDisposition {
    AfterMs { ms: u64 },
    AfterFix,
    CheckOutcome,
    Never,
}
