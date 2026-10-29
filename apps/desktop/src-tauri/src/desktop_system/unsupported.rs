use super::*;
pub struct Native;
impl Platform for Native {
    fn login_status(&self) -> Result<LoginItemState, String> {
        Err("Login items require macOS".into())
    }
    fn set_login(&self, _on: bool) -> Result<(), String> {
        Err("Login items require macOS".into())
    }
    async fn notification_status(&self) -> Result<PermissionState, String> {
        Err("Notifications require macOS".into())
    }
    async fn request_notifications(&self) -> Result<(), String> {
        Err("Notifications require macOS".into())
    }
    async fn post_notification(
        &self,
        _notice: crate::notify::TurnNotice,
        _sound: bool,
    ) -> Result<(), String> {
        Err("Notifications require macOS".into())
    }
}
