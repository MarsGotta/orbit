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
        pub done_at: String, pub project: String, pub source_id: String, pub source: String,
    }
}
wire! {
    pub struct VaultDependency {
        pub prerequisite: String, pub dependent: String, pub relation: String,
    }
}
wire! {
    pub struct VaultQuery { pub kind: String, pub filters: BTreeMap<String, String> }
}
wire! {
    pub struct VaultMeta {
        pub path: String, pub kind: String, pub title: String, pub r#type: String,
        pub description: String, pub state: String, pub start: String,
        pub links: Vec<String>, pub backlink_count: u32, pub task_count: u32,
        pub modified_at: String,
    }
}
wire! {
    pub struct VaultActivity {
        pub date: String, pub time: String, pub action: String, pub title: String,
        pub path: String, pub by: String, pub detail: String,
    }
}
wire! {
    pub struct VaultSchema {
        pub path: String, pub body: String, pub folders: BTreeMap<String, String>,
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
