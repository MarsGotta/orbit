//! The configuration of one agent, read and changed on the hub (hub-v1
//! «Agents»): its personality, its skills and which groups of Orbit's
//! tools it may use. The hub is the single source; the Mac keeps none of it.
//! Pure: where each order goes and what a refusal says are decided here, the
//! daemon only performs the request.
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// The five groups of Orbit's tools an agent may use. All five travel
/// together, on reads and writes: a group missing from a hub's answer is
/// unknown, never off, so it fails the read instead of reading as `false`.
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
    /// `hermes`, `openai`, `acp`, or a kind this Mac does not know yet.
    pub kind: String,
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
    Config {
        agent_id: String,
    },
    Personality {
        agent_id: String,
        text: String,
    },
    /// The five groups as the user left them; a switch is the decision.
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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HubMethod {
    Get,
    Put,
    Delete,
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
    pub fn agent_id(&self) -> &str {
        match self {
            Self::Config { agent_id }
            | Self::Personality { agent_id, .. }
            | Self::Access { agent_id, .. }
            | Self::Skill { agent_id, .. }
            | Self::SkillSave { agent_id, .. }
            | Self::SkillEnabled { agent_id, .. }
            | Self::SkillDelete { agent_id, .. } => agent_id,
        }
    }

    pub fn route(&self) -> HubRoute {
        let agent = |tail: &[&str]| {
            ["v1", "agents", self.agent_id()]
                .iter()
                .chain(tail)
                .map(|segment| (*segment).to_owned())
                .collect::<Vec<_>>()
        };
        let (method, segments, body) = match self {
            Self::Config { .. } => (HubMethod::Get, agent(&["config"]), None),
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
    Config {
        config: AgentConfig,
    },
    Skill {
        skill: SkillDocument,
    },
    /// The hub accepted the order.
    Done,
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
