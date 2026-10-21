use super::{ProjectTopic, TopicSource};
use orbit_core::{
    PermissionState,
    calendar::CalendarAuthorization,
    topics::{Knowledge, TopicData, permissions::Value},
};

pub(super) struct Projector;

impl ProjectTopic for Projector {
    fn project(&self, source: &TopicSource<'_>) -> TopicData {
        let capabilities = &source.state.capabilities();
        let observed = capabilities.native_helper();
        let permission = |value| {
            if observed {
                value
            } else {
                PermissionState::Unknown
            }
        };
        let calendar = match capabilities.calendar() {
            Some(CalendarAuthorization::FullAccess) => PermissionState::Granted,
            Some(
                CalendarAuthorization::Denied
                | CalendarAuthorization::Restricted
                | CalendarAuthorization::WriteOnly,
            ) => PermissionState::Denied,
            Some(CalendarAuthorization::NotDetermined) => PermissionState::NotDetermined,
            Some(CalendarAuthorization::Unavailable | CalendarAuthorization::Unknown) | None => {
                PermissionState::Unknown
            }
        };
        TopicData::Permissions(Knowledge::Present {
            value: Value {
                microphone: permission(capabilities.microphone_state()),
                screen_audio: permission(capabilities.screen_state()),
                screen_recording: permission(capabilities.screen_state()),
                accessibility: permission(capabilities.accessibility_state()),
                calendar: permission(calendar),
            },
            as_of_ms: None,
        })
    }
}

#[cfg(test)]
mod ref206_tests {
    use super::*;
    use orbit_core::{State, topics::Knowledge};

    #[test]
    fn screen_uses_one_observation_for_both_capture_capabilities() {
        let mut state = State::default();
        {
            let fixture_value = serde_json::json!(true);
            crate::daemon::authority_fixture::set(
                &mut state,
                "capabilities.native_helper",
                fixture_value,
            );
        }
        {
            let fixture_value = serde_json::json!(PermissionState::Denied);
            crate::daemon::authority_fixture::set(
                &mut state,
                "capabilities.microphone_state",
                fixture_value,
            );
        }
        {
            let fixture_value = serde_json::json!(PermissionState::NotDetermined);
            crate::daemon::authority_fixture::set(
                &mut state,
                "capabilities.screen_state",
                fixture_value,
            );
        }
        {
            let fixture_value = serde_json::json!(PermissionState::Granted);
            crate::daemon::authority_fixture::set(
                &mut state,
                "capabilities.accessibility_state",
                fixture_value,
            );
        }
        {
            let fixture_value = serde_json::json!(Some(CalendarAuthorization::FullAccess));
            crate::daemon::authority_fixture::set(
                &mut state,
                "capabilities.calendar",
                fixture_value,
            );
        }
        let source = TopicSource {
            state: &state,
            runtime: None,
            now_ms: 0,
        };
        let TopicData::Permissions(Knowledge::Present { value, .. }) = Projector.project(&source)
        else {
            panic!("expected a permission fact");
        };
        assert_eq!(value.microphone, PermissionState::Denied);
        assert_eq!(value.screen_audio, PermissionState::NotDetermined);
        assert_eq!(value.screen_recording, PermissionState::NotDetermined);
        assert_eq!(value.accessibility, PermissionState::Granted);
        assert_eq!(value.calendar, PermissionState::Granted);
        {
            let fixture_value = serde_json::json!(PermissionState::NotGranted);
            crate::daemon::authority_fixture::set(
                &mut state,
                "capabilities.screen_state",
                fixture_value,
            );
        }
        let non_grant = TopicSource {
            state: &state,
            runtime: None,
            now_ms: 1,
        };
        let TopicData::Permissions(Knowledge::Present { value, .. }) =
            Projector.project(&non_grant)
        else {
            panic!("expected a permission fact");
        };
        assert_eq!(value.screen_audio, PermissionState::NotGranted);
        assert_eq!(value.screen_recording, PermissionState::NotGranted);
        {
            let fixture_value = serde_json::json!(false);
            crate::daemon::authority_fixture::set(
                &mut state,
                "capabilities.native_helper",
                fixture_value,
            );
        }
        let disconnected = TopicSource {
            state: &state,
            runtime: None,
            now_ms: 1,
        };
        let TopicData::Permissions(Knowledge::Present { value, .. }) =
            Projector.project(&disconnected)
        else {
            panic!("expected an unknown permission fact");
        };
        assert_eq!(value.microphone, PermissionState::Unknown);
        assert_eq!(value.screen_recording, PermissionState::Unknown);
        assert_eq!(value.calendar, PermissionState::Unknown);
    }
}
