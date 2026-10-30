//! Reviewed proposals are hub facts. The device forwards explicit actions only.
use super::*;

wire! {
    #[serde(rename_all = "snake_case")]
    pub enum DistillationStatus {
        Queued, Preparing, Proposed, Failed, Stale, Applying, Applied,
        Discarded, Undoing, Undone, RecoveryRequired,
    }
}
wire! {
    #[serde(rename_all = "snake_case")]
    pub enum DistillationAction { Apply, Discard, Undo, Prepare }
}
wire! {
    #[serde(rename_all = "snake_case")]
    pub enum ConflictChoice { Incoming, Current, Both }
}
wire! {
    pub struct DistillationQuote {
        pub id: String,
        #[cfg_attr(feature = "ts", ts(type = "number"))]
        pub at_ms: u64,
        pub speaker: String, pub text: String,
    }
}
wire! {
    pub struct DistillationProperty {
        pub key: String,
        #[cfg_attr(feature = "ts", ts(type = "unknown"))]
        pub before: serde_json::Value,
        #[cfg_attr(feature = "ts", ts(type = "unknown"))]
        pub after: serde_json::Value,
        pub quote_ids: Vec<String>,
    }
}
wire! {
    pub struct DistillationLines {
        pub section: String, pub before: String, pub after: String, pub quote_ids: Vec<String>,
    }
}
wire! {
    #[serde(rename_all = "snake_case")]
    pub enum DistillationChangeKind { Create, Update }
}
wire! {
    pub struct DistillationWarning {
        pub code: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        pub words: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        pub max_words: Option<u32>,
    }
}
wire! {
    pub struct DistillationChange {
        pub id: String, pub path: String, pub kind: DistillationChangeKind,
        pub title: String, pub include: bool, pub base_hash: Option<String>,
        pub properties: Vec<DistillationProperty>, pub lines: Vec<DistillationLines>,
        pub warnings: Vec<DistillationWarning>,
    }
}
wire! {
    pub struct DistillationConflict {
        pub id: String, pub path: String, pub property: String,
        #[cfg_attr(feature = "ts", ts(type = "unknown"))]
        pub current: serde_json::Value,
        #[cfg_attr(feature = "ts", ts(type = "unknown"))]
        pub incoming: serde_json::Value,
        pub quote_ids: Vec<String>, pub choice: Option<ConflictChoice>,
    }
}
wire! {
    pub struct DistillationSource { pub path: String, pub summary: String, pub sha256: String }
}
wire! {
    pub struct DistillationSkipped { pub path: String, pub code: String }
}
wire! {
    pub struct Distillation {
        pub id: String, pub source_kind: String, pub meeting_id: String,
        #[serde(default)]
        pub title: String,
        pub created_at: String,
        #[serde(default)]
        pub by: String,
        #[serde(default)]
        pub summary: String,
        pub revision: u32, pub status: DistillationStatus,
        pub changes: Vec<DistillationChange>, pub conflicts: Vec<DistillationConflict>,
        pub quotes: Vec<DistillationQuote>,
        #[serde(default)]
        pub people: BTreeMap<String, Option<String>>,
        pub new_source: Option<DistillationSource>,
        #[cfg_attr(feature = "ts", ts(type = "Record<string, unknown> | null"))]
        pub error: Option<BTreeMap<String, serde_json::Value>>,
        pub changed_paths: Vec<String>, pub skipped: Vec<DistillationSkipped>,
        pub can_prepare_again: bool,
    }
}
wire! {
    pub struct DistillationPage { pub entries: Vec<Distillation>, pub next_page: Option<u32> }
}
wire! {
    #[serde(deny_unknown_fields)]
    pub struct DistillationEdit {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        pub include: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(type = "Record<string, unknown>", optional))]
        pub properties: Option<BTreeMap<String, serde_json::Value>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        pub lines: Option<BTreeMap<String, String>>,
    }
}
wire! {
    #[serde(deny_unknown_fields)]
    pub struct DistillationReview {
        pub revision: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        pub changes: Option<BTreeMap<String, DistillationEdit>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        pub decisions: Option<BTreeMap<String, ConflictChoice>>,
    }
}
