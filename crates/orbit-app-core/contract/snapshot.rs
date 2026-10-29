use super::*;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct AppSnapshot {
    #[ts(type = "number")]
    pub revision: u64,
    // The Mac's current zone, independently of any agenda read. Empty
    // only when the operating system cannot report its zone.
    pub system_timezone: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub system_timezone_error: Option<String>,
    /// The observation instant for Briefing's next event. A value from the
    /// client's injected clock; never serialized or published on its own.
    #[serde(skip)]
    #[ts(skip)]
    pub(crate) briefing_clock_ms: i64,
    pub connection: ConnectionSnapshot,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub status_availability: Option<crate::features::FeatureAvailability>,
    pub privacy_stopped: bool,
    pub permissions: PermissionSnapshot,
    pub meeting: MeetingSnapshot,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub recording_availability: Option<crate::features::FeatureAvailability>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub call_availability: Option<crate::features::FeatureAvailability>,
    pub chat: ChatSnapshot,
    pub call: Option<CallSnapshot>,
    pub calendar: CalendarSnapshot,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub notes: Option<NotesSnapshot>,
    pub notice: Option<orbit_core::notice::Notice>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub notice_availability: Option<crate::features::FeatureAvailability>,
    pub settings: SettingsSnapshot,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub computer_control: Option<Box<ComputerControlSnapshot>>,
    #[ts(optional = nullable)]
    pub briefing: Option<BriefingSnapshot>,
}

impl Default for AppSnapshot {
    fn default() -> Self {
        let permissions = PermissionSnapshot {
            microphone: PermissionState::Unknown,
            screen: PermissionState::Unknown,
            accessibility: PermissionState::Unknown,
            calendars: PermissionState::Unknown,
            availability: None,
        };
        Self {
            revision: 0,
            system_timezone: String::new(),
            system_timezone_error: None,
            briefing_clock_ms: 0,
            connection: ConnectionSnapshot {
                daemon: DaemonConnection::Connecting,
                hub: HubConnection::Offline,
            },
            status_availability: None,
            privacy_stopped: false,
            permissions: permissions.clone(),
            meeting: idle_meeting(),
            recording_availability: None,
            call_availability: None,
            chat: empty_chat(),
            call: None,
            calendar: unread_calendar(),
            notes: Some(unread_notes()),
            notice: None,
            notice_availability: None,
            settings: initial_settings(permissions),
            computer_control: None,
            briefing: None,
        }
    }
}

fn idle_meeting() -> MeetingSnapshot {
    MeetingSnapshot {
        phase: MeetingPhase::Idle,
        recording_id: None,
        title: None,
        started_at_ms: None,
        ends_at_ms: None,
        limit_minutes: 0,
        audio: MeetingAudioKind::System,
        tracks: Vec::new(),
        upload: MeetingUploadSnapshot {
            pending_chunks: 0,
            offline: true,
        },
        call_panel: None,
        call_preference: None,
        call_preference_unreadable: None,
        question: None,
        ask: None,
        voice_trigger: None,
        microphone_muted: false,
        recent: Vec::new(),
        applications: Vec::new(),
        applications_state: MeetingApplicationsState::Unavailable,
        applications_detail: None,
    }
}

fn empty_chat() -> ChatSnapshot {
    ChatSnapshot {
        availability: None,
        session_id: None,
        agent_id: None,
        turn: Vec::new(),
        waiting_sessions: Default::default(),
        history_revision: None,
        pending_voice: None,
        voice_audio: None,
        pending_contract: None,
        voice: VoiceSnapshot {
            state: VoiceState::Unknown,
            level: 0,
            availability: None,
            too_short: None,
        },
        activity: None,
        speaking: None,
        work: None,
    }
}

fn unread_calendar() -> CalendarSnapshot {
    CalendarSnapshot {
        authorization: CalendarAuthorizationSnapshot::Unknown,
        calendars: Vec::new(),
        window: CalendarWindowSnapshot {
            start_ms: 0,
            end_ms: 0,
            timezone: String::new(),
        },
        range: CalendarRangeSnapshot::default(),
        events: Vec::new(),
        reading: CalendarReadingSnapshot {
            state: CalendarReadingState::Unknown,
            blocker: None,
            detail: None,
        },
        stale: true,
        writes: Vec::new(),
    }
}

fn unread_notes() -> NotesSnapshot {
    NotesSnapshot {
        list: Vec::new(),
        projection_clock: None,
        series_previous: None,
        reminder: None,
        // Not read yet: on its way, never «no notes». The first read
        // (`client::notes`) replaces it with what it found.
        index: Some(Box::new(NotesIndexSnapshot {
            state: NotesIndexState::Loading,
            more: false,
            detail: None,
        })),
        open: None,
        open_read: None,
        library: None,
        ready: None,
    }
}

fn initial_settings(permissions: PermissionSnapshot) -> SettingsSnapshot {
    SettingsSnapshot {
        assistant_name: None,
        hub: SettingsHubSnapshot {
            url: None,
            paired: false,
            device_name: None,
            device_id_short: None,
            connection: None,
        },
        permissions,
        helper_connected: false,
        calendars: CalendarSettingsSnapshot { total: None },
        login_item: LoginItemState::Unknown,
        notifications: NotificationSnapshot::default(),
        versions: VersionSettingsSnapshot {
            app: env!("CARGO_PKG_VERSION").into(),
        },
        update: UpdateSnapshot::default(),
        privacy_stopped: false,
        onboarding_done: false,
        profile: Some(ProfileSettingsSnapshot { name: None }),
        error_reports: None,
        language: None,
    }
}

impl AppSnapshot {
    /// Empty local presentation state before any topic or keyed read arrives.
    pub fn empty(revision: u64, daemon: DaemonConnection) -> Self {
        Self {
            revision,
            connection: ConnectionSnapshot {
                daemon,
                hub: HubConnection::Offline,
            },
            ..Self::default()
        }
    }
}

app_type! {
    pub struct AppBootstrap {
        pub revision: crate::features::FrameRevision,
        pub frames: Vec<crate::features::FeatureFrame>,
    }
}

app_type! {
    pub struct CommandReceipt {
        pub revision: crate::features::FrameRevision,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        pub delivery: Option<ReceiptDelivery>,
    }
}

app_type! {
    #[serde(tag = "state", rename_all = "snake_case")]
    pub enum ReceiptDelivery {
        Reflected,
        FrameNotObserved {
            #[ts(type = "string")]
            topic: orbit_core::topics::TopicId,
            #[ts(type = "number")]
            expected_revision: u64,
            #[ts(type = "number | null")]
            observed_revision: Option<u64>,
            expected_epoch: String,
            observed_epoch: Option<String>,
            #[ts(type = "number")]
            waited_ms: u64,
        },
    }
}
