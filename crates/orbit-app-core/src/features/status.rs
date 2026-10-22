use super::{FeatureAvailability, FeatureInput};
use crate::topics::TopicFreshness;
use crate::{
    AppSnapshot, CalendarSettingsSnapshot, ConnectionSnapshot, DaemonConnection, HubConnection,
    HubLinkConnection, SettingsHubSnapshot, VersionSettingsSnapshot, topics::TopicConnection,
};
use orbit_core::{
    Connection,
    topics::{Knowledge, TopicData, TopicId},
};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../apps/desktop/src/bridge/generated/")]
pub struct StatusView {
    pub availability: FeatureAvailability,
    pub connection: ConnectionSnapshot,
    pub privacy_stopped: bool,
    pub system_timezone: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub system_timezone_error: Option<String>,
    pub settings: StatusSettingsView,
    // The time the topic connection was lost, only while a status fact is stale.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "number")]
    pub stale_since_ms: Option<i64>,
    /// The daemon's latest notice, from the notices topic, and whether it is
    /// known: no topic is an unknown notice, never «no notice».
    pub notice_availability: FeatureAvailability,
    /// A daemon restart starts a different sequence ledger.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub notice_epoch: Option<String>,
    pub notice: Option<orbit_core::notice::Notice>,
    /// The recent notices, oldest first: the interface reads the ones it has
    /// not seen yet, in order.
    pub notices: Vec<orbit_core::notice::Notice>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../apps/desktop/src/bridge/generated/")]
pub struct StatusSettingsView {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub assistant_name: Option<String>,
    /// The agents a conversation can be with, in the hub's order; one for
    /// a hub without agents. Empty until the daemon says.
    #[serde(default)]
    pub agents: Vec<orbit_core::agents::Agent>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub default_agent_id: Option<String>,
    /// The services an agent can be added with from Orbit; absent for a hub
    /// that adds none and hands no conversation over.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub addable_agents: Option<Vec<String>>,
    /// The connected hub's attachment ceilings; absent before its `ready`
    /// and for a hub without attachments (0.9.0), when the composer offers none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub attachments: Option<orbit_core::attachments::AttachmentLimits>,
    pub hub: SettingsHubSnapshot,
    pub helper_connected: bool,
    pub calendars: CalendarSettingsSnapshot,
    pub versions: VersionSettingsSnapshot,
    pub privacy_stopped: bool,
    /// The error-report switch, when this build offers it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub error_reports: Option<bool>,
    /// How long a call from the app lasts at most, in minutes, for
    /// «Cuánto dura cada cosa»: orbit-core's `voice::CALL_MINUTES`.
    pub call_minutes: u16,
    /// How long a call begun with ⌃⌥D lasts at most, in minutes:
    /// `voice::HOTKEY_CALL_MINUTES`.
    pub hotkey_call_minutes: u16,
}

pub type View = StatusView;

pub fn project(input: &FeatureInput<'_>) -> View {
    let (system_timezone, system_timezone_error) = match &input.clock.timezone_name {
        Ok(name) => (name.clone(), None),
        Err(cause) => (String::new(), Some(cause.clone())),
    };
    let daemon = match input.topics.connection() {
        TopicConnection::Connecting => DaemonConnection::Connecting,
        TopicConnection::Ready => DaemonConnection::Ready,
        TopicConnection::Ended { .. } => DaemonConnection::Lost,
    };
    let calendar_total = input.topics.topic(TopicId::Agenda).and_then(|topic| {
        if let TopicData::Agenda(Knowledge::Present { value, .. }) = &topic.knowledge {
            value.calendars
        } else {
            None
        }
    });
    let (notice, notices, notice_availability) = notice(input);
    let mut view = StatusView {
        notice_availability,
        notice_epoch: input
            .topics
            .topic(TopicId::Notices)
            .and_then(|_| input.topics.epoch())
            .map(str::to_owned),
        notice,
        notices,
        availability: FeatureAvailability::missing(input.topics),
        connection: ConnectionSnapshot {
            daemon,
            hub: HubConnection::Offline,
        },
        privacy_stopped: false,
        system_timezone,
        system_timezone_error,
        settings: StatusSettingsView {
            attachments: None,
            assistant_name: None,
            agents: Vec::new(),
            default_agent_id: None,
            addable_agents: None,
            hub: SettingsHubSnapshot {
                url: None,
                paired: false,
                device_name: None,
                device_id_short: None,
                connection: None,
            },
            helper_connected: false,
            calendars: CalendarSettingsSnapshot {
                total: calendar_total,
            },
            versions: VersionSettingsSnapshot {
                app: env!("CARGO_PKG_VERSION").into(),
            },
            privacy_stopped: false,
            error_reports: None,
            call_minutes: orbit_core::voice::CALL_MINUTES,
            hotkey_call_minutes: orbit_core::voice::HOTKEY_CALL_MINUTES,
        },
        stale_since_ms: None,
    };
    if let Some(topic) = input.topics.topic(TopicId::Status)
        && let TopicData::Status(knowledge) = &topic.knowledge
    {
        match knowledge {
            Knowledge::Present { value, .. } => {
                view.availability = match &topic.freshness {
                    TopicFreshness::Current => FeatureAvailability::Current,
                    TopicFreshness::Stale { since_ms } => {
                        view.stale_since_ms = *since_ms;
                        FeatureAvailability::Stale {
                            since_ms: *since_ms,
                        }
                    }
                };
                view.connection.hub = match value.hub {
                    Connection::Online => HubConnection::Paired,
                    Connection::Connecting => HubConnection::Connecting,
                    Connection::Offline | Connection::Revoked => HubConnection::Offline,
                };
                view.privacy_stopped = value.privacy_stopped;
                view.settings.privacy_stopped = view.privacy_stopped;
                view.settings.helper_connected = value.helper_connected;
                view.settings.hub = SettingsHubSnapshot {
                    url: value
                        .account
                        .as_ref()
                        .and_then(|account| account.hub_url.as_deref())
                        .map(ToString::to_string),
                    paired: value
                        .account
                        .as_ref()
                        .and_then(|account| account.device_id_short.as_deref())
                        .is_some(),
                    device_name: value
                        .account
                        .as_ref()
                        .and_then(|account| account.device_name.as_deref())
                        .map(ToString::to_string),
                    device_id_short: value
                        .account
                        .as_ref()
                        .and_then(|account| account.device_id_short.as_deref())
                        .map(ToString::to_string),
                    connection: Some(match value.hub {
                        Connection::Online => HubLinkConnection::Online,
                        Connection::Connecting => HubLinkConnection::Connecting,
                        Connection::Offline => HubLinkConnection::Offline,
                        Connection::Revoked => HubLinkConnection::Revoked,
                    }),
                };
                view.settings.assistant_name = value.assistant_name.clone();
                view.settings.agents = value.agents.clone();
                view.settings.default_agent_id = value.default_agent_id.clone();
                view.settings.addable_agents = value.addable_agents.clone();
                view.settings.attachments = value.attachments;
                view.settings.error_reports = value.error_reports;
            }
            Knowledge::Unknown { why } => {
                view.availability = FeatureAvailability::unknown_pending(why);
            }
            Knowledge::Absent { refusal } => {
                view.availability = FeatureAvailability::Absent {
                    cause: refusal.to_string(),
                };
            }
        }
    }
    view
}

/// The current notice, the recent ones and whether they are known.
type Notices = (
    Option<orbit_core::notice::Notice>,
    Vec<orbit_core::notice::Notice>,
    FeatureAvailability,
);

fn notice(input: &FeatureInput<'_>) -> Notices {
    let Some(cell) = input.topics.topic(TopicId::Notices) else {
        return (None, Vec::new(), FeatureAvailability::missing(input.topics));
    };
    match &cell.knowledge {
        TopicData::Notices(Knowledge::Present { value, .. }) => (
            value.notice.clone(),
            value.notices.clone(),
            match &cell.freshness {
                TopicFreshness::Current => FeatureAvailability::Current,
                TopicFreshness::Stale { since_ms } => FeatureAvailability::Stale {
                    since_ms: *since_ms,
                },
            },
        ),
        TopicData::Notices(Knowledge::Unknown { why }) => {
            (None, Vec::new(), FeatureAvailability::unknown_pending(why))
        }
        TopicData::Notices(Knowledge::Absent { refusal }) => (
            None,
            Vec::new(),
            FeatureAvailability::Absent {
                cause: refusal.to_string(),
            },
        ),
        TopicData::Status(_)
        | TopicData::Permissions(_)
        | TopicData::Recording(_)
        | TopicData::Agenda(_)
        | TopicData::Notes(_)
        | TopicData::Assistant(_)
        | TopicData::Control(_) => (
            None,
            Vec::new(),
            FeatureAvailability::Unknown {
                cause: "notices_topic_type_mismatch".into(),
            },
        ),
    }
}

pub fn apply(view: &View, snapshot: &mut AppSnapshot) {
    snapshot.notice_availability = Some(view.notice_availability.clone());
    snapshot.notice = view.notice.clone();
    snapshot.status_availability = Some(view.availability.clone());
    snapshot.connection = view.connection.clone();
    snapshot.privacy_stopped = view.privacy_stopped;
    snapshot.system_timezone = view.system_timezone.clone();
    snapshot.system_timezone_error = view.system_timezone_error.clone();
    if let Some(name) = &view.settings.assistant_name {
        snapshot.settings.assistant_name = Some(name.clone());
    }
    snapshot.settings.hub = view.settings.hub.clone();
    snapshot.settings.helper_connected = view.settings.helper_connected;
    snapshot.settings.calendars = view.settings.calendars.clone();
    snapshot.settings.versions = view.settings.versions.clone();
    snapshot.settings.privacy_stopped = view.settings.privacy_stopped;
    snapshot.settings.error_reports = view.settings.error_reports;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "command", rename_all = "snake_case")]
