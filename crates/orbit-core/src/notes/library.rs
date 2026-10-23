//! The hub's templates and their links, and what the Mac adds to the library
//! read: the orders on their way, the refused ones and the undos answered.
use super::{Dictionary, DictionaryConflict, DictionaryFailure, TemplateSection};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// One of the hub's templates (`GET /v1/note-templates`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Template {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub context: String,
    #[serde(default)]
    pub sections: Vec<TemplateSection>,
    #[serde(default)]
    pub builtin: bool,
    /// A copy says of which template, with its name at the time.
    #[serde(default)]
    pub forked_from: Option<TemplateOrigin>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TemplateOrigin {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TemplateList {
    pub templates: Vec<Template>,
}

/// One series with a template assigned (`GET /v1/note-template-links`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TemplateLink {
    pub calendar_item_id: String,
    pub template_id: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TemplateLinks {
    pub links: Vec<TemplateLink>,
}

/// What the library read carries, one per page item (`ReadId::NoteLibrary`).
/// The templates and assignments are the hub's; the rest is what this Mac
/// knows of the orders the user gave in this session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "item", rename_all = "snake_case")]
pub enum LibraryItem {
    Dictionary(Dictionary),
    DictionaryRefused {
        failure: DictionaryFailure,
    },
    /// What the hub warned of when it took a correction, by the order's
    /// target (`DictionaryRequest::target`).
    DictionaryConflicts {
        target: String,
        conflicts: Vec<DictionaryConflict>,
    },
    Template(Template),
    Link(TemplateLink),
    /// A list the hub could not give, with its cause.
    Unavailable {
        list: LibraryList,
        refusal: crate::refusal::Refusal,
    },
    /// An order on its way to the hub, by what it is about.
    Pending {
        target: String,
    },
    /// An order the hub refused or could not receive.
    Refused(LibraryRefusal),
    /// An undo («Deshacer») that was answered.
    Restored {
        request_id: String,
        template_id: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LibraryList {
    Dictionaries,
    Templates,
    Links,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LibraryRefusal {
    /// `template` or `series`.
    pub target_kind: String,
    pub target_id: String,
    /// `save`, `delete`, `assign` or `unassign`.
    pub action: String,
    /// The hub's HTTP status; 0 when the order did not reach it.
    pub status: u16,
    /// The hub's code, or `hub_unreachable`.
    pub code: String,
    /// The hub's numbers for its refusal (`links`, `max_links`, `series`,
    /// `max_series`, `bytes`…); empty when it gave none.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub numbers: BTreeMap<String, u64>,
}
