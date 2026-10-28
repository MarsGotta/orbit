use super::*;

app_type! {
    /// How many calendars the Mac has, from its last listing. There
    /// is nothing to select, so there is no «selected» beside it. `None` means
    /// not known yet — the helper has not listed them — and never «0».
    pub struct CalendarSettingsSnapshot {
        #[ts(type = "number | null")]
        pub total: Option<u32>,
    }
}

app_type! {
    #[serde(rename_all = "snake_case")]
    pub enum LoginItemState {
        On,
        Off,
        Unknown,
    }
}

app_type! {
    pub struct VersionSettingsSnapshot {
        pub app: String,
    }
}

app_type! {
    #[serde(rename_all = "snake_case")]
    #[ts(rename = "UpdateState")]
    pub enum SettingsUpdateState {
        Disabled,
        Idle,
        Checking,
        Available,
        UpToDate,
        NotNewer,
        Installing,
        Failed,
    }
}

app_type! {
    pub struct InstallResultSnapshot {
        pub version: Option<String>,
        pub when: String,
        pub failure: Option<String>,
        pub note: Option<String>,
        pub log_path: Option<String>,
    }
}

app_type! {
    pub struct UpdateSnapshot {
        #[ts(optional)]
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub install_blocked: Option<String>,
        pub state: SettingsUpdateState,
        pub version: Option<String>,
        #[ts(optional)]
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub installed_version: Option<String>,
        pub message: Option<String>,
        #[ts(optional, type = "unknown")]
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub refusal: Option<orbit_core::refusal::Refusal>,
        #[ts(optional)]
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub last_installation: Option<InstallResultSnapshot>,
    }
}

impl Default for UpdateSnapshot {
    fn default() -> Self {
        Self {
            state: SettingsUpdateState::Idle,
            version: None,
            installed_version: None,
            message: None,
            refusal: None,
            last_installation: None,
            install_blocked: None,
        }
    }
}

app_type! {
    pub struct ProfileSettingsSnapshot {
        pub name: Option<String>,
    }
}

app_type! {
    pub struct SettingsSnapshot {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        pub assistant_name: Option<String>,
        pub hub: SettingsHubSnapshot,
        pub permissions: PermissionSnapshot,
        pub helper_connected: bool,
        pub calendars: CalendarSettingsSnapshot,
        pub login_item: LoginItemState,
        pub versions: VersionSettingsSnapshot,
        pub update: UpdateSnapshot,
        pub privacy_stopped: bool,
        pub onboarding_done: bool,
        #[ts(optional)]
        pub profile: Option<ProfileSettingsSnapshot>,
        /// «Enviar informes de errores anónimos»: the user's choice, absent
        /// when this build cannot send (no switch is offered) or the daemon
        /// has not said yet.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        pub error_reports: Option<bool>,
        /// The per-Mac language choice; absent before the daemon reports it.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        pub language: Option<orbit_core::language::LanguageChoice>,
    }
}
