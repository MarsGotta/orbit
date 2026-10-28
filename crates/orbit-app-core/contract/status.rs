use super::*;

app_type! {
    pub struct SettingsHubSnapshot {
        pub url: Option<String>,
        pub paired: bool,
        pub device_name: Option<String>,
        pub device_id_short: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        pub connection: Option<HubLinkConnection>,
    }
}

app_type! {
    /// The engine, and why it is not ready when it is not. The
    /// macOS permission is the whole decision, so there is no «enabled» here:
    /// only the permissions it is missing or has not been told about yet.
    #[serde(tag = "state", rename_all = "snake_case")]
    pub enum EngineSnapshot {
        Ready {
            tools: u32,
        },
        Starting,
        PrivacyStopped,
        /// The helper that reports what macOS grants is not connected.
        HelperNotConnected,
        PermissionMissing {
            accessibility: bool,
            screen_recording: bool,
        },
        PermissionUnknown {
            accessibility: bool,
            screen_recording: bool,
        },
        Failed {
            cause: String,
            #[serde(default, skip_serializing_if = "Option::is_none")]
            #[ts(optional, type = "number")]
            retry_in_ms: Option<u64>,
            #[serde(default, skip_serializing_if = "Option::is_none")]
            #[ts(optional, type = "number")]
            retry_at_ms: Option<i64>,
        },
        /// The daemon did not say: never read as ready nor as missing.
        Unknown,
    }
}

app_type! {
    pub struct ControlHistorySnapshot {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        pub agent: Option<orbit_core::computer_control::ControlAgent>,
        pub operation_id: String,
        pub requested: String,
        pub application: String,
        pub channel: String,
        #[ts(type = "number")]
        pub at_ms: i64,
        pub outcome: String,
    }
}

app_type! {
    pub struct ComputerControlSnapshot {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        pub availability: Option<crate::features::FeatureAvailability>,
        pub operation_id: String,
        pub stage: String,
        pub detail: String,
        pub remaining_steps: u16,
        #[ts(type = "number")]
        pub expires_in_ms: u64,
        /// Whether the engine is ready and, when it is not, why — the
        /// macOS permission it is missing, the privacy stop, or its failure.
        pub engine: EngineSnapshot,
        /// The tools the engine declares, as it declares them.
        pub engine_tools: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        pub engine_tools_error: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        pub history_error: Option<String>,
        pub history: Vec<ControlHistorySnapshot>,
        /// Settings › the assistant's folders: each one and whether it opens.
        #[serde(default)]
        pub folders: AssistantFoldersSnapshot,
    }
}

app_type! {
    /// What Settings knows of the folders the user gave the assistant. A
    /// list that could not be read says so; one not reported yet is
    /// `unknown`, never an empty list.
    #[derive(Default)]
    pub struct AssistantFoldersSnapshot {
        pub state: AssistantFoldersState,
        pub folders: Vec<orbit_core::assistant_files::FolderView>,
        /// How many folders can be given (`assistant_files::MAX_FOLDERS`).
        pub max: u16,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        pub unreadable: Option<String>,
    }
}

app_type! {
    #[derive(Copy, Default)]
    #[serde(rename_all = "snake_case")]
    pub enum AssistantFoldersState {
        Known,
        #[default]
        Unknown,
        Unreadable,
    }
}