#[ts(export, export_to = "../../apps/desktop/src/bridge/generated/")]
pub enum StatusCommand {
    OpenUrl { url: String },
    Quit,
}

#[cfg(test)]
mod status_tests {
    use super::*;
    use crate::{DaemonConnection, topics::TopicCache};
    use orbit_core::topics::{Knowledge, TopicData, TopicFrame};

    fn status_value(
        hub: &str,
        privacy_stopped: bool,
        helper_connected: bool,
    ) -> orbit_core::topics::status::Value {
        serde_json::from_value(serde_json::json!({
            "hub": hub,
            "account": null,
            "privacy_stopped": privacy_stopped,
            "helper_connected": helper_connected
        }))
        .unwrap_or_else(|error| panic!("synthetic status: {error}"))
    }

    fn status_json(input: &FeatureInput<'_>) -> serde_json::Value {
        serde_json::to_value(project(input))
            .unwrap_or_else(|error| panic!("synthetic status view: {error}"))
    }

    #[test]
    fn ended_daemon_reports_its_cause_and_elapsed_ms() {
        let snapshot = crate::AppSnapshot::default();
        let mut topics = TopicCache::default();
        topics
            .ended("El almacén se está recuperando: paso fragmentos, 413 ms transcurridos".into());
        let view = project(&FeatureInput::new(&snapshot, &topics));
        assert_eq!(view.connection.daemon, DaemonConnection::Lost);
        assert!(
            matches!(view.availability, FeatureAvailability::Unknown { cause } if cause.contains("fragmentos") && cause.contains("413 ms"))
        );
    }

