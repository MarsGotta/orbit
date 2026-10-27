//! The configuration of one agent, read and changed on the hub (hub-v1
//! «Agents»): its personality, its skills and which groups of Orbit's
//! tools it may use. The hub is the single source; the Mac keeps none of it.
//! Pure: where each order goes and what a refusal says are decided here, the
//! daemon only performs the request.
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Orbit's tool access groups. The original groups remain required; a
/// missing original group fails the read instead of being treated as off.
/// The newer notes group uses its historical on default for an older hub.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ts",
    ts(export_to = "../../apps/desktop/src/bridge/generated/")
)]
pub struct AccessGroups {
    pub screen: bool,
    pub control: bool,
    pub calendar: bool,
    pub meetings: bool,
    pub files: bool,
    // A pre-vault hub has no notes group. Keep the historical default on;
    // the current hub still enforces every read and stores its choice.
    #[serde(default = "notes_enabled")]
    pub notes: bool,
}

fn notes_enabled() -> bool {
    true
}

/// The agent's personality (Markdown) and whether its kind lets Orbit
/// change it; `max_bytes` is the hub's ceiling, shown before saving.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ts",
    ts(export_to = "../../apps/desktop/src/bridge/generated/")
)]
pub struct PersonalityConfig {
    pub text: String,
    pub editable: bool,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub max_bytes: u64,
    /// Why the field cannot be changed from Orbit, as the hub says it.
    #[serde(default)]
    pub cause: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ts",
    ts(export_to = "../../apps/desktop/src/bridge/generated/")
)]
pub struct AccessConfig {
    pub groups: AccessGroups,
    pub editable: bool,
    #[serde(default)]
    pub cause: Option<String>,
}

/// One skill of the agent's library, as the list shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ts",
    ts(export_to = "../../apps/desktop/src/bridge/generated/")
)]
pub struct SkillSummary {
    pub name: String,
    pub description: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ts",
    ts(export_to = "../../apps/desktop/src/bridge/generated/")
)]
pub struct SkillsConfig {
    pub editable: bool,
    #[serde(default)]
    pub cause: Option<String>,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub max_skills: u64,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub max_skill_bytes: u64,
    pub items: Vec<SkillSummary>,
}

/// `GET /v1/agents/{agent_id}/config`. Read tolerantly: a field a newer hub
/// adds does not refuse the read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ts",
    ts(export_to = "../../apps/desktop/src/bridge/generated/")
)]
pub struct AgentConfig {
    pub agent_id: String,
    /// `hermes`, `openai`, `claude-code`, `codex`, or a kind this Mac does not know yet.
    pub kind: String,
    /// Absolute hub workspace; absent on older hubs and other agent kinds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub working_folder: Option<String>,
    pub personality: PersonalityConfig,
    pub access: AccessConfig,
    pub skills: SkillsConfig,
}

/// `GET /v1/agents/{agent_id}/skills/{name}`: the whole `SKILL.md`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ts",
    ts(export_to = "../../apps/desktop/src/bridge/generated/")
)]
pub struct SkillDocument {
    pub name: String,
    pub description: String,
    pub enabled: bool,
    pub content: String,
}

