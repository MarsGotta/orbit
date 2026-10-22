//! The agents a hub offers (hub-v1 «Agents»): who a conversation is with and
//! which names «Oye <nombre>» hears. Pure: the daemon passes in what the hub
//! said, keeps what this module admits, and asks it who is who.
use serde::{Deserialize, Serialize};

/// The id of the one agent of a hub that names none (0.9.0); that hub's
/// `assistant_name` is the agent's name. The hub uses the same id for its
/// implicit agent.
pub const IMPLICIT_AGENT_ID: &str = "default";

/// One agent as the interface shows it: its stable id, the name the user
/// reads and says aloud, and whether it reads aloud in a voice of its own.
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
}

/// The agents a hub said it has, in its order, and which one is its default.
/// Built only by [`AgentRoster::admit`] or [`AgentRoster::single`]; one read
/// back from disk (`hub-agents.toml`) is admitted again before it is used.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentRoster {
    default_agent_id: String,
    agents: Vec<Agent>,
}

/// Why a hub's list of agents was not taken. The Mac keeps the list it had.
/// Each cause carries its position and the count, never a name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RosterRefusal {
    Empty,
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
    /// whose ids are unique and whose default is one of them; a missing
    /// default is the first agent, as the hub's own rule says. The hub
    /// refuses an id's syntax and a name's length at its start, so the Mac
    /// does not repeat those bounds; the hub frame bounds the list's size.
    pub fn admit(
        agents: Vec<Agent>,
        default_agent_id: Option<String>,
    ) -> Result<Self, RosterRefusal> {
        let count = agents.len();
        if count == 0 {
            return Err(RosterRefusal::Empty);
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
        })
    }

    /// The one agent of a hub that names none, called by its `assistant_name`.
    pub fn single(name: &str) -> Self {
        Self {
            default_agent_id: IMPLICIT_AGENT_ID.into(),
            agents: vec![Agent {
                id: IMPLICIT_AGENT_ID.into(),
                name: name.into(),
                voice: None,
            }],
        }
    }

    /// A roster read back from disk, admitted again; `None` if it no longer holds.
    pub fn checked(self) -> Option<Self> {
        Self::admit(self.agents, Some(self.default_agent_id)).ok()
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
        Agent {
            id: id.into(),
            name: name.into(),
            voice: None,
        }
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
        ];
        for (agents, default, expected) in cases {
            assert_eq!(AgentRoster::admit(agents, default), Err(expected));
        }
        assert_eq!(
            RosterRefusal::UnknownDefault { count: 1 }.code(),
            "default_agent_unknown"
        );
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
