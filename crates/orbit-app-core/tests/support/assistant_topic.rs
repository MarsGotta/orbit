use orbit_app_core::{
    AppSnapshot, DaemonConnection,
    features::{FeatureInput, assistant, recording},
    topics::TopicCache,
};
use orbit_core::{
    PermissionState,
    topics::{
        Knowledge, Pending, TopicData, TopicFrame,
        assistant::{Value, VoiceAvailability},
    },
};

pub fn assistant_value() -> Value {
    Value {
        session_id: None,
        agent_id: None,
        turn: None,
        pending: None,
        waiting_sessions: Default::default(),
        history_revision: None,
        voice_active: false,
        pending_voice: Vec::new(),
        turn_in_flight: false,
        activity: None,
        voice: VoiceAvailability::Unknown {
            why: Pending::NotObservedYet,
        },
        speaking: None,
        call: Knowledge::Unknown {
            why: Pending::NotObservedYet,
        },
        short_voice_note: None,
        work: None,
        microphone_muted: false,
    }
}

pub fn snapshot(value: Value, screen: PermissionState) -> AppSnapshot {
    snapshot_with_privacy(value, screen, false)
}

pub fn snapshot_with_privacy(
    value: Value,
    screen: PermissionState,
    privacy_stopped: bool,
) -> AppSnapshot {
    let mut topics = TopicCache::default();
    topics.observe_at(
        TopicFrame {
            epoch: "assistant-contract".into(),
            revision: 1,
            data: TopicData::Assistant(Knowledge::Present {
                value,
                as_of_ms: None,
            }),
        },
        Some(1),
    );
    topics.observe_at(
        TopicFrame {
            epoch: "assistant-contract".into(),
            revision: 2,
            data: TopicData::Permissions(Knowledge::Present {
                value: orbit_core::topics::permissions::Value {
                    microphone: PermissionState::Unknown,
                    screen_audio: PermissionState::Unknown,
                    screen_recording: screen,
                    accessibility: PermissionState::Unknown,
                    calendar: PermissionState::Unknown,
                },
                as_of_ms: None,
            }),
        },
        Some(1),
    );
    topics.observe_at(
        TopicFrame {
            epoch: "assistant-contract".into(),
            revision: 3,
            data: TopicData::Status(Knowledge::Present {
                value: orbit_core::topics::status::Value {
                    attachments: None,
                    assistant_name: None,
                    agents: Vec::new(),
                    default_agent_id: None,
                    addable_agents: None,
                    hub: orbit_core::Connection::Online,
                    account: None,
                    privacy_stopped,
                    helper_connected: true,
                    error_reports: None,
                    language: None,
                },
                as_of_ms: None,
            }),
        },
        Some(1),
    );
    let mut snapshot = AppSnapshot::empty(1, DaemonConnection::Ready);
    assistant::apply(
        &assistant::project(&FeatureInput::new(&snapshot, &topics)),
        &mut snapshot,
    );
    recording::apply(
        &recording::project(&FeatureInput::new(&snapshot, &topics)),
        &mut snapshot,
    );
    snapshot
}