/// What Ajustes › Agentes asks of the hub, one route each. Reads and orders
/// alike go as the user gives them; nothing is queued or retried on the Mac.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "call", rename_all = "snake_case", deny_unknown_fields)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ts",
    ts(export_to = "../../apps/desktop/src/bridge/generated/")
)]
pub enum AgentCall {
    AiSettings,
    AiProviderSave {
        provider_id: String,
        provider: crate::ai_config::AiProviderInput,
    },
    AiFunctionSave {
        function: String,
        choice: crate::ai_config::AiChoice,
    },
    TranscriptionSettings,
    TranscriptionSave {
        model: crate::transcription_config::TranscriptionModel,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        api_key: Option<crate::transcription_config::TranscriptionKey>,
    },
    Config {
        agent_id: String,
    },
    WorkingFolder {
        agent_id: String,
        path: String,
    },
    Personality {
        agent_id: String,
        text: String,
    },
    /// The six groups as the user left them; a switch is the decision.
    Access {
        agent_id: String,
        groups: AccessGroups,
    },
    Skill {
        agent_id: String,
        name: String,
    },
    /// Creates the skill or replaces its `SKILL.md`.
    SkillSave {
        agent_id: String,
        name: String,
        content: String,
    },
    SkillEnabled {
        agent_id: String,
        name: String,
        enabled: bool,
    },
    SkillDelete {
        agent_id: String,
        name: String,
    },
    /// The agent's profile (hub-v1 «Assistants from Orbit»): how Orbit
    /// shows and calls it, and whether it is the primary.
    Profile {
        agent_id: String,
        name: String,
        description: String,
        color: String,
        primary: bool,
    },
    /// A Claude Code (`claude-code`) or Codex (`codex`) agent on the hub's
    /// own adapter: the Mac chooses only these.
    Add {
        service: String,
        name: String,
        description: String,
        color: String,
        access: AccessGroups,
    },
    /// An agent added from Orbit; its conversations stay.
    Remove {
        agent_id: String,
    },
    /// The vendor's sign-in, run by the hub: the address to open.
    SignIn {
        agent_id: String,
    },
    SignInStatus {
        agent_id: String,
    },
    /// Claude Code's: the code its page shows, `<code>#<state>`. Never logged.
    SignInCode {
        agent_id: String,
        code: String,
    },
    SignInCancel {
        agent_id: String,
    },
    SignOut {
        agent_id: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HubMethod {
    Get,
    Put,
    Post,
    Delete,
}

/// Where one sign-in is, as the hub says it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ts",
    ts(export_to = "../../apps/desktop/src/bridge/generated/")
)]
pub enum SignInState {
    /// The user has the address (and the code) to sign in with.
    Waiting,
    Done,
    Cancelled,
    /// `cause` says why.
    Failed,
    /// A state a newer hub added.
    #[serde(other)]
    Unknown,
}

/// `POST|GET|DELETE /v1/agents/{agent_id}/sign-in`. Read tolerantly.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ts",
    ts(export_to = "../../apps/desktop/src/bridge/generated/")
)]
pub struct SignInAttempt {
    pub agent_id: String,
    pub state: SignInState,
    /// The address the Mac opens in the browser, while waiting.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub url: Option<String>,
    /// Codex's one-time code, shown to the user to type on that page.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub code: Option<String>,
    /// Claude Code waits for the code its page shows (`SignInCode`).
    #[serde(default)]
    pub expects_code: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional, type = "number"))]
    pub expires_at_ms: Option<i64>,
    /// Why it failed: `plan_required`, `device_code_disabled`, `expired`, `code_rejected`,
    /// `timed_out`, `no_url`, `exited`…
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub cause: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional, type = "number"))]
    pub exit_status: Option<i64>,
}

/// Where one call goes: its path segments (each encoded by the adapter, so
/// an id or a name is never read as a path) and its JSON body.
#[derive(Debug, Clone, PartialEq)]
pub struct HubRoute {
    pub method: HubMethod,
    pub segments: Vec<String>,
    pub body: Option<serde_json::Value>,
}

impl AgentCall {
    /// The agent the call is about; `None` for one that adds an agent.
    pub fn agent_id(&self) -> Option<&str> {
        match self {
            Self::Config { agent_id }
            | Self::WorkingFolder { agent_id, .. }
            | Self::Personality { agent_id, .. }
            | Self::Access { agent_id, .. }
            | Self::Skill { agent_id, .. }
            | Self::SkillSave { agent_id, .. }
            | Self::SkillEnabled { agent_id, .. }
            | Self::SkillDelete { agent_id, .. }
            | Self::Profile { agent_id, .. }
            | Self::Remove { agent_id }
            | Self::SignIn { agent_id }
            | Self::SignInStatus { agent_id }
            | Self::SignInCode { agent_id, .. }
            | Self::SignInCancel { agent_id }
            | Self::SignOut { agent_id } => Some(agent_id),
            Self::AiSettings
            | Self::AiProviderSave { .. }
            | Self::AiFunctionSave { .. }
            | Self::Add { .. }
            | Self::TranscriptionSettings
            | Self::TranscriptionSave { .. } => None,
        }
    }

