//! Read-only vault contracts. The hub owns settings, credentials and all note I/O.
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

macro_rules! wire {
    ($item:item) => {
        #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
        #[cfg_attr(feature = "ts", derive(ts_rs::TS))]
        #[cfg_attr(
            feature = "ts",
            ts(export_to = "../../apps/desktop/src/bridge/generated/")
        )]
        $item
    };
}

mod index;
pub use index::*;

wire! {
    #[serde(tag = "call", rename_all = "snake_case", deny_unknown_fields)]
    pub enum VaultCall {
        Config,
        Configure { configuration: VaultConnection },
        Disconnect,
        Read { request: VaultRead },
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ts",
    ts(export_to = "../../apps/desktop/src/bridge/generated/")
)]
pub struct VaultConnection {
    pub kind: String,
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub vault: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub token: Option<String>,
    #[serde(default = "markdown_style")]
    pub link_style: String,
    #[serde(default)]
    pub maintenance_model: String,
}

impl std::fmt::Debug for VaultConnection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VaultConnection").finish_non_exhaustive()
    }
}

wire! {
    pub struct VaultSettings {
        pub configured: bool,
        #[serde(default)]
        pub kind: String,
        #[serde(default)]
        pub path: String,
        #[serde(default)]
        pub url: String,
        #[serde(default)]
        pub vault: String,
        #[serde(default)]
        pub has_token: bool,
        #[serde(default = "markdown_style")]
        pub link_style: String,
        #[serde(default)]
        pub maintenance_model: String,
        #[serde(default)]
        pub maintenance_enabled: bool,
    }
}

wire! {
    #[serde(rename_all = "snake_case")]
    pub enum VaultOperation { List, Search, Read, Backlinks, Outlinks, Index, Catalog, Tasks }
}

wire! {
    #[serde(deny_unknown_fields)]
    pub struct VaultRead {
        pub operation: VaultOperation,
        #[serde(default)]
        pub path: String,
        #[serde(default)]
        pub query: String,
        #[serde(default = "yes")]
        pub content: bool,
        #[serde(default = "first")]
        pub page: u32,
        #[serde(default = "page_size")]
        pub page_size: u32,
        #[serde(default = "all_state")]
        pub state: String,
        #[serde(default)]
        pub owner: String,
        #[serde(default)]
        pub today: String,
    }
}

fn yes() -> bool {
    true
}
fn first() -> u32 {
    1
}
fn page_size() -> u32 {
    50
}
fn markdown_style() -> String {
    "markdown".into()
}
fn all_state() -> String {
    "all".into()
}

wire! {
    pub struct VaultEntry {
        pub path: String, pub kind: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        pub title: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        pub r#type: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        pub snippet: Option<String>,
    }
}
wire! {
    pub struct VaultPage {
        pub entries: Vec<VaultEntry>,
        pub next_page: Option<u32>,
    }
}
wire! {
    pub struct VaultLink { pub target: String, pub path: String }
}
wire! {
    pub struct VaultNote {
        pub path: String,
        pub r#type: String, pub role: String,
        #[cfg_attr(feature = "ts", ts(type = "Record<string, unknown>"))]
        pub properties: BTreeMap<String, serde_json::Value>,
        pub schema_warnings: Vec<String>,
        pub conformance: VaultConformance,
        pub dependency_tasks: Vec<VaultTask>,
        #[cfg_attr(feature = "ts", ts(type = "Record<string, unknown>"))]
        pub frontmatter: BTreeMap<String, serde_json::Value>,
        pub body: String,
        pub outlinks: Vec<VaultLink>,
        pub synthesized: bool,
        #[serde(default)]
        pub raw: String,
        #[serde(default)]
        pub citations: Vec<VaultCitation>,
        #[serde(default)]
        pub tasks: Vec<VaultTask>,
        #[serde(default)]
        pub dependencies: Vec<VaultDependency>,
        #[serde(default)]
        pub queries: Vec<VaultQuery>,
        #[serde(default)]
        pub history: Vec<VaultActivity>,
        #[serde(default)]
        pub history_next_page: Option<u32>,
        #[serde(default)]
        pub next_page: Option<u32>,
    }
}

wire! {
    #[serde(tag = "outcome", rename_all = "snake_case")]
    pub enum VaultOutcome {
        Config { configuration: VaultSettings },
        Page { page: VaultPage },
        Note { note: VaultNote },
        Catalog { catalog: VaultCatalog },
        Tasks { tasks: VaultTaskPage },
        Refused { code: String, message: String,
            #[cfg_attr(feature = "ts", ts(type = "Record<string, number>"))]
            numbers: BTreeMap<String, u64> },
        Failed {
            #[cfg_attr(feature = "ts", ts(type = "Record<string, unknown>"))]
            refusal: crate::refusal::Refusal },
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VaultResponse {
    pub v: u32,
    pub id: String,
    pub outcome: VaultOutcome,
}
