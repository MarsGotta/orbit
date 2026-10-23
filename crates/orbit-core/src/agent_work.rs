//! What the agent did in a turn (hub-v1 `chat.item`), as the Mac keeps it for
//! the chat: its newest steps, how many tools ran and what they used, and
//! when the turn started and ended. Pure: the caller passes the times.
use serde::{Deserialize, Serialize};

/// The steps kept for one turn. They ride the assistant topic beside the
/// turn: sixteen items at the ceilings below take about 70 KiB in four-byte
/// characters, and the one plan's twenty steps 40 KiB more
/// (`TopicId::Assistant`). Older steps leave the list but stay counted in
/// `tools` and `subjects`, which the folded summary uses. It does not limit
/// what the agent does.
pub const MAX_WORK_ITEMS: usize = 16;
/// The distinct things the summary names («la pantalla y el calendario»);
/// past eight a sentence is no longer read at a glance.
pub const MAX_WORK_SUBJECTS: usize = 8;
/// The hub's own ceilings for one `chat.item` (`orbit_hub.assistant.activity`,
/// registered in contracts/constants.json), applied again here because the
/// item comes from outside: they keep a hub that sends more from filling the
/// assistant topic, which would then be refused whole.
pub const MAX_ITEM_LABEL_CHARS: usize = 500;
pub const MAX_ITEM_SUBJECT_CHARS: usize = 80;
pub const MAX_ITEM_CAUSE_CHARS: usize = 300;
pub const MAX_PLAN_STEPS: usize = 20;
/// A step's id, which the hub passes on from the agent unbounded (an ACP
/// `toolCallId`). Ids seen are a few dozen characters; 128 keeps one that is
/// not from filling the topic. Only the Mac's copy is cut.
pub const MAX_ITEM_ID_CHARS: usize = 128;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ts",
    ts(export_to = "../../apps/desktop/src/bridge/generated/")
)]
#[serde(rename_all = "snake_case")]
pub enum WorkKind {
    Tool,
    Reasoning,
    Plan,
    Subagent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ts",
    ts(export_to = "../../apps/desktop/src/bridge/generated/")
)]
#[serde(rename_all = "snake_case")]
pub enum WorkPhase {
    Started,
    Progress,
    Completed,
    Failed,
}

impl WorkPhase {
    pub fn ended(self) -> bool {
        matches!(self, Self::Completed | Self::Failed)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ts",
    ts(export_to = "../../apps/desktop/src/bridge/generated/")
)]
#[serde(rename_all = "snake_case")]
pub enum WorkStepStatus {
    Pending,
    InProgress,
    Completed,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ts",
    ts(export_to = "../../apps/desktop/src/bridge/generated/")
)]
pub struct WorkStep {
    pub text: String,
    pub status: WorkStepStatus,
}

/// One step, as its last `chat.item` described it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ts",
    ts(export_to = "../../apps/desktop/src/bridge/generated/")
)]
pub struct WorkItem {
    pub id: String,
    pub kind: WorkKind,
    pub phase: WorkPhase,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub subject: Option<String>,
    /// A plan's steps; empty for any other kind.
    #[serde(default)]
    pub steps: Vec<WorkStep>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub cause: Option<String>,
}

fn cut(text: &str, limit: usize) -> String {
    match text.char_indices().nth(limit) {
        Some((end, _)) => text[..end].to_owned(),
        None => text.to_owned(),
    }
}

impl WorkItem {
    /// The step a `chat.item` describes, within the ceilings above, with
    /// steps only for a plan; `None` for a kind, phase or step status this
    /// Mac does not know.
    pub fn from_wire(data: &crate::wire::hub::ItemData) -> Option<Self> {
        fn known<T: serde::de::DeserializeOwned>(value: &str) -> Option<T> {
            serde_json::from_value(serde_json::Value::String(value.to_owned())).ok()
        }
        let kind: WorkKind = known(&data.kind)?;
        let steps = data
            .steps
            .iter()
            .flatten()
            .filter(|_| kind == WorkKind::Plan)
            .take(MAX_PLAN_STEPS)
            .map(|step| {
                Some(WorkStep {
                    text: cut(&step.text, MAX_ITEM_LABEL_CHARS),
                    status: known(&step.status)?,
                })
            })
            .collect::<Option<Vec<_>>>()?;
        Some(Self {
            id: cut(&data.item_id, MAX_ITEM_ID_CHARS),
            kind,
            phase: known(&data.phase)?,
            label: cut(&data.label, MAX_ITEM_LABEL_CHARS),
            subject: data
                .subject
                .as_deref()
                .map(|subject| cut(subject, MAX_ITEM_SUBJECT_CHARS)),
            steps,
            cause: data
                .cause
                .as_deref()
                .map(|cause| cut(cause, MAX_ITEM_CAUSE_CHARS)),
        })
    }
}

