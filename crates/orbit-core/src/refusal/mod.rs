//! Shared, typed reasons for refusing an operation. Decisions depend on facts
//! supplied by callers; this module never reads the clock or performs effects.
//!
//! The wire knows one [`Refusal`] type with one flat list of codes. Each
//! domain declares its own codes, their causes and when each may be retried,
//! in its own file below; the families every domain shares (too large, busy,
//! timeout…) are in `generic`. A new code goes in its domain's file with the
//! next free place (see `catalogue`); a domain with no file yet gets one,
//! named in the chain at the end of this file.
use serde::{Deserialize, Serialize, Serializer};
use serde_json::Value;
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "ms", rename_all = "snake_case")]
pub enum Retry {
    Never,
    AfterFix,
    AfterMs(u64),
    CheckOutcome,
}

macro_rules! parameter_enum {
    ($name:ident { $($variant:ident => $label:literal),+ $(,)? }) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
        #[cfg_attr(feature = "ts", derive(ts_rs::TS))]
        #[cfg_attr(feature = "ts", ts(export_to = "../../apps/desktop/src/bridge/generated/"))]
        #[serde(rename_all = "snake_case")]
        pub enum $name { $($variant),+ }
        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(match self { $(Self::$variant => $label),+ })
            }
        }
    };
}

mod app;
mod assistant;
mod calendar;
mod control;
mod expected;
mod files;
mod generic;
mod helper;
mod hub;
mod privacy;
mod recording;
mod updates;

pub use app::recovery_step_name;
pub use assistant::{AttachmentCause, AttachmentCeiling, HubSessionField};
pub use calendar::CalendarReadLimit;
pub use control::{EngineCause, SelectionFacts, SelectionFailure, SelectionWindow};
pub use expected::Expected;
pub use generic::{DaemonReplyIssue, InternalCause, ReadInterruption, StorageCause};
pub use helper::HelperCode;
pub use hub::{DeviceReplyRetry, Service};
pub use privacy::{Permission, missing_permission_name, missing_system_permission_refusal};
pub use updates::{InstallLogIssue, UpdateCause, UpdateNetworkCause, UpdateSizeSource};

parameter_enum!(Resource { Subscriptions => "subscriptions", IpcResponse => "ipc_response", IpcRequest => "ipc_request", HubResponse => "hub_response", Message => "message", Recording => "recording", Session => "session", Audio => "audio", Calendar => "calendar", Source => "source", Capture => "capture", ComputerAction => "computer_action", Storage => "storage", Folder => "folder", File => "file" });
parameter_enum!(Operation { IpcRequest => "ipc_request", Subscription => "subscription", Chat => "chat", SessionList => "session_list", Interaction => "interaction", Recording => "recording", Audio => "audio", AudioSession => "audio_session", AudioDrain => "audio_drain", Capture => "capture", Playback => "playback", Call => "call", Calendar => "calendar", ComputerAction => "computer_action", Storage => "storage", Pairing => "pairing", VoiceTranscription => "voice_transcription", PrivacyStop => "privacy_stop", Files => "files" });
parameter_enum!(Parameter { Text => "text", Duration => "duration", Title => "title", ApplicationPid => "application_pid", Request => "request", Selection => "selection", Identifier => "identifier", Meeting => "meeting" });

