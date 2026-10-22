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
    /// The meeting's dictionaries, or with `use_for_series` its series'.
    /// `dictionary_id` is one (every hub takes it); `dictionary_ids` is the
    /// whole list, for a hub whose documents carry
    /// `MeetingDictionary::dictionary_ids`. One of the two.
    Select {
        recording_id: String,
        dictionary_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        dictionary_ids: Option<Vec<String>>,
        use_for_series: bool,
    },
    /// One correction into one dictionary (`dictionary_id`) or into several
    /// at once (`dictionary_ids`, all or nothing); `name` creates the ones
    /// that do not exist yet.
    Correct {
        recording_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        dictionary_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        dictionary_ids: Option<Vec<String>>,
        wrong: String,
        correct: String,
        name: Option<String>,
    },
    Remove {
        dictionary_id: String,
        entry_id: String,
    },
    /// «Deshacer» a correction: `wrong` leaves the term `correct` in each of
    /// these dictionaries.
    RemoveCorrection {
        dictionary_ids: Vec<String>,
        wrong: String,
        correct: String,
    },
}

impl DictionaryRequest {
    pub fn target(&self) -> String {
        match self {
            Self::Correct {
                dictionary_ids: Some(ids),
                ..
            }
            | Self::RemoveCorrection {
                dictionary_ids: ids,
                ..
            } => format!("dictionaries:{}", ids.join(",")),
            Self::Correct { dictionary_id, .. } => {
                format!(
                    "dictionary:{}",
                    dictionary_id.as_deref().unwrap_or_default()
                )
            }
            Self::Save { dictionary_id, .. } | Self::Remove { dictionary_id, .. } => {
                format!("dictionary:{dictionary_id}")
            }
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
    /// The first of `dictionary_ids`, the one that wins a conflict; the only
    /// one a hub up to 0.10.1 gives.
    pub dictionary_id: Option<String>,
    /// Every dictionary the meeting uses, in the hub's order. `None`: a hub
    /// up to 0.10.1, which takes one dictionary per meeting or series.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub dictionary_ids: Option<Vec<String>>,
    pub source: Option<String>,
    pub series: Option<DictionarySeries>,
}

impl MeetingDictionary {
    /// The meeting's dictionaries from either hub: an older hub's one
    /// dictionary is a list of one.
    pub fn ids(&self) -> Vec<String> {
        match &self.dictionary_ids {
            Some(ids) => ids.clone(),
            None => self.dictionary_id.iter().cloned().collect(),
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    const ORDERS: &str =
        include_str!("../../../../contracts/fixtures/hub/meeting-dictionary-orders.json");

    /// The orders as the hub's own test sends them to its real route
    /// (`server/tests/test_meeting_dictionary_lists.py`): the Mac writes
    /// exactly these bytes' shapes, the old single-dictionary ones included.
    #[test]
    fn every_dictionary_order_the_hub_accepts_is_the_one_the_mac_writes()
    -> Result<(), serde_json::Error> {
        let orders: Vec<serde_json::Value> = serde_json::from_str(ORDERS)?;
        assert!(orders.len() >= 7);
        for order in orders {
            let request: DictionaryRequest = serde_json::from_value(order.clone())?;
            assert_eq!(serde_json::to_value(&request)?, order);
        }
        Ok(())
    }

    #[test]
    fn a_correction_into_several_dictionaries_and_its_undo_share_one_target() {
        let ids = vec!["uno".to_owned(), "dos".to_owned()];
        let correct = DictionaryRequest::Correct {
            recording_id: "reunion".into(),
            dictionary_id: None,
            dictionary_ids: Some(ids.clone()),
            wrong: "fondos".into(),
            correct: "LedgerBox".into(),
            name: None,
        };
        let undo = DictionaryRequest::RemoveCorrection {
            dictionary_ids: ids,
            wrong: "fondos".into(),
            correct: "LedgerBox".into(),
        };
        assert_eq!(correct.target(), "dictionaries:uno,dos");
        assert_eq!(undo.target(), correct.target());
        let single = DictionaryRequest::Correct {
            recording_id: "reunion".into(),
            dictionary_id: Some("uno".into()),
            dictionary_ids: None,
            wrong: "fondos".into(),
            correct: "LedgerBox".into(),
            name: None,
        };
        assert_eq!(single.target(), "dictionary:uno");
    }

    #[test]
    fn a_meeting_reads_its_dictionaries_from_this_hub_and_from_an_older_one()
    -> Result<(), serde_json::Error> {
        let older: MeetingDictionary = serde_json::from_value(serde_json::json!(
            {"dictionary_id": "uno", "source": "requested", "series": null}
        ))?;
        assert_eq!(older.dictionary_ids, None);
        assert_eq!(older.ids(), ["uno"]);
        let none: MeetingDictionary = serde_json::from_value(serde_json::json!(
            {"dictionary_id": null, "source": null, "series": null}
        ))?;
        assert!(none.ids().is_empty());
        let several: MeetingDictionary = serde_json::from_value(serde_json::json!(
            {"dictionary_id": "uno", "dictionary_ids": ["uno", "dos"], "source": "series",
             "series": {"calendar_item_id": "serie-inventada", "title": "Revisión semanal"}}
        ))?;
        assert_eq!(several.ids(), ["uno", "dos"]);
        Ok(())
    }
}
