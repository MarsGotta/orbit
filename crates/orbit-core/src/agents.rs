//! The agents a hub offers (hub-v1 «Agents»): who a conversation is with and
//! which names «Oye <nombre>» hears. Pure: the daemon passes in what the hub
//! said, keeps what this module admits, and asks it who is who.
use serde::{Deserialize, Serialize};

/// The id of the one agent of a hub that names none (0.9.0); that hub's
/// `assistant_name` is the agent's name. The hub uses the same id for its
/// implicit agent.
pub const IMPLICIT_AGENT_ID: &str = "default";

/// The agents the Mac takes from one hub. Every one rides the status topic,
/// whose frame it protects: 32 at the hub's own bounds (a 32-byte id, a
/// 48-character name, a 90-character description, each character up to four
/// bytes, and the account and usage) take about 33 KiB of it (`TopicId::Status`).
/// Sixteen times the two the self-hosting guide configures; not measured
/// with users. The hub holds the same ceiling (`roster.MAX_AGENTS`), refusing
/// to start or to add one past it. A longer list is refused with its count,
/// and the Mac keeps the list it had.
pub const MAX_AGENTS: usize = 32;

/// One agent as the interface shows it: its stable id, the name the user
/// reads and says aloud, whether it reads aloud in a voice of its own, and
/// what the hub keeps of it (hub-v1 «Assistants from Orbit»): its profile,
/// whether Orbit can remove it, and its account. A hub before that says
/// none of the last, and a list kept by an older Mac has none either.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ts",
    ts(export_to = "../../apps/desktop/src/bridge/generated/")
)]
pub struct Agent {
    pub id: String,
    pub name: String,
    /// `voice` among its `capabilities` in `ready`. `None` when the hub did
    /// not say: the one agent of a hub that lists none (0.9.0), or a list
    /// kept by an older Mac. Not saying is not «no voice».
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub voice: Option<bool>,
    /// What answers: `hermes`, `openai`, `claude-code`, `codex`, or one this
    /// Mac does not know yet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub service: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub description: Option<String>,
    /// `c1` to `c4`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub color: Option<String>,
    /// Added from Orbit: Orbit can remove it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    #[cfg_attr(feature = "ts", ts(as = "Option<bool>", optional))]
    pub removable: bool,
    /// Only for an agent Orbit can sign in (Claude Code, Codex).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub account: Option<AgentAccount>,
    /// The account's address and the plan's name, when its adapter said them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub account_email: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub plan: Option<String>,
    /// Present while its plan is at its limit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub limit: Option<AgentLimit>,
    /// Present only when its provider reported it: no report, no bar.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub usage: Option<AgentUsage>,
}

/// The share of a plan's window used, as the provider last reported it
/// (today only Claude Code with a claude.ai subscription, when it changes).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ts",
    ts(export_to = "../../apps/desktop/src/bridge/generated/")
)]
pub struct AgentUsage {
    /// 0 to 100.
    pub used_percent: u8,
    /// `five_hour`, `seven_day`… as the provider names it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub window: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional, type = "number"))]
    pub resets_at_ms: Option<i64>,
}

/// What the hub last learned of an agent's account on the server.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ts",
    ts(export_to = "../../apps/desktop/src/bridge/generated/")
)]
pub enum AgentAccount {
    SignedIn,
    SignedOut,
    /// Its session on the server expired: it answers again once signed in.
    Expired,
    /// The hub has not learned it yet, or said something this Mac does not know.
    #[serde(other)]
    Unknown,
}

/// An agent at its plan's limit; `until_ms` is when it resets, when the
/// provider said. Not saying is not «no limit».
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ts",
    ts(export_to = "../../apps/desktop/src/bridge/generated/")
)]
pub struct AgentLimit {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional, type = "number"))]
    pub until_ms: Option<i64>,
}

impl AgentAccount {
    /// The hub's word for it; one this Mac does not know is `Unknown`.
    pub fn from_wire(text: &str) -> Self {
        match text {
            "signed_in" => Self::SignedIn,
            "signed_out" => Self::SignedOut,
            "expired" => Self::Expired,
            _ => Self::Unknown,
        }
    }
}

impl Agent {
    /// An agent known only by its id and name, as a hub before profiles lists it.
    pub fn named(id: &str, name: &str) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            voice: None,
            service: None,
            description: None,
            color: None,
            removable: false,
            account: None,
            account_email: None,
            plan: None,
            limit: None,
            usage: None,
        }
    }
}

/// The agents a hub said it has, in its order, and which one is its default.
/// Built only by [`AgentRoster::admit`] or [`AgentRoster::single`]; one read
/// back from disk (`hub-agents.toml`) is admitted again before it is used.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentRoster {
    default_agent_id: String,
    agents: Vec<Agent>,
    /// The services an agent can be added with from Orbit; `None` from a hub
    /// without «Assistants from Orbit», which adds none and hands no
    /// conversation over.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    addable: Option<Vec<String>>,
}