/// The end of the chain: every domain's codes, gathered in the order the
/// chain visits the domains, become the one wire type. Each entry is
/// `Variant = place => "code" { fields }, retry rule;`, the rule an
/// expression over the variant's fields.
macro_rules! refusal_catalogue {
    ([] ; $(
        $(#[$meta:meta])*
        $variant:ident = $place:literal => $code:literal
        $( { $($(#[$attr:meta])* $field:ident: $ty:ty),* $(,)? } )?,
        retry $retry:expr;
    )*) => {
        #[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
        #[serde(tag = "code", rename_all = "snake_case")]
        pub enum Refusal { $($(#[$meta])* $variant $( { $($(#[$attr])* $field: $ty),* } )?,)* }

        #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
        #[serde(rename_all = "snake_case")]
        pub enum RefusalCode { $($variant = $place,)* }
        crate::catalogue::wire_order!(RefusalCode { $($variant),* });
        crate::catalogue::ts_union!(RefusalCode, []);

        impl RefusalCode {
            pub fn name(self) -> &'static str { match self { $(Self::$variant => $code,)* } }
        }

        impl Refusal {
            pub fn code(&self) -> RefusalCode { match self { $(Self::$variant { .. } => RefusalCode::$variant,)* } }

            /// Each code's rule is declared beside it, in its domain.
            #[allow(unused_variables)]
            pub fn retry(&self) -> Retry {
                match self { $(Self::$variant $( { $($field),* } )? => $retry,)* }
            }
        }

        /// The wire shape of a refusal without its `retry`, which
        /// [`Refusal`]'s `Serialize` adds from [`Refusal::retry`].
        #[derive(Serialize)]
        #[serde(tag = "code", rename_all = "snake_case")]
        enum Wire<'a> { $($variant $( { $($(#[$attr])* $field: &'a $ty),* } )?,)* }

        impl<'a> From<&'a Refusal> for Wire<'a> {
            fn from(refusal: &'a Refusal) -> Self {
                match refusal { $(Refusal::$variant $( { $($field),* } )? => Self::$variant $( { $($field),* } )?,)* }
            }
        }
    };
}

/// `skip_serializing_if` for an optional field of [`Wire`], which holds it by reference.
fn absent<T>(value: &&Option<T>) -> bool {
    value.is_none()
}

use app::app_refusals;
use assistant::assistant_refusals;
use calendar::calendar_refusals;
use control::control_refusals;
use files::files_refusals;
use generic::generic_refusals;
use helper::helper_refusals;
use hub::hub_refusals;
use privacy::privacy_refusals;
use recording::recording_refusals;
use updates::updates_refusals;

generic_refusals! { [
    app_refusals, recording_refusals, calendar_refusals, assistant_refusals,
    control_refusals, files_refusals, privacy_refusals, updates_refusals,
    hub_refusals, helper_refusals, refusal_catalogue
] ; }

impl Refusal {
    /// The hub cannot be asked now: this Mac is not paired, has no link to
    /// it, or the user's privacy stop is on. What the hub keeps (notes,
    /// templates) is then unknown here, never a failure of the read: the
    /// connection and the stop say themselves elsewhere (seen in a run of the
    /// real test on the Mac).
    pub fn needs_the_hub(&self) -> bool {
        matches!(
            self,
            Self::NotPaired | Self::HubUnreachable { .. } | Self::PrivacyStopped { .. }
        )
    }
}

impl Serialize for Refusal {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut value =
            serde_json::to_value(Wire::from(self)).map_err(serde::ser::Error::custom)?;
        if let Value::Object(ref mut fields) = value {
            fields.insert(
                "retry".into(),
                serde_json::to_value(self.retry()).map_err(serde::ser::Error::custom)?,
            );
        }
        value.serialize(serializer)
    }
}

// Error adapters require Display. Diagnostics retain the wire code and fields;
// the interface and MCP own the user-facing sentences.
impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match serde_json::to_string(self) {
            Ok(value) => f.write_str(&value),
            Err(_) => f.write_str(self.code().name()),
        }
    }
}
impl std::error::Error for Refusal {}

#[cfg(test)]
mod tests {
    use super::RefusalCode;

    /// Places are the codes' order on the wire: no gap, so the next code's
    /// place is the number of codes there are.
    #[test]
    fn every_place_is_taken_once_from_zero() {
        for (place, code) in RefusalCode::ALL.iter().enumerate() {
            assert_eq!(*code as usize, place, "{code:?}");
        }
    }
}
