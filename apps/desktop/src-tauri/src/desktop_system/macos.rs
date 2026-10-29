//! Native APIs run in Orbit.app, so macOS uses the sending bundle's identity.
//! The only unsafe operations are SDK calls and borrowing callback arguments
//! for the lifetime guaranteed by Apple's completion-handler contracts.
#![allow(unsafe_code)]
use super::*;
use block2::RcBlock;
use objc2_foundation::{NSError, NSString, NSUUID};
use objc2_service_management::{SMAppService, SMAppServiceStatus};
use objc2_user_notifications::{
    UNAuthorizationOptions, UNAuthorizationStatus, UNMutableNotificationContent,
    UNNotificationRequest, UNNotificationSettings, UNNotificationSound, UNUserNotificationCenter,
};
use std::{ptr::NonNull, sync::Mutex};
use tokio::sync::oneshot;

mod presentation;
pub use presentation::install;

pub struct Native;

fn error_result(error: *mut NSError) -> Result<(), String> {
    // Apple owns a non-null NSError throughout this callback; nothing escapes it.
    unsafe { error.as_ref() }.map_or(Ok(()), |error| Err(describe(error)))
}
fn describe(error: &NSError) -> String {
    format!(
        "{} {}: {}",
        error.domain(),
        error.code(),
        error.localizedDescription()
    )
}

impl Platform for Native {
    fn login_status(&self) -> Result<LoginItemState, String> {
        // macOS 15+ is required by Orbit. This is the main bundle, never a helper.
        let status = unsafe { SMAppService::mainAppService().status() };
        match status {
            SMAppServiceStatus::Enabled => Ok(LoginItemState::On),
            SMAppServiceStatus::RequiresApproval => Ok(LoginItemState::RequiresApproval),
            SMAppServiceStatus::NotRegistered | SMAppServiceStatus::NotFound => {
                Ok(LoginItemState::Off)
            }
            other => Err(format!("Unknown macOS login item status {}", other.0)),
        }
    }
    fn set_login(&self, on: bool) -> Result<(), String> {
        // No launch daemon or helper is registered; unregistering cannot stop Orbit.
        unsafe {
            let service = SMAppService::mainAppService();
            let status = service.status();
            if (on && status == SMAppServiceStatus::Enabled)
                || (!on && status == SMAppServiceStatus::NotRegistered)
            {
                return Ok(());
            }
            let result = if on {
                service.registerAndReturnError()
            } else {
                service.unregisterAndReturnError()
            };
            // Registration may require the person's approval; publish that state.
            if on && service.status() == SMAppServiceStatus::RequiresApproval {
                return Ok(());
            }
            result.map_err(|error| describe(&error))
        }
    }
    async fn notification_status(&self) -> Result<PermissionState, String> {
        let received = {
            let (sender, receiver) = oneshot::channel();
            let sender = Mutex::new(Some(sender));
            let block = RcBlock::new(move |settings: NonNull<UNNotificationSettings>| {
                // The center keeps settings alive until this callback returns.
                let status = unsafe { settings.as_ref() }.authorizationStatus();
                let result = match status {
                    UNAuthorizationStatus::NotDetermined => Ok(PermissionState::NotDetermined),
                    UNAuthorizationStatus::Denied => Ok(PermissionState::Denied),
                    UNAuthorizationStatus::Authorized
                    | UNAuthorizationStatus::Provisional
                    | UNAuthorizationStatus::Ephemeral => Ok(PermissionState::Granted),
                    other => Err(format!(
                        "Unknown macOS notification authorization status {}",
                        other.0
                    )),
                };
                if let Some(sender) = sender
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .take()
                {
                    let _ = sender.send(result);
                }
            });
            UNUserNotificationCenter::currentNotificationCenter()
                .getNotificationSettingsWithCompletionHandler(&block);
            receiver
        };
        received
            .await
            .map_err(|_| "macOS notification settings callback closed".to_owned())?
    }
    async fn request_notifications(&self) -> Result<(), String> {
        let received = {
            let (sender, receiver) = oneshot::channel();
            let sender = Mutex::new(Some(sender));
            let block = RcBlock::new(move |_granted: objc2::runtime::Bool, error: *mut NSError| {
                if let Some(sender) = sender
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .take()
                {
                    let _ = sender.send(error_result(error));
                }
            });
            UNUserNotificationCenter::currentNotificationCenter()
                .requestAuthorizationWithOptions_completionHandler(
                    UNAuthorizationOptions::Alert | UNAuthorizationOptions::Sound,
                    &block,
                );
            receiver
        };
        received
            .await
            .map_err(|_| "macOS notification authorization callback closed".to_owned())?
    }
    async fn post_notification(
        &self,
        notice: crate::notify::TurnNotice,
        sound: bool,
    ) -> Result<(), String> {
        let received = {
            let content = UNMutableNotificationContent::new();
            content.setTitle(&NSString::from_str(&notice.title));
            content.setBody(&NSString::from_str(&notice.body));
            if sound {
                content.setSound(Some(&UNNotificationSound::defaultSound()));
            }
            let request = UNNotificationRequest::requestWithIdentifier_content_trigger(
                &NSUUID::UUID().UUIDString(),
                &content,
                None,
            );
            let (sender, receiver) = oneshot::channel();
            let sender = Mutex::new(Some(sender));
            let block = RcBlock::new(move |error: *mut NSError| {
                if let Some(sender) = sender
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .take()
                {
                    let _ = sender.send(error_result(error));
                }
            });
            UNUserNotificationCenter::currentNotificationCenter()
                .addNotificationRequest_withCompletionHandler(&request, Some(&block));
            receiver
        };
        received
            .await
            .map_err(|_| "macOS notification delivery callback closed".to_owned())?
    }
}