    pub fn route(&self) -> HubRoute {
        let agent = |tail: &[&str]| agent_segments(self.agent_id(), tail);
        let (method, segments, body) = match self {
            Self::AiSettings => crate::ai_config::read_route(),
            Self::AiProviderSave {
                provider_id,
                provider,
            } => crate::ai_config::provider_route(provider_id, provider),
            Self::AiFunctionSave { function, choice } => {
                crate::ai_config::function_route(function, choice)
            }
            Self::TranscriptionSettings => transcription_route(HubMethod::Get, None),
            Self::TranscriptionSave { model, api_key } => transcription_route(
                HubMethod::Put,
                Some(serde_json::json!({ "model": model, "api_key": api_key })),
            ),
            Self::Config { .. } => (HubMethod::Get, agent(&["config"]), None),
            Self::WorkingFolder { path, .. } => {
                working_folder_route(agent(&["working-folder"]), path)
            }
            Self::Personality { text, .. } => (
                HubMethod::Put,
                agent(&["personality"]),
                Some(serde_json::json!({ "text": text })),
            ),
            Self::Access { groups, .. } => (
                HubMethod::Put,
                agent(&["access"]),
                Some(serde_json::json!({ "groups": groups })),
            ),
            Self::Skill { name, .. } => (HubMethod::Get, agent(&["skills", name]), None),
            Self::SkillSave { name, content, .. } => (
                HubMethod::Put,
                agent(&["skills", name]),
                Some(serde_json::json!({ "content": content })),
            ),
            Self::SkillEnabled { name, enabled, .. } => (
                HubMethod::Put,
                agent(&["skills", name, "enabled"]),
                Some(serde_json::json!({ "enabled": enabled })),
            ),
            Self::SkillDelete { name, .. } => (HubMethod::Delete, agent(&["skills", name]), None),
            Self::Profile {
                name,
                description,
                color,
                primary,
                ..
            } => profile_route(agent(&["profile"]), name, description, color, *primary),
            Self::Add {
                service,
                name,
                description,
                color,
                access,
            } => (
                HubMethod::Post,
                agent(&[]),
                Some(serde_json::json!({
                    "service": service, "name": name, "description": description,
                    "color": color, "access": access
                })),
            ),
            Self::Remove { .. } => (HubMethod::Delete, agent(&[]), None),
            Self::SignIn { .. } => (HubMethod::Post, agent(&["sign-in"]), None),
            Self::SignInStatus { .. } => (HubMethod::Get, agent(&["sign-in"]), None),
            Self::SignInCode { code, .. } => (
                HubMethod::Post,
                agent(&["sign-in", "code"]),
                Some(serde_json::json!({ "code": code })),
            ),
            Self::SignInCancel { .. } => (HubMethod::Delete, agent(&["sign-in"]), None),
            Self::SignOut { .. } => (HubMethod::Post, agent(&["sign-out"]), None),
        };
        HubRoute {
            method,
            segments,
            body,
        }
    }
}

/// One number of a hub refusal: hub-v1 keeps them scalar (integers, text,
/// booleans) so the Mac never decodes a list there.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ts",
    ts(export_to = "../../apps/desktop/src/bridge/generated/")
)]
pub enum RefusalNumber {
    Flag(bool),
    Integer(#[cfg_attr(feature = "ts", ts(type = "number"))] i64),
    Text(String),
}

/// The hub's own refusal of a call: its code, the HTTP status and every
/// scalar number (`field`, `rule`, `bytes`, `max_bytes`…), carried whole to
/// the screen.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ts",
    ts(export_to = "../../apps/desktop/src/bridge/generated/")
)]
pub struct AgentRefusal {
    pub code: String,
    pub status: u16,
    pub numbers: BTreeMap<String, RefusalNumber>,
}

