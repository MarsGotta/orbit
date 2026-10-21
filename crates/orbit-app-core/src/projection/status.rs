use super::*;

pub(crate) fn project_update(state: UpdateState) -> UpdateSnapshot {
    match state {
        UpdateState::Disabled => UpdateSnapshot {
            state: SettingsUpdateState::Disabled,
            ..UpdateSnapshot::default()
        },
        UpdateState::Available { version } => UpdateSnapshot {
            state: SettingsUpdateState::Available,
            version: Some(version),
            installed_version: None,
            message: None,
            refusal: None,
            last_installation: None,
            install_blocked: None,
        },
        UpdateState::UpToDate => UpdateSnapshot {
            state: SettingsUpdateState::UpToDate,
            version: None,
            installed_version: None,
            message: None,
            refusal: None,
            last_installation: None,
            install_blocked: None,
        },
        UpdateState::NotNewer {
            installed,
            published,
        } => UpdateSnapshot {
            state: SettingsUpdateState::NotNewer,
            version: Some(published),
            installed_version: Some(installed),
            message: None,
            refusal: None,
            last_installation: None,
            install_blocked: None,
        },
        UpdateState::Refused { refusal } => UpdateSnapshot {
            state: SettingsUpdateState::Failed,
            version: None,
            installed_version: None,
            message: None,
            refusal: Some(refusal),
            last_installation: None,
            install_blocked: None,
        },
    }
}

#[cfg(test)]
mod permission_tests {
    use crate::{PermissionState, features::FeatureInput, topics::TopicCache};

    #[test]
    fn unobserved_permission_is_not_a_denial() {
        let local = crate::AppSnapshot::default();
        let topics = TopicCache::default();
        let permissions =
            crate::features::permissions::project(&FeatureInput::new(&local, &topics));
        assert_eq!(permissions.microphone, PermissionState::Unknown);
        assert_eq!(permissions.screen, PermissionState::Unknown);
        assert_eq!(permissions.accessibility, PermissionState::Unknown);
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests;