/// The work of one turn, named by the turn's id (its `chat.send` UUID).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ts",
    ts(export_to = "../../apps/desktop/src/bridge/generated/")
)]
pub struct AgentWork {
    pub turn_id: String,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub started_at_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional, type = "number"))]
    pub ended_at_ms: Option<u64>,
    pub items: Vec<WorkItem>,
    /// Tool steps that ended, the ones no longer listed included.
    pub tools: u32,
    /// What those tools used, each once, in order.
    pub subjects: Vec<String>,
}

impl AgentWork {
    pub fn new(turn_id: impl Into<String>, started_at_ms: u64) -> Self {
        Self {
            turn_id: turn_id.into(),
            started_at_ms,
            ended_at_ms: None,
            items: Vec::new(),
            tools: 0,
            subjects: Vec::new(),
        }
    }

    /// The step's latest state replaces its earlier one, in place. A turn
    /// has one plan, the latest the agent sent, whatever its id.
    pub fn record(&mut self, item: WorkItem) {
        let index = self.items.iter().position(|known| {
            known.id == item.id || (item.kind == WorkKind::Plan && known.kind == WorkKind::Plan)
        });
        let ended_before = index.is_some_and(|index| self.items[index].phase.ended());
        if item.kind == WorkKind::Tool {
            if item.phase.ended() && !ended_before {
                self.tools = self.tools.saturating_add(1);
            }
            if let Some(subject) = &item.subject
                && self.subjects.len() < MAX_WORK_SUBJECTS
                && !self.subjects.contains(subject)
            {
                self.subjects.push(subject.clone());
            }
        }
        match index {
            Some(index) => self.items[index] = item,
            None => self.items.push(item),
        }
        if self.items.len() > MAX_WORK_ITEMS {
            // The oldest step that ended leaves first; a running one stays in view.
            let oldest = self
                .items
                .iter()
                .position(|known| known.phase.ended())
                .unwrap_or(0);
            self.items.remove(oldest);
        }
    }

    /// The turn ended at `at_ms`; the first end is the one kept.
    pub fn end(&mut self, at_ms: u64) {
        if self.ended_at_ms.is_none() {
            self.ended_at_ms = Some(at_ms.max(self.started_at_ms));
        }
    }
}

/// What a turn did, as the hub keeps it with its answer (hub-v1, the
/// messages page): the folded line of an answer after later turns and when
/// the conversation is opened again. The steps themselves are live only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ts",
    ts(export_to = "../../apps/desktop/src/bridge/generated/")
)]
pub struct WorkSummary {
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub started_at_ms: u64,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub ended_at_ms: u64,
    pub tools: u32,
    pub subjects: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool(id: &str, phase: WorkPhase, subject: &str) -> WorkItem {
        WorkItem {
            id: id.into(),
            kind: WorkKind::Tool,
            phase,
            label: format!("Usando {subject}"),
            subject: Some(subject.into()),
            steps: Vec::new(),
            cause: None,
        }
    }

    #[test]
    fn a_step_is_replaced_in_place_and_a_tool_counts_once_when_it_ends() {
        let mut work = AgentWork::new("turno", 1_000);
        work.record(tool("a", WorkPhase::Started, "la pantalla"));
        work.record(tool("b", WorkPhase::Started, "el calendario"));
        work.record(tool("a", WorkPhase::Completed, "la pantalla"));
        work.record(tool("a", WorkPhase::Completed, "la pantalla"));
        assert_eq!(
            work.items.iter().map(|item| &item.id).collect::<Vec<_>>(),
            ["a", "b"]
        );
        assert_eq!(work.items[0].phase, WorkPhase::Completed);
        assert_eq!(work.tools, 1);
        assert_eq!(work.subjects, ["la pantalla", "el calendario"]);
    }

