use super::{FeatureAvailability, FeatureInput};
use crate::topics::TopicFreshness;
use crate::{AppSnapshot, PermissionSnapshot};
use orbit_core::topics::{Knowledge, TopicData, TopicId};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

pub type View = PermissionSnapshot;

pub fn project(input: &FeatureInput<'_>) -> View {
    let Some(topic) = input.topics.topic(TopicId::Permissions) else {
        return unknown_permissions(FeatureAvailability::missing(input.topics));
    };
    let TopicData::Permissions(knowledge) = &topic.knowledge else {
        return unknown_permissions(FeatureAvailability::Unknown {
            cause: "permissions_topic_type_mismatch".into(),
        });
    };
    let value = match knowledge {
        Knowledge::Present { value, .. } => value,
        Knowledge::Unknown { why } => {
            return unknown_permissions(FeatureAvailability::unknown_pending(why));
        }
        Knowledge::Absent { refusal } => {
            return unknown_permissions(FeatureAvailability::Absent {
                cause: refusal.to_string(),
            });
        }
    };
    let availability = match &topic.freshness {
        TopicFreshness::Current => FeatureAvailability::Current,
        TopicFreshness::Stale { since_ms } => FeatureAvailability::Stale {
            since_ms: *since_ms,
        },
    };
    let permission = |state| match state {
        orbit_core::PermissionState::Granted => crate::PermissionState::Granted,
        orbit_core::PermissionState::Denied => crate::PermissionState::Denied,
        orbit_core::PermissionState::NotGranted => crate::PermissionState::NotGranted,
        orbit_core::PermissionState::NotDetermined => crate::PermissionState::NotDetermined,
        orbit_core::PermissionState::Unknown => crate::PermissionState::Unknown,
    };
    PermissionSnapshot {
        microphone: permission(value.microphone),
        screen: if value.screen_audio == value.screen_recording {
            permission(value.screen_recording)
        } else {
            crate::PermissionState::Unknown
        },
        accessibility: permission(value.accessibility),
        calendars: permission(value.calendar),
        availability: Some(availability),
    }
}

fn unknown_permissions(availability: FeatureAvailability) -> PermissionSnapshot {
    PermissionSnapshot {
        microphone: crate::PermissionState::Unknown,
        screen: crate::PermissionState::Unknown,
        accessibility: crate::PermissionState::Unknown,
        calendars: crate::PermissionState::Unknown,
        availability: Some(availability),
    }
}

pub fn apply(view: &View, snapshot: &mut AppSnapshot) {
    snapshot.permissions = view.clone();
    snapshot.settings.permissions = view.clone();
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "command", rename_all = "snake_case")]
#[ts(export, export_to = "../../apps/desktop/src/bridge/generated/")]
pub enum PermissionsCommand {
    OpenSystemPrivacy { pane: String },
}

#[cfg(test)]
mod ref206_tests {
    use super::*;
    use crate::{DaemonConnection, topics::TopicCache};
    use orbit_core::topics::{Knowledge, TopicData, TopicFrame};

    #[test]
    fn a_permission_topic_change_changes_the_feature_without_a_snapshot_change() {
        let snapshot = crate::AppSnapshot::empty(1, crate::DaemonConnection::Ready);
        let empty = TopicCache::default();
        let mut observed = TopicCache::default();
        observed.observe_at(
            TopicFrame {
                epoch: "ref206".into(),
                revision: 1,
                data: TopicData::Permissions(Knowledge::Present {
                    value: serde_json::from_value(serde_json::json!({
                        "microphone": "denied",
                        "screen_audio": "unknown",
                        "screen_recording": "unknown",
                        "accessibility": "unknown",
                        "calendar": "unknown"
                    }))
                    .unwrap_or_else(|error| panic!("synthetic permissions: {error}")),
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
    fn conflicting_screen_uses_cannot_become_a_grant() {
        let snapshot = crate::AppSnapshot::empty(1, crate::DaemonConnection::Ready);
        let mut observed = TopicCache::default();
        observed.observe_at(
            TopicFrame {
                epoch: "ref206".into(),
                revision: 1,
                data: TopicData::Permissions(Knowledge::Present {
                    value: serde_json::from_value(serde_json::json!({
                        "microphone": "granted",
                        "screen_audio": "denied",
                        "screen_recording": "granted",
                        "accessibility": "not_determined",
                        "calendar": "unknown"
                    }))
                    .unwrap_or_else(|error| panic!("synthetic permissions: {error}")),
                    as_of_ms: None,
                }),
            },
            Some(1_000),
        );
        let view = project(&FeatureInput::new(&snapshot, &observed));
        assert_eq!(view.microphone, crate::PermissionState::Granted);
        assert_eq!(view.screen, crate::PermissionState::Unknown);
        assert_eq!(view.accessibility, crate::PermissionState::NotDetermined);
    }

    #[test]
    fn an_observed_unknown_overrides_a_stale_denial_in_both_views() {
        let mut snapshot = crate::AppSnapshot::empty(1, DaemonConnection::Ready);
        snapshot.permissions.microphone = crate::PermissionState::Denied;
        let mut topics = TopicCache::default();
        topics.observe_at(
            TopicFrame {
                epoch: "ref206".into(),
                revision: 1,
                data: TopicData::Permissions(Knowledge::Present {
                    value: serde_json::from_value(serde_json::json!({
                        "microphone": "unknown", "screen_audio": "unknown",
                        "screen_recording": "unknown", "accessibility": "unknown",
                        "calendar": "unknown"
                    }))
                    .unwrap_or_else(|error| panic!("synthetic permissions: {error}")),
                    as_of_ms: None,
                }),
            },
            Some(1_000),
        );
        let view = project(&FeatureInput::new(&snapshot, &topics));
        assert_eq!(view.microphone, crate::PermissionState::Unknown);
        apply(&view, &mut snapshot);
        assert_eq!(
            snapshot.permissions.microphone,
            crate::PermissionState::Unknown
        );
        assert_eq!(
            snapshot.settings.permissions.microphone,
            crate::PermissionState::Unknown
        );
    }

    #[test]
    fn a_known_non_grant_reaches_the_feature_from_the_topic() {
        let snapshot = crate::AppSnapshot::empty(1, crate::DaemonConnection::Ready);
        let mut topics = TopicCache::default();
        topics.observe_at(
            TopicFrame {
                epoch: "ref206".into(),
                revision: 1,
                data: TopicData::Permissions(Knowledge::Present {
                    value: serde_json::from_value(serde_json::json!({
                        "microphone": "unknown", "screen_audio": "not_granted",
                        "screen_recording": "not_granted", "accessibility": "not_granted",
                        "calendar": "unknown"
                    }))
                    .unwrap_or_else(|error| panic!("synthetic permissions: {error}")),
                    as_of_ms: None,
                }),
            },
            Some(1_000),
        );
        let view = project(&FeatureInput::new(&snapshot, &topics));
        assert_eq!(view.screen, crate::PermissionState::NotGranted);
        assert_eq!(view.accessibility, crate::PermissionState::NotGranted);
        assert_eq!(view.microphone, crate::PermissionState::Unknown);
    }
}
