//! The bounded, paginated reading index of the hub's Markdown working copy.
use super::*;

wire! {
    pub struct VaultCitation {
        pub id: String, pub number: u32, pub resource: String, pub title: String,
    }
}
wire! {
    pub struct VaultTask {
        pub file: String, pub line: u32, pub block_id: String, pub text: String,
        pub who: String, pub who_path: String, pub due: String, pub done: bool,
        pub scheduled: String, pub start: String, pub priority: String, pub recurrence: String,
        pub id: String, pub depends_on: Vec<String>, pub tags: Vec<String>,
        pub done_at: String, pub project: String, pub source_id: String, pub source: String,
    }
}
wire! {
    pub struct VaultDependency {
        pub prerequisite: String, pub dependent: String, pub relation: String,
    }
}
wire! {
    pub struct VaultDiagnostic {
        pub code: String, pub field: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        pub value: Option<String>,
        #[cfg_attr(feature = "ts", ts(type = "Record<string, number>"))]
        pub numbers: BTreeMap<String, u64>,
    }
}
wire! {
    pub struct VaultQuery {
        pub kind: String, pub lines: Vec<String>, pub warnings: Vec<String>,
        pub tasks: Vec<VaultTask>, pub total: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        pub diagnostics: Option<Vec<VaultDiagnostic>>,
    }
}
wire! {
    pub struct VaultMeta {
        pub path: String, pub kind: String, pub title: String, pub r#type: String,
        pub role: String, pub description: String, pub state: String, pub start: String,
        pub links: Vec<String>, pub backlink_count: u32, pub task_count: u32,
        pub modified_at: String,
    }
}
wire! {
    pub struct VaultActivity {
        pub date: String, pub time: String, pub action: String, pub title: String,
        pub path: String, pub by: String, pub detail: String,
        #[serde(default)]
        pub text: String,
    }
}
wire! {
    pub struct VaultConformanceIssue { pub field: String, pub code: String, pub message: String }
}
wire! {
    pub struct VaultConformance { pub conformant: bool, pub issues: Vec<VaultConformanceIssue> }
}
wire! {
    pub struct VaultType { pub role: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        pub folder: Option<String>,
    }
}
wire! {
    pub struct VaultSchema {
        pub path: String, pub body: String, pub folders: BTreeMap<String, String>,
        pub types: BTreeMap<String, VaultType>, pub fields: BTreeMap<String, String>,
        pub warnings: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        pub diagnostics: Option<Vec<VaultDiagnostic>>,
    }
}
wire! {
    pub struct VaultCatalog {
        pub notes: Vec<VaultMeta>, pub next_page: Option<u32>, pub notes_count: u32,
        pub sources_count: u32, pub open_tasks: u32, pub okf_version: String,
        pub schema: VaultSchema, pub recent: Vec<VaultActivity>,
        pub type_counts: BTreeMap<String, u32>,
    }
}
wire! {
    pub struct VaultTaskPage {
        pub tasks: Vec<VaultTask>, pub next_page: Option<u32>, pub total: u32,
        pub counts: BTreeMap<String, u32>,
    }
}

/// Raw Markdown, parsed metadata and one page of links, with JSON escaping.
/// Separate from note bytes and the generic command-response ceiling.
pub const MAX_VAULT_RESPONSE_BYTES: usize = 4 * 1024 * 1024;
