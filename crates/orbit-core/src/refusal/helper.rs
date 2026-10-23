//! The link with the Mac's helper (`OrbitNative`): it is not running, its
//! reply cannot be read, or it refused with a code of its own.
use super::Refusal;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ts",
    ts(export_to = "../../apps/desktop/src/bridge/generated/")
)]
#[serde(rename_all = "snake_case")]
pub enum HelperCode {
    AccessibilityPermissionRequired,
    CalendarBusy,
    CalendarClockUnavailable,
    CalendarEventTooLarge,
    CalendarInvalidated,
    CalendarLimitExceeded,
    CalendarPermissionRequired,
    CalendarReadExpired,
    CalendarReadFailed,
    CalendarSelectionUnavailable,
    CaptureClockUnavailable,
    InvalidCalendarRequest,
    InvalidScreenInventory,
    InvalidScreenshotDimensions,
    InvalidWindowBinding,
    InvalidWindowDimensions,
    PrivacyStopped,
    ScreenInventoryBusy,
    ScreenInventoryTooLarge,
    ScreenPermissionRequired,
    ScreenSelectionExpired,
    ScreenSelectionUnavailable,
    ScreenshotBusy,
    ScreenshotExpired,
    ScreenshotSdkFailed,
    ScreenshotTooLarge,
    SelectedTextChanged,
    SelectedTextClassificationIncomplete,
    SelectedTextEmpty,
    SelectedTextExpired,
    SelectedTextIdentityChanged,
    SelectedTextInvalidRange,
    SelectedTextInvalidText,
    SelectedTextLimit,
    SelectedTextNotExposed,
    SelectedTextSecureField,
    SelectedTextUnavailable,
    SelectedTextWindowAmbiguous,
    WindowIdentityChanged,
}

macro_rules! helper_refusals {
    ([$next:ident $(, $rest:ident)*] ; $($entries:tt)*) => {
        $next! { [$($rest),*] ; $($entries)*
            HelperReplyInvalid = 3 => "helper_reply_invalid" {
                field: String,
                expected: Expected,
                received_type: String,
            }, retry Retry::AfterFix;
            HelperRefused = 5 => "helper_refused" {
                #[serde(rename = "helper_code")]
                code: HelperCode,
            }, retry Retry::AfterFix;
            HelperNotRunning = 15 => "helper_not_running", retry Retry::AfterFix;
        }
    };
}
pub(super) use helper_refusals;

impl Refusal {
    pub fn helper_reply_invalid(problem: crate::wire::native::NativeDecodeProblem) -> Self {
        Self::HelperReplyInvalid {
            field: problem.field,
            expected: format!("native_v1/{}", problem.command).into(),
            received_type: problem.observed_type.into(),
        }
    }
}