/// Why a hub's list of agents was not taken. The Mac keeps the list it had.
/// Each cause carries its position and the count, never a name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RosterRefusal {
    Empty,
    /// More than [`MAX_AGENTS`].
    TooMany {
        count: usize,
    },
    BlankId {
        position: usize,
        count: usize,
    },
    BlankName {
        position: usize,
        count: usize,
    },
    RepeatedId {
        first: usize,
        second: usize,
        count: usize,
    },
    /// `default_agent_id` names no agent of the list.
    UnknownDefault {
        count: usize,
    },
}

impl RosterRefusal {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Empty => "agents_empty",
            Self::TooMany { .. } => "agents_too_many",
            Self::BlankId { .. } => "agent_id_blank",
            Self::BlankName { .. } => "agent_name_blank",
            Self::RepeatedId { .. } => "agent_id_repeated",
            Self::UnknownDefault { .. } => "default_agent_unknown",
        }
    }

    /// The numbers of the cause, for the daemon's log.
    pub fn numbers(&self) -> Vec<(&'static str, usize)> {
        match *self {
            Self::Empty => vec![("count", 0)],
            Self::TooMany { count } => vec![("count", count), ("max", MAX_AGENTS)],
            Self::BlankId { position, count } | Self::BlankName { position, count } => {
                vec![("position", position), ("count", count)]
            }
            Self::RepeatedId {
                first,
                second,
                count,
            } => vec![("first", first), ("second", second), ("count", count)],
            Self::UnknownDefault { count } => vec![("count", count)],
        }
    }
}

impl AgentRoster {
    /// The list a hub sent with `ready`. The Mac relies on a non-empty list
    /// of at most [`MAX_AGENTS`] whose ids are unique and whose default is
    /// one of them; a missing default is the first agent, as the hub's own
    /// rule says. The hub refuses an id's syntax and a name's length at its
    /// start, so the Mac does not repeat those bounds.
    pub fn admit(
        agents: Vec<Agent>,
        default_agent_id: Option<String>,
    ) -> Result<Self, RosterRefusal> {
        let count = agents.len();
        if count == 0 {
            return Err(RosterRefusal::Empty);
        }
        if count > MAX_AGENTS {
            return Err(RosterRefusal::TooMany { count });
        }
        for (index, agent) in agents.iter().enumerate() {
            let position = index + 1;
            if agent.id.trim().is_empty() {
                return Err(RosterRefusal::BlankId { position, count });
            }
            if agent.name.trim().is_empty() {
                return Err(RosterRefusal::BlankName { position, count });
            }
            if let Some(first) = agents[..index]
                .iter()
                .position(|other| other.id == agent.id)
            {
                return Err(RosterRefusal::RepeatedId {
                    first: first + 1,
                    second: position,
                    count,
                });
            }
        }
        let default_agent_id = match default_agent_id {
            Some(id) if agents.iter().any(|agent| agent.id == id) => id,
            Some(_) => return Err(RosterRefusal::UnknownDefault { count }),
            None => agents[0].id.clone(),
        };
        Ok(Self {
            default_agent_id,
            agents,
            addable: None,
        })
    }

    /// The services the hub said agents can be added with (`addable_agents`).
    pub fn with_addable(self, addable: Option<Vec<String>>) -> Self {
        Self { addable, ..self }
    }

    pub fn addable(&self) -> Option<&[String]> {
        self.addable.as_deref()
    }

    /// The one agent of a hub that names none, called by its `assistant_name`.
    pub fn single(name: &str) -> Self {
        Self {
            default_agent_id: IMPLICIT_AGENT_ID.into(),
            agents: vec![Agent::named(IMPLICIT_AGENT_ID, name)],
            addable: None,
        }
    }

    /// A roster read back from disk, admitted again; `None` if it no longer holds.
    pub fn checked(self) -> Option<Self> {
        let addable = self.addable;
        Self::admit(self.agents, Some(self.default_agent_id))
            .ok()
            .map(|roster| roster.with_addable(addable))
    }

    pub fn agents(&self) -> &[Agent] {
        &self.agents
    }

    pub fn default_agent_id(&self) -> &str {
        &self.default_agent_id
    }

    pub fn agent(&self, id: &str) -> Option<&Agent> {
        self.agents.iter().find(|agent| agent.id == id)
    }

    /// The default agent's name; admission guarantees it is in the list.
    pub fn default_name(&self) -> &str {
        self.agent(&self.default_agent_id)
            .map_or("", |agent| agent.name.as_str())
    }

    /// The agent whose voice reads `agent_id`'s answers aloud: its own, or
    /// the default agent's for one that says it has none, as the hub falls
    /// back. One that does not say is read in its own, which the hub tries
    /// first.
    pub fn reading_voice<'a>(&'a self, agent_id: &'a str) -> &'a str {
        match self.agent(agent_id) {
            Some(Agent {
                voice: Some(false), ..
            }) => &self.default_agent_id,
            _ => agent_id,
        }
    }