    #[test]
    fn past_the_ceiling_the_oldest_ended_step_leaves_and_stays_counted() {
        let mut work = AgentWork::new("turno", 0);
        work.record(tool("en-curso", WorkPhase::Started, "el Mac"));
        for index in 0..MAX_WORK_ITEMS {
            work.record(tool(&format!("t{index}"), WorkPhase::Completed, "el Mac"));
        }
        assert_eq!(work.items.len(), MAX_WORK_ITEMS);
        assert_eq!(work.items[0].id, "en-curso", "a running step stays listed");
        assert!(work.items.iter().all(|item| item.id != "t0"));
        assert_eq!(work.tools, MAX_WORK_ITEMS as u32);
    }

    #[test]
    fn the_first_end_is_kept_and_never_precedes_the_start() {
        let mut work = AgentWork::new("turno", 5_000);
        work.end(4_000);
        work.end(9_000);
        assert_eq!(work.ended_at_ms, Some(5_000));
    }

    #[test]
    fn a_wire_item_is_cut_to_the_ceilings_and_an_unknown_kind_is_left_out() {
        let data = crate::wire::hub::ItemData {
            session_id: "s".into(),
            item_id: "plan".into(),
            kind: "plan".into(),
            phase: "progress".into(),
            label: "ñ".repeat(MAX_ITEM_LABEL_CHARS + 5),
            subject: None,
            steps: Some(
                (0..MAX_PLAN_STEPS + 2)
                    .map(|index| crate::wire::hub::ItemStep {
                        text: format!("Paso {index}"),
                        status: "in_progress".into(),
                    })
                    .collect(),
            ),
            cause: None,
        };
        let item = WorkItem::from_wire(&data);
        assert!(item.as_ref().is_some_and(|item| {
            item.label.chars().count() == MAX_ITEM_LABEL_CHARS
                && item.steps.len() == MAX_PLAN_STEPS
                && item.steps[0].status == WorkStepStatus::InProgress
        }));
        let future = crate::wire::hub::ItemData {
            kind: "browser".into(),
            ..data
        };
        assert_eq!(WorkItem::from_wire(&future), None);
    }

    /// What the assistant topic's ceiling counts on: a cut id, steps only
    /// in a plan, and one plan per turn.
    #[test]
    fn only_one_plan_has_steps_and_an_id_is_cut() {
        let step = || crate::wire::hub::ItemStep {
            text: "Paso".into(),
            status: "pending".into(),
        };
        let data = crate::wire::hub::ItemData {
            session_id: "s".into(),
            item_id: "x".repeat(MAX_ITEM_ID_CHARS + 5),
            kind: "tool".into(),
            phase: "started".into(),
            label: "Mirando la pantalla".into(),
            subject: None,
            steps: Some(vec![step()]),
            cause: None,
        };
        let tool = WorkItem::from_wire(&data);
        assert!(tool.as_ref().is_some_and(|tool| {
            tool.steps.is_empty() && tool.id.chars().count() == MAX_ITEM_ID_CHARS
        }));
        let plan = |id: &str| crate::wire::hub::ItemData {
            item_id: id.into(),
            kind: "plan".into(),
            steps: Some(vec![step()]),
            ..data.clone()
        };
        let mut work = AgentWork::new("turno", 0);
        for id in ["plan-a", "plan-b"] {
            if let Some(item) = WorkItem::from_wire(&plan(id)) {
                work.record(item);
            }
        }
        assert_eq!(
            work.items
                .iter()
                .map(|item| item.id.as_str())
                .collect::<Vec<_>>(),
            ["plan-b"]
        );
    }

    #[test]
    fn subjects_stop_at_their_ceiling() {
        let mut work = AgentWork::new("turno", 0);
        for index in 0..MAX_WORK_SUBJECTS + 3 {
            work.record(tool(
                &format!("t{index}"),
                WorkPhase::Completed,
                &format!("s{index}"),
            ));
        }
        assert_eq!(work.subjects.len(), MAX_WORK_SUBJECTS);
        assert_eq!(work.tools, (MAX_WORK_SUBJECTS + 3) as u32);
    }
}