/// A hub code is snake_case ASCII; anything else is not echoed as one.
/// 64 bytes is the daemon's bound on a hub code (`MAX_HUB_CODE_BYTES`).
fn is_code(code: &str) -> bool {
    !code.is_empty()
        && code.len() <= 64
        && code
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte == b'_')
}

impl AgentRefusal {
    /// The refusal in a hub error envelope (`code`, `message`, `refusal`);
    /// `None` when it carries no code, as an older hub's unknown route.
    pub fn from_envelope(status: u16, envelope: &serde_json::Value) -> Option<Self> {
        let code = envelope
            .get("code")?
            .as_str()
            .filter(|code| is_code(code))?;
        let numbers = envelope
            .get("refusal")
            .and_then(serde_json::Value::as_object)
            .map(|fields| {
                fields
                    .iter()
                    .filter(|(key, _)| !matches!(key.as_str(), "code" | "retry"))
                    .filter_map(|(key, value)| {
                        let number = match value {
                            serde_json::Value::Bool(flag) => RefusalNumber::Flag(*flag),
                            serde_json::Value::Number(number) => {
                                RefusalNumber::Integer(number.as_i64()?)
                            }
                            serde_json::Value::String(text) => RefusalNumber::Text(text.clone()),
                            _ => return None,
                        };
                        Some((key.clone(), number))
                    })
                    .collect()
            })
            .unwrap_or_default();
        Some(Self {
            code: code.to_owned(),
            status,
            numbers,
        })
    }
}

/// What came of one call. `Failed` is the Mac's side: it could not ask the
/// hub (not paired, privacy stop, no link) or could not read its answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ts",
    ts(export_to = "../../apps/desktop/src/bridge/generated/")
)]
pub enum AgentOutcome {
    AiSettings {
        settings: crate::ai_config::AiSettings,
    },
    TranscriptionSettings {
        settings: crate::transcription_config::TranscriptionSettings,
    },
    Config {
        config: AgentConfig,
    },
    Skill {
        skill: SkillDocument,
    },
    /// The hub accepted the order. The agents it changed arrive as
    /// `agents.changed`, on the status topic.
    Done,
    /// The agent the hub added, by the id it made.
    Added {
        agent_id: String,
    },
    SignIn {
        attempt: SignInAttempt,
    },
    Refused {
        refusal: AgentRefusal,
    },
    Failed {
        #[cfg_attr(feature = "ts", ts(type = "Record<string, unknown>"))]
        refusal: crate::refusal::Refusal,
    },
}

#[cfg(test)]
mod tests;

fn working_folder_route(
    segments: Vec<String>,
    path: &str,
) -> (HubMethod, Vec<String>, Option<serde_json::Value>) {
    (
        HubMethod::Put,
        segments,
        Some(serde_json::json!({ "path": path })),
    )
}

fn transcription_route(
    method: HubMethod,
    body: Option<serde_json::Value>,
) -> (HubMethod, Vec<String>, Option<serde_json::Value>) {
    (
        method,
        ["v1", "settings", "transcription"]
            .map(str::to_owned)
            .into(),
        body,
    )
}

fn profile_route(
    segments: Vec<String>,
    name: &str,
    description: &str,
    color: &str,
    primary: bool,
) -> (HubMethod, Vec<String>, Option<serde_json::Value>) {
    (
        HubMethod::Put,
        segments,
        Some(
            serde_json::json!({"name": name, "description": description, "color": color, "primary": primary}),
        ),
    )
}

fn agent_segments(agent_id: Option<&str>, tail: &[&str]) -> Vec<String> {
    ["v1", "agents"]
        .into_iter()
        .chain(agent_id)
        .chain(tail.iter().copied())
        .map(str::to_owned)
        .collect()
}