    #[test]
    fn status_frame_carries_later_system_zone() {
        let mut initial = crate::AppSnapshot::empty(1, crate::DaemonConnection::Ready);
        initial.system_timezone = "UTC".into();
        let mut changed = initial.clone();
        changed.system_timezone = "Europe/Madrid".into();
        let topics = TopicCache::default();
        let input = FeatureInput::new(&changed, &topics);
        let view = project(&input);
        assert_eq!(status_json(&input)["system_timezone"], "Europe/Madrid");
        assert_ne!(project(&FeatureInput::new(&initial, &topics)), view);
        apply(&view, &mut initial);
        assert_eq!(initial.system_timezone, "Europe/Madrid");
    }

    #[test]
    fn a_status_topic_change_changes_the_feature_without_a_snapshot_change() {
        let snapshot = crate::AppSnapshot::empty(1, crate::DaemonConnection::Ready);
        let empty = TopicCache::default();
        let mut observed = TopicCache::default();
        observed.observe_at(
            TopicFrame {
                epoch: "status-fixture".into(),
                revision: 1,
                data: TopicData::Status(Knowledge::Present {
                    value: status_value("online", true, true),
                    as_of_ms: None,
                }),
            },
            Some(1_000),
        );
        assert_ne!(
            project(&FeatureInput::new(&snapshot, &empty)),
            project(&FeatureInput::new(&snapshot, &observed)),
        );
    }

