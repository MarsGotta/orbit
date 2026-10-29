use super::*;
use std::sync::Mutex;

struct System {
    login: Mutex<LoginItemState>,
    permission: Mutex<PermissionState>,
    delivery: Result<(), String>,
    registration: Result<(), String>,
}
impl Platform for System {
    fn login_status(&self) -> Result<LoginItemState, String> {
        Ok(self
            .login
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone())
    }
    fn set_login(&self, _on: bool) -> Result<(), String> {
        self.registration.clone()?;
        *self
            .login
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = LoginItemState::RequiresApproval;
        Ok(())
    }
    async fn notification_status(&self) -> Result<PermissionState, String> {
        Ok(self
            .permission
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone())
    }
    async fn request_notifications(&self) -> Result<(), String> {
        *self
            .permission
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = PermissionState::Granted;
        Ok(())
    }
    async fn post_notification(
        &self,
        _notice: crate::notify::TurnNotice,
        _sound: bool,
    ) -> Result<(), String> {
        self.delivery.clone()
    }
}
fn client() -> anyhow::Result<(tempfile::TempDir, DaemonClient)> {
    let directory = tempfile::tempdir()?;
    let paths = orbit_ipc::Paths::resolve(Some(directory.path().into()))?;
    let (calls, _) = tokio::sync::mpsc::channel(1);
    let (_, ended) = tokio::sync::watch::channel(None);
    let transport = orbit_ipc::LocalClient::new(calls, &orbit_ipc::TopicChannels::new(), ended);
    Ok((directory, DaemonClient::connect(paths, transport)))
}
#[tokio::test]
async fn registration_publishes_system_approval_instead_of_the_requested_boolean()
-> anyhow::Result<()> {
    let (_directory, client) = client()?;
    let system = System {
        login: Mutex::new(LoginItemState::Off),
        permission: Mutex::new(PermissionState::Denied),
        delivery: Ok(()),
        registration: Ok(()),
    };
    set_login(&client, &system, true)?;
    assert_eq!(
        client.composed_snapshot().settings.login_item,
        LoginItemState::RequiresApproval
    );
    *system
        .login
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = LoginItemState::Off;
    refresh(&client, &system).await;
    assert_eq!(
        client.composed_snapshot().settings.login_item,
        LoginItemState::Off
    );
    assert_eq!(
        client.composed_snapshot().settings.notifications.state,
        PermissionState::Denied
    );
    Ok(())
}
#[tokio::test]
async fn delivery_errors_reach_settings_and_a_success_clears_them() -> anyhow::Result<()> {
    let (_directory, client) = client()?;
    let mut system = System {
        login: Mutex::new(LoginItemState::Off),
        permission: Mutex::new(PermissionState::NotDetermined),
        delivery: Err("notification center error 17".into()),
        registration: Ok(()),
    };
    let notice = crate::notify::TurnNotice {
        title: "Orbit".into(),
        body: "An answer is ready".into(),
    };
    post(&client, &system, notice.clone(), false).await;
    let snapshot = client.composed_snapshot();
    assert_eq!(
        snapshot.settings.notifications.state,
        PermissionState::Granted
    );
    assert_eq!(
        snapshot.settings.notifications.error.as_deref(),
        Some("notification center error 17")
    );
    refresh(&client, &system).await;
    assert!(
        client
            .composed_snapshot()
            .settings
            .notifications
            .error
            .is_some()
    );
    system.delivery = Ok(());
    post(&client, &system, notice, false).await;
    assert!(
        client
            .composed_snapshot()
            .settings
            .notifications
            .error
            .is_none()
    );
    Ok(())
}

#[tokio::test]
async fn migration_preserves_login_choice_and_retires_only_this_apps_legacy_plist()
-> anyhow::Result<()> {
    let (_directory, client) = client()?;
    let legacy = tempfile::tempdir()?;
    std::fs::write(legacy.path().join("Orbit.plist"), "production fixture")?;
    std::fs::write(legacy.path().join("Orbit Pruebas.plist"), "test fixture")?;
    std::fs::write(legacy.path().join("Other.plist"), "unrelated fixture")?;
    let system = System {
        login: Mutex::new(LoginItemState::Off),
        permission: Mutex::new(PermissionState::Denied),
        delivery: Ok(()),
        registration: Ok(()),
    };
    retire_legacy(&client, &system, legacy.path(), "Orbit Pruebas")?;
    assert!(!legacy.path().join("Orbit Pruebas.plist").exists());
    assert!(legacy.path().join("Orbit.plist").exists());
    assert!(legacy.path().join("Other.plist").exists());
    assert_eq!(
        client.composed_snapshot().settings.login_item,
        LoginItemState::RequiresApproval
    );
    Ok(())
}

#[tokio::test]
async fn denied_notifications_do_not_report_delivery_and_registration_failure_keeps_the_legacy_file()
-> anyhow::Result<()> {
    let (_directory, client) = client()?;
    let system = System {
        login: Mutex::new(LoginItemState::Off),
        permission: Mutex::new(PermissionState::Denied),
        delivery: Err("must not post without system permission".into()),
        registration: Err("registration error 19".into()),
    };
    post(
        &client,
        &system,
        crate::notify::TurnNotice {
            title: "Orbit".into(),
            body: "An answer is ready".into(),
        },
        false,
    )
    .await;
    assert_eq!(
        client.composed_snapshot().settings.notifications.state,
        PermissionState::Denied
    );
    assert!(
        client
            .composed_snapshot()
            .settings
            .notifications
            .error
            .is_none()
    );
    let legacy = tempfile::tempdir()?;
    let file = legacy.path().join("Orbit.plist");
    std::fs::write(&file, "legacy fixture")?;
    assert!(retire_legacy(&client, &system, legacy.path(), "Orbit").is_err());
    assert!(file.exists());
    assert_eq!(
        client.composed_snapshot().settings.login_item,
        LoginItemState::Off
    );
    Ok(())
}
