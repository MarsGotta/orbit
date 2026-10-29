//! App-owned system integrations. Only macOS decides permission and login status.
use orbit_app_core::{AppError, DaemonClient, LoginItemState, PermissionState};
use std::future::Future;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub use macos::Native;
#[cfg(not(target_os = "macos"))]
mod unsupported;
#[cfg(not(target_os = "macos"))]
pub use unsupported::Native;

/// The OS boundary; tests replace it without registering items or posting notices.
pub trait Platform {
    fn login_status(&self) -> Result<LoginItemState, String>;
    fn set_login(&self, on: bool) -> Result<(), String>;
    fn notification_status(&self) -> impl Future<Output = Result<PermissionState, String>> + Send;
    fn request_notifications(&self) -> impl Future<Output = Result<(), String>> + Send;
    fn post_notification(
        &self,
        notice: crate::notify::TurnNotice,
        sound: bool,
    ) -> impl Future<Output = Result<(), String>> + Send;
}

pub async fn refresh(client: &DaemonClient, platform: &impl Platform) {
    client.set_login_item(platform.login_status().unwrap_or(LoginItemState::Unknown));
    match platform.notification_status().await {
        Ok(state) => {
            client.set_notifications(state, None);
        }
        Err(error) => {
            client.set_notifications(PermissionState::Unknown, Some(Err(error)));
        }
    }
}

pub fn set_login(
    client: &DaemonClient,
    platform: &impl Platform,
    on: bool,
) -> Result<(), AppError> {
    let result = platform.set_login(on);
    let status = platform.login_status();
    client.set_login_item(status.clone().unwrap_or(LoginItemState::Unknown));
    result.and(status.map(|_| ())).map_err(AppError::daemon)
}

pub async fn request_notifications(
    client: &DaemonClient,
    platform: &impl Platform,
) -> Result<(), AppError> {
    let result = platform.request_notifications().await;
    let (state, result) = match platform.notification_status().await {
        Ok(state) => (state, result),
        Err(error) => (PermissionState::Unknown, result.and(Err(error))),
    };
    client.set_notifications(state, Some(result.clone()));
    result.map_err(AppError::daemon)
}

pub async fn post(
    client: &DaemonClient,
    platform: &impl Platform,
    notice: crate::notify::TurnNotice,
    sound: bool,
) {
    let state = match platform.notification_status().await {
        Ok(PermissionState::NotDetermined) => {
            if request_notifications(client, platform).await.is_err() {
                return;
            }
            platform.notification_status().await
        }
        result => result,
    };
    match state {
        Ok(PermissionState::Granted) => {
            let result = platform.post_notification(notice, sound).await;
            client.set_notifications(PermissionState::Granted, Some(result));
        }
        Ok(state) => {
            client.set_notifications(state, None);
        }
        Err(error) => {
            client.set_notifications(PermissionState::Unknown, Some(Err(error)));
        }
    }
}

/// Retire only this bundle's old autostart file, retaining its existing choice.
/// The old plugin never loaded a job: its file only requested the next login.
#[cfg(any(target_os = "macos", test))]
pub fn retire_legacy(
    client: &DaemonClient,
    platform: &impl Platform,
    directory: &std::path::Path,
    product: &str,
) -> Result<(), AppError> {
    let file = directory.join(format!("{product}.plist"));
    if !file
        .try_exists()
        .map_err(|error| AppError::daemon(error.to_string()))?
    {
        return Ok(());
    }
    if platform.login_status().map_err(AppError::daemon)? == LoginItemState::Off {
        set_login(client, platform, true)?;
    }
    std::fs::remove_file(file).map_err(|error| AppError::daemon(error.to_string()))
}

pub fn initialize<R: tauri::Runtime>(app: &tauri::AppHandle<R>, client: &DaemonClient) {
    use tauri::Manager;
    app.manage(Native);
    #[cfg(target_os = "macos")]
    macos::install();
    #[cfg(target_os = "macos")]
    if let Some(home) = std::env::var_os("HOME") {
        let directory = std::path::PathBuf::from(home).join("Library/LaunchAgents");
        let product = app.package_info().name.as_str();
        if let Err(error) = retire_legacy(client, &Native, &directory, product) {
            eprintln!("Could not retire Orbit's legacy login item: {error}");
        }
    }
    #[cfg(not(target_os = "macos"))]
    let _ = client;
    refresh_app(app);
}

pub fn refresh_app<R: tauri::Runtime>(app: &tauri::AppHandle<R>) {
    use tauri::Manager;
    if app.try_state::<Native>().is_none() {
        return;
    }
    if let Some(client) = app.try_state::<DaemonClient>() {
        let client = client.inner().clone();
        tauri::async_runtime::spawn(async move {
            refresh(&client, &Native).await;
        });
    }
}

#[cfg(test)]
#[path = "desktop_system_tests.rs"]
mod tests;