    #[test]
    fn a_stale_status_reports_disconnection_time_without_inventing_one() {
        let snapshot = crate::AppSnapshot::empty(1, crate::DaemonConnection::Lost);
        let mut observed = TopicCache::default();
        observed.observe_at(
            TopicFrame {
                epoch: "status-fixture".into(),
                revision: 1,
                data: TopicData::Status(Knowledge::Present {
                    value: status_value("online", false, false),
                    as_of_ms: None,
                }),
            },
            Some(1_000),
        );
        observed.ended("El daemon terminó".into());
        let since_ms = match observed
            .topic(TopicId::Status)
            .map(|topic| &topic.freshness)
        {
            Some(TopicFreshness::Stale { since_ms }) => *since_ms,
            other => panic!("expected stale status, got {other:?}"),
        };
        assert_eq!(
            status_json(&FeatureInput::new(&snapshot, &observed)).get("stale_since_ms"),
            Some(&serde_json::json!(since_ms))
        );
        assert_ne!(since_ms, Some(1_000));
        assert_eq!(
            status_json(&FeatureInput::new(&snapshot, &TopicCache::default()))
                .get("stale_since_ms"),
            None
        );
        let mut never_observed = TopicCache::default();
        never_observed.observe_at(
            TopicFrame {
                epoch: "status-fixture".into(),
                revision: 0,
                data: TopicData::not_observed(orbit_core::topics::TopicId::Status),
            },
            Some(2_000),
        );
        never_observed.ended("El daemon terminó".into());
        assert_eq!(
            status_json(&FeatureInput::new(&snapshot, &never_observed)).get("stale_since_ms"),
            None
        );
    }
}

#[cfg(test)]
mod notice_tests {
    use super::*;
    use crate::topics::TopicCache;

    #[test]
    fn engine_exit_reaches_the_app_with_its_cause_and_status() {
        use orbit_core::notice::{Notice, NoticeCode, NoticeKey};
        let snapshot = crate::AppSnapshot::default();
        let notice = Notice::new(NoticeCode::EngineExited)
            .with_token(NoticeKey::Cause, "engine")
            .with_token(NoticeKey::Code, "signal")
            .with(NoticeKey::Status, 15_i64);
        let mut topics = TopicCache::default();
        topics.observe_at(
            orbit_core::topics::TopicFrame {
                epoch: "engine-exit".into(),
                revision: 1,
                data: TopicData::Notices(Knowledge::Present {
                    value: orbit_core::topics::notices::Value {
                        notice: Some(notice.clone()),
                        notices: vec![notice.clone()],
                    },
                    as_of_ms: None,
                }),
            },
            Some(0),
        );
        let view = project(&FeatureInput::new(&snapshot, &topics));
        assert_eq!(view.notice, Some(notice.clone()));
        assert_eq!(view.notices, vec![notice]);
    }

    #[test]
    fn the_notice_cursor_carries_the_daemon_epoch_across_restarts() {
        let snapshot = crate::AppSnapshot::default();
        let mut topics = TopicCache::default();
        for epoch in ["daemon-before-restart", "daemon-after-restart"] {
            topics.observe_at(
                orbit_core::topics::TopicFrame {
                    epoch: epoch.into(),
                    revision: 1,
                    data: TopicData::Notices(Knowledge::Present {
                        value: orbit_core::topics::notices::Value {
                            notice: None,
                            notices: Vec::new(),
                        },
                        as_of_ms: None,
                    }),
                },
                Some(0),
            );
            let view = project(&FeatureInput::new(&snapshot, &topics));
            assert_eq!(view.notice_epoch.as_deref(), Some(epoch));
        }
    }

    /// Moved here from the retired activity function, which used to carry
    /// the notice: no topic is an unknown notice, never a remembered one.
    #[test]
    fn without_topics_the_notice_is_not_known() {
        let mut snapshot = crate::AppSnapshot::empty(1, crate::DaemonConnection::Ready);
        snapshot.notice = Some(orbit_core::notice::Notice::new(
            orbit_core::notice::NoticeCode::RecordingStarted,
        ));
        let topics = TopicCache::default();
        let view = project(&FeatureInput::new(&snapshot, &topics));
        assert!(view.notice.is_none());
        assert!(matches!(
            view.notice_availability,
            FeatureAvailability::Unknown { .. }
        ));
    }
}