    /// The agent whose name the helper heard. The helper answers with the
    /// name exactly as it was given in `assistant_names`.
    pub fn named(&self, name: &str) -> Option<&Agent> {
        self.agents.iter().find(|agent| agent.name == name)
    }

    /// Every name the wake phrase listens for, in the hub's order.
    pub fn names(&self) -> Vec<String> {
        self.agents.iter().map(|agent| agent.name.clone()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agent(id: &str, name: &str) -> Agent {
        Agent::named(id, name)
    }

    #[test]
    fn a_hub_list_is_taken_in_its_order_with_its_default() {
        let roster = AgentRoster::admit(
            vec![agent("deimos", "Deimos"), agent("atlas", "Atlas")],
            Some("atlas".into()),
        )
        .unwrap_or_else(|refusal| panic!("{refusal:?}"));
        assert_eq!(roster.default_agent_id(), "atlas");
        assert_eq!(roster.default_name(), "Atlas");
        assert_eq!(roster.names(), ["Deimos", "Atlas"]);
        assert_eq!(roster.named("Atlas").map(|a| a.id.as_str()), Some("atlas"));
        assert_eq!(
            roster.named("atlas"),
            None,
            "the helper answers the name as given"
        );
        // Without a default, the first one is, as the hub's rule says.
        let first = AgentRoster::admit(vec![agent("vega", "Vega")], None)
            .unwrap_or_else(|refusal| panic!("{refusal:?}"));
        assert_eq!(first.default_agent_id(), "vega");
    }

    #[test]
    fn a_list_the_mac_cannot_rely_on_says_why_with_its_numbers() {
        let cases = [
            (vec![], None, RosterRefusal::Empty),
            (
                vec![agent("deimos", "Deimos"), agent(" ", "Atlas")],
                None,
                RosterRefusal::BlankId {
                    position: 2,
                    count: 2,
                },
            ),
            (
                vec![agent("deimos", "")],
                None,
                RosterRefusal::BlankName {
                    position: 1,
                    count: 1,
                },
            ),
            (
                vec![
                    agent("deimos", "Deimos"),
                    agent("atlas", "Atlas"),
                    agent("deimos", "Otro"),
                ],
                None,
                RosterRefusal::RepeatedId {
                    first: 1,
                    second: 3,
                    count: 3,
                },
            ),
            (
                vec![agent("deimos", "Deimos")],
                Some("atlas".to_owned()),
                RosterRefusal::UnknownDefault { count: 1 },
            ),
            (
                (0..=MAX_AGENTS)
                    .map(|index| agent(&format!("agente-{index}"), "Agente"))
                    .collect(),
                None,
                RosterRefusal::TooMany {
                    count: MAX_AGENTS + 1,
                },
            ),
        ];
        for (agents, default, expected) in cases {
            assert_eq!(AgentRoster::admit(agents, default), Err(expected));
        }
        assert_eq!(
            RosterRefusal::UnknownDefault { count: 1 }.code(),
            "default_agent_unknown"
        );
        assert_eq!(
            RosterRefusal::TooMany { count: 33 }.numbers(),
            [("count", 33), ("max", MAX_AGENTS)]
        );
        let full = (0..MAX_AGENTS)
            .map(|index| agent(&format!("agente-{index}"), "Agente"))
            .collect();
        assert!(AgentRoster::admit(full, None).is_ok());
    }

    #[test]
    fn a_hub_that_names_no_agent_has_one_called_by_its_name() {
        let roster = AgentRoster::single("Atlas");
        assert_eq!(roster.default_agent_id(), IMPLICIT_AGENT_ID);
        assert_eq!(roster.default_name(), "Atlas");
        assert_eq!(roster.names(), ["Atlas"]);
        assert_eq!(roster.clone().checked(), Some(roster));
    }

    #[test]
    fn an_agent_without_a_voice_is_read_in_the_default_agents() {
        let voiced = |id: &str, voice: Option<bool>| Agent {
            voice,
            ..agent(id, id)
        };
        let roster = AgentRoster::admit(
            vec![
                voiced("deimos", Some(true)),
                voiced("atlas", Some(true)),
                voiced("nova", Some(false)),
                voiced("vega", None),
            ],
            Some("deimos".into()),
        )
        .unwrap_or_else(|refusal| panic!("{refusal:?}"));
        assert_eq!(roster.reading_voice("atlas"), "atlas");
        assert_eq!(roster.reading_voice("nova"), "deimos");
        assert_eq!(roster.reading_voice("deimos"), "deimos");
        assert_eq!(
            roster.reading_voice("vega"),
            "vega",
            "an agent that does not say is tried in its own voice first"
        );
        let single = AgentRoster::single("Vega");
        assert_eq!(single.reading_voice(IMPLICIT_AGENT_ID), IMPLICIT_AGENT_ID);
    }
}
