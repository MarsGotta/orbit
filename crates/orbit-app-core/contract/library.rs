use super::*;

app_type! {
    /// The template a copy comes from, with its name at the time.
    pub struct NoteTemplateOriginSnapshot {
        pub id: String,
        pub name: String,
    }
}

app_type! {
    /// A template as the user sees it in Settings: the hub's; `pending` while
    /// an order about it is on its way.
    pub struct NoteTemplateDetailSnapshot {
        pub id: String,
        pub name: String,
        pub context: String,
        pub sections: Vec<TemplateSection>,
        /// «De Orbit»: never changed or deleted; editing it saves a copy.
        pub builtin: bool,
        pub forked_from: Option<NoteTemplateOriginSnapshot>,
        pub pending: bool,
    }
}

app_type! {
    /// A series with its template assigned.
    pub struct NoteTemplateLinkSnapshot {
        pub calendar_item_id: String,
        pub template_id: String,
        /// `None` when the template is not in the list (not read yet).
        pub template_name: Option<String>,
        pub pending: bool,
    }
}

app_type! {
    /// What a refused order was about.
    #[serde(rename_all = "snake_case")]
    pub enum NoteLibraryAction {
        Save,
        Delete,
        Assign,
        Unassign,
    }
}

app_type! {
    /// An order the hub refused (its status and code) or that did not reach
    /// it (`status` 0, `hub_unreachable` or `not_sent`).
    pub struct NoteLibraryRefusalSnapshot {
        /// `template` or `series`.
        pub target_kind: String,
        pub target_id: String,
        pub action: NoteLibraryAction,
        pub status: u16,
        pub code: String,
    }
}

app_type! {
    /// The editor's bounds, from the registry app-core shares with the hub.
    pub struct NoteTemplateBoundsSnapshot {
        pub max_sections: usize,
        pub max_name_chars: usize,
        pub max_context_chars: usize,
        pub max_instructions_chars: usize,
        pub max_custom_templates: usize,
    }
}

app_type! {
    /// Whether a list the hub keeps is known: never read is `unknown`, never
    /// an empty list.
    pub struct NoteLibraryListSnapshot {
        pub state: NoteTemplatesState,
        pub detail: Option<String>,
    }
}

app_type! {
    /// Whether the library read reached the app.
    #[serde(rename_all = "snake_case")]
    pub enum NoteLibraryReadState {
        Loading,
        Available,
        Failed,
    }
}

app_type! {
    /// Templates, assignments and refusals.
    pub struct NoteLibrarySnapshot {
        #[serde(default)]
        #[ts(optional)]
        pub dictionaries: Option<NoteLibraryListSnapshot>,
        #[serde(default)]
        pub dictionary_items: Vec<DictionarySnapshot>,
        #[serde(default)]
        pub dictionary_pending: Vec<String>,
        #[serde(default)]
        pub dictionary_failures: Vec<orbit_core::notes::DictionaryFailure>,
        pub read: NoteLibraryReadState,
        pub read_detail: Option<String>,
        pub templates: NoteLibraryListSnapshot,
        pub items: Vec<NoteTemplateDetailSnapshot>,
        pub links: NoteLibraryListSnapshot,
        pub assignments: Vec<NoteTemplateLinkSnapshot>,
        pub refusals: Vec<NoteLibraryRefusalSnapshot>,
        /// The undos the hub took, newest last, by request.
        pub restores: Vec<NoteTemplateRestoreSnapshot>,
        pub bounds: NoteTemplateBoundsSnapshot,
    }
}

app_type! {
    /// An undo the hub took: the template and its series are back.
    pub struct NoteTemplateRestoreSnapshot {
        pub request_id: String,
        pub template_id: String,
    }
}

app_type! {
    /// The template assigned to a note's series: known (`template_id`, or
    /// `None` for General), not read yet, or why it could not be read.
    #[serde(tag = "state", rename_all = "snake_case")]
    pub enum NoteSeriesAssignedSnapshot {
        Unknown {},
        Known { template_id: Option<String> },
        Failed { cause: String },
    }
}

app_type! {
    /// The series a note belongs to (its event's stable key).
    pub struct NoteSeriesLinkSnapshot {
        pub calendar_item_id: String,
        pub assigned: NoteSeriesAssignedSnapshot,
    }
}

pub use orbit_core::notes::TemplateSection;

app_type! {
    pub struct DictionarySnapshot {
        pub dictionary: orbit_core::notes::Dictionary,
        pub pending: bool,
    }
}