#[cfg(test)]
mod assistant_name_tests {
    #[test]
    fn status_projects_the_name_and_keeps_it_when_the_fact_is_missing()
    -> Result<(), Box<dyn std::error::Error>> {
        use crate::{
            features::{FeatureInput, status},
            topics::TopicCache,
        };
        use orbit_core::topics::TopicFrame;
        let mut snapshot = crate::AppSnapshot::default();
        let mut topics = TopicCache::default();
        let frame: TopicFrame = serde_json::from_value(serde_json::json!({
            "epoch": "assistant-name", "revision": 1, "data": {"topic": "status",
            "knowledge": {"state": "present", "value": {
                "hub":"online", "account": null, "helper_connected": true,
                "privacy_stopped": false, "assistant_name":"Atlas"
            }}}
        }))?;
        topics.observe(frame);
        let view = status::project(&FeatureInput::new(&snapshot, &topics));
        assert_eq!(
            serde_json::to_value(&view)?["settings"]["assistant_name"],
            "Atlas"
        );
        status::apply(&view, &mut snapshot);
        let missing = status::project(&FeatureInput::new(&snapshot, &TopicCache::default()));
        status::apply(&missing, &mut snapshot);
        assert_eq!(
            serde_json::to_value(snapshot)?["settings"]["assistant_name"],
            "Atlas"
        );
        Ok(())
    }

    /// The daemon's agents and their default reach the interface; a
    /// status from before agents has none, and the interface then names
    /// the assistant by `assistant_name`.
    #[test]
    fn status_projects_the_agents_and_their_default() -> Result<(), Box<dyn std::error::Error>> {
        use crate::{
            features::{FeatureInput, status},
            topics::TopicCache,
        };
        use orbit_core::topics::TopicFrame;
        let snapshot = crate::AppSnapshot::default();
        let mut topics = TopicCache::default();
        let frame: TopicFrame = serde_json::from_value(serde_json::json!({
            "epoch": "agents", "revision": 1, "data": {"topic": "status",
            "knowledge": {"state": "present", "value": {
                "hub":"online", "account": null, "helper_connected": true,
                "privacy_stopped": false, "assistant_name":"Deimos",
                "agents": [{"id":"deimos","name":"Deimos"},{"id":"atlas","name":"Atlas"}],
                "default_agent_id": "deimos"
            }}}
        }))?;
        topics.observe(frame);
        let view = serde_json::to_value(status::project(&FeatureInput::new(&snapshot, &topics)))?;
        assert_eq!(
            view["settings"]["agents"],
            serde_json::json!([{"id":"deimos","name":"Deimos"},{"id":"atlas","name":"Atlas"}])
        );
        assert_eq!(view["settings"]["default_agent_id"], "deimos");
        let before: TopicFrame = serde_json::from_value(serde_json::json!({
            "epoch": "agents", "revision": 2, "data": {"topic": "status",
            "knowledge": {"state": "present", "value": {
                "hub":"online", "account": null, "helper_connected": true,
                "privacy_stopped": false
            }}}
        }))?;
        topics.observe(before);
        let view = serde_json::to_value(status::project(&FeatureInput::new(&snapshot, &topics)))?;
        assert_eq!(view["settings"]["agents"], serde_json::json!([]));
        assert!(view["settings"].get("default_agent_id").is_none());
        Ok(())
    }

    /// Each agent's profile and account, and the services the hub can add,
    /// reach the interface (hub-v1 «Assistants from Orbit»); a hub before
    /// them says none, and the interface offers no adding.
    #[test]
    fn status_projects_the_profiles_and_what_the_hub_can_add()
    -> Result<(), Box<dyn std::error::Error>> {
        use crate::{
            features::{FeatureInput, status},
            topics::TopicCache,
        };
        use orbit_core::topics::TopicFrame;
        let snapshot = crate::AppSnapshot::default();
        let mut topics = TopicCache::default();
        let nova = serde_json::json!({"id":"codex-1a2b3c4d","name":"Nova","service":"codex",
            "description":"Programa conmigo","color":"c2","removable":true,"account":"expired",
            "limit":{"until_ms":1_790_000_000_000_i64},
            "usage":{"used_percent":72,"window":"seven_day"}});
        let frame: TopicFrame = serde_json::from_value(serde_json::json!({
            "epoch": "agents", "revision": 1, "data": {"topic": "status",
            "knowledge": {"state": "present", "value": {
                "hub":"online", "account": null, "helper_connected": true,
                "privacy_stopped": false, "agents": [nova.clone()],
                "default_agent_id": "codex-1a2b3c4d", "addable_agents": ["codex"]
            }}}
        }))?;
        topics.observe(frame);
        let view = serde_json::to_value(status::project(&FeatureInput::new(&snapshot, &topics)))?;
        assert_eq!(view["settings"]["agents"], serde_json::json!([nova]));
        assert_eq!(
            view["settings"]["addable_agents"],
            serde_json::json!(["codex"])
        );
        Ok(())
    }
}
