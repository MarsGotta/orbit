use super::*;
use crate::{features::FeatureInput, topics::TopicCache};
use orbit_core::{
    Connection,
    topics::{Knowledge, TopicData, TopicFrame},
};

fn status_topic(hub: Connection, privacy_stopped: bool) -> TopicData {
    TopicData::Status(Knowledge::Present {
        value: orbit_core::topics::status::Value {
            attachments: None,
            assistant_name: None,
            agents: Vec::new(),
            default_agent_id: None,
            hub,
            account: None,
            privacy_stopped,
            helper_connected: true,
            error_reports: None,
        },
        as_of_ms: None,
    })
}

fn observe(topics: &mut TopicCache, revision: u64, data: TopicData) {
    topics.observe_at(
        TopicFrame {
            epoch: "status-fixture".into(),
            revision,
            data,
        },
        Some(1),
    );
}

#[test]
fn connecting_hub_is_never_projected_as_unpaired() {
    let mut topics = TopicCache::default();
    observe(&mut topics, 1, status_topic(Connection::Connecting, false));
    let local = crate::AppSnapshot::default();
    let view = crate::features::status::project(&FeatureInput::new(&local, &topics));
    assert_eq!(view.connection.hub, HubConnection::Connecting);
    assert_ne!(view.connection.hub, HubConnection::Unpaired);
}

#[test]
fn default_state_projects_every_field() {
    let snapshot = crate::AppSnapshot::empty(7, crate::DaemonConnection::Ready);
    assert_eq!(snapshot.revision, 7);
    assert_eq!(snapshot.connection.daemon, DaemonConnection::Ready);
    assert_eq!(snapshot.connection.hub, HubConnection::Offline);
    assert!(!snapshot.privacy_stopped);
    assert_eq!(snapshot.permissions.microphone, PermissionState::Unknown);
    assert_eq!(snapshot.permissions.screen, PermissionState::Unknown);
    assert_eq!(snapshot.permissions.accessibility, PermissionState::Unknown);
    assert_eq!(snapshot.permissions.calendars, PermissionState::Unknown);
    assert_eq!(snapshot.meeting.phase, MeetingPhase::Idle);
    assert_eq!(snapshot.meeting.title, None);
    assert_eq!(snapshot.meeting.started_at_ms, None);
    assert_eq!(snapshot.chat.session_id, None);
    assert!(!snapshot.chat.turn.iter().any(|message| message.streaming));
    assert!(snapshot.chat.turn.is_empty());
    assert_eq!(snapshot.chat.pending_contract, None);
    assert_eq!(
        snapshot.notice, None,
        "an unobserved notice is not an old default notice"
    );
}

#[test]
fn projects_observed_permission_states_instead_of_boolean_fallbacks() {
    let mut topics = TopicCache::default();
    observe(
        &mut topics,
        1,
        TopicData::Permissions(Knowledge::Present {
            value: orbit_core::topics::permissions::Value {
                microphone: orbit_core::PermissionState::NotDetermined,
                screen_audio: orbit_core::PermissionState::Denied,
                screen_recording: orbit_core::PermissionState::Denied,
                accessibility: orbit_core::PermissionState::Granted,
                calendar: orbit_core::PermissionState::Unknown,
            },
            as_of_ms: None,
        }),
    );
    let snapshot = crate::AppSnapshot::default();
    let view = crate::features::permissions::project(&FeatureInput::new(&snapshot, &topics));
    assert_eq!(view.microphone, PermissionState::NotDetermined);
    assert_eq!(view.screen, PermissionState::Denied);
    assert_eq!(view.accessibility, PermissionState::Granted);
}

#[test]
fn update_service_states_are_projected_for_settings() {
    assert_eq!(
        project_update(UpdateState::Available {
            version: "0.3.0".into()
        }),
        UpdateSnapshot {
            state: SettingsUpdateState::Available,
            version: Some("0.3.0".into()),
            installed_version: None,
            message: None,
            refusal: None,
            last_installation: None,
            install_blocked: None,
        }
    );
    assert_eq!(
        project_update(UpdateState::UpToDate),
        UpdateSnapshot {
            state: SettingsUpdateState::UpToDate,
            version: None,
            installed_version: None,
            message: None,
            refusal: None,
            last_installation: None,
            install_blocked: None,
        }
    );
    assert_eq!(
        project_update(UpdateState::NotNewer {
            installed: "0.3.13".into(),
            published: "0.3.12".into(),
        }),
        UpdateSnapshot {
            state: SettingsUpdateState::NotNewer,
            version: Some("0.3.12".into()),
            installed_version: Some("0.3.13".into()),
            message: None,
            refusal: None,
            last_installation: None,
            install_blocked: None,
        }
    );
    assert_eq!(
        project_update(UpdateState::Refused {
            refusal: orbit_core::refusal::Refusal::Update {
                cause: orbit_core::refusal::UpdateCause::HttpStatus {
                    url: "http://fixture/latest.txt".into(),
                    status: 503,
                },
            }
        }),
        UpdateSnapshot {
            state: SettingsUpdateState::Failed,
            version: None,
            installed_version: None,
            message: None,
            refusal: Some(orbit_core::refusal::Refusal::Update {
                cause: orbit_core::refusal::UpdateCause::HttpStatus {
                    url: "http://fixture/latest.txt".into(),
                    status: 503,
                },
            }),
            last_installation: None,
            install_blocked: None,
        }
    );
}

#[test]
fn projects_settings_and_the_app_version() {
    let app_version = env!("CARGO_PKG_VERSION");
    let mut topics = TopicCache::default();
    observe(&mut topics, 1, status_topic(Connection::Online, true));
    observe(
        &mut topics,
        1,
        TopicData::Permissions(Knowledge::Present {
            value: orbit_core::topics::permissions::Value {
                microphone: orbit_core::PermissionState::Granted,
                screen_audio: orbit_core::PermissionState::Denied,
                screen_recording: orbit_core::PermissionState::Denied,
                accessibility: orbit_core::PermissionState::Granted,
                calendar: orbit_core::PermissionState::NotDetermined,
            },
            as_of_ms: None,
        }),
    );
    let mut snapshot = crate::AppSnapshot::default();
    let input = FeatureInput::new(&snapshot, &topics);
    let status = crate::features::status::project(&input);
    let permissions = crate::features::permissions::project(&input);
    crate::features::status::apply(&status, &mut snapshot);
    crate::features::permissions::apply(&permissions, &mut snapshot);
    assert_eq!(
        snapshot.settings.permissions.microphone,
        PermissionState::Granted
    );
    assert_eq!(
        snapshot.settings.permissions.screen,
        PermissionState::Denied
    );
    assert_eq!(
        snapshot.settings.permissions.accessibility,
        PermissionState::Granted
    );
    assert_eq!(
        snapshot.settings.permissions.calendars,
        PermissionState::NotDetermined
    );
    assert!(snapshot.settings.helper_connected);
    assert_eq!(snapshot.settings.versions.app, app_version);
    assert!(snapshot.settings.privacy_stopped);
    assert!(!snapshot.settings.onboarding_done);
}
