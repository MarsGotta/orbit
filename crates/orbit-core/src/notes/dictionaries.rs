//! The hub's named meeting vocabularies and the orders that edit them.
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ts",
    ts(export_to = "../../apps/desktop/src/bridge/generated/")
)]
pub enum DictionaryRequest {
    Save {
        dictionary_id: String,
        name: String,
    },
    Select {
        recording_id: String,
        dictionary_id: Option<String>,
        use_for_series: bool,
    },
    Correct {
        recording_id: String,
        dictionary_id: String,
        wrong: String,
        correct: String,
        name: Option<String>,
    },
    Remove {
        dictionary_id: String,
        entry_id: String,
    },
}

impl DictionaryRequest {
    pub fn target(&self) -> String {
        match self {
            Self::Save { dictionary_id, .. }
            | Self::Correct { dictionary_id, .. }
            | Self::Remove { dictionary_id, .. } => format!("dictionary:{dictionary_id}"),
            Self::Select { recording_id, .. } => format!("dictionary_meeting:{recording_id}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ts",
    ts(export_to = "../../apps/desktop/src/bridge/generated/")
)]
pub struct DictionaryEntry {
    pub id: String,
    pub correct: String,
    pub wrong: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ts",
    ts(export_to = "../../apps/desktop/src/bridge/generated/")
)]
pub struct DictionarySeries {
    pub calendar_item_id: String,
    pub title: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ts",
    ts(export_to = "../../apps/desktop/src/bridge/generated/")
)]
pub struct MeetingDictionary {
    pub dictionary_id: Option<String>,
    pub source: Option<String>,
    pub series: Option<DictionarySeries>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ts",
    ts(export_to = "../../apps/desktop/src/bridge/generated/")
)]
pub struct Dictionary {
    pub id: String,
    pub name: String,
    pub terms: Vec<DictionaryEntry>,
    pub series: Vec<DictionarySeries>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DictionaryList {
    pub dictionaries: Vec<Dictionary>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ts",
    ts(export_to = "../../apps/desktop/src/bridge/generated/")
)]
pub struct DictionaryFailure {
    pub target: String,
    pub code: String,
    pub detail: String,
    #[cfg_attr(feature = "ts", ts(type = "Record<string, number>"))]
    pub numbers: BTreeMap<String, u64>,
}
