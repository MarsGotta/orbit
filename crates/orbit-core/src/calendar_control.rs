//! Local calendar intents and an ephemeral IPC projection, never read authority.
//!
//! REF-10: the macOS calendar permission is the only decision. The daemon
//! observes its state; a client can ask macOS for the permission and read a
//! window of every calendar; there is nothing to list, select or disconnect.
//!
//! P9-21: a client can also create, change and delete events. The same
//! permission is the whole decision: a write is validated and sent, never
//! confirmed, and its outcome — saved, refused by name, or unknown because its
//! answer was lost — is in the view under the client's `request_id`.
use serde::{Deserialize, Serialize};

use crate::calendar::{
    CalendarAuthorization, CalendarDelete, CalendarEntry, CalendarError, CalendarOccurrence,
    CalendarSave, CalendarWindow, CalendarWriteRefusal,
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ts",
    ts(export_to = "../../apps/desktop/src/bridge/generated/")
)]
pub enum CalendarCommand {
    RequestPermission,
    /// Read `window` of every calendar the permission covers, for the agenda
    /// on screen. The daemon reads it in pieces and publishes the result.
    Read {
        window: CalendarWindow,
    },
    /// Create an event or change an occurrence (P9-21). `request_id` is the
    /// client's, to find the outcome in `CalendarView::writes`.
    Save {
        request_id: String,
        save: CalendarSave,
    },
    /// Delete an occurrence, or it and the ones after it (P9-21).
    Delete {
        request_id: String,
        delete: CalendarDelete,
    },
}

impl<'de> Deserialize<'de> for CalendarCommand {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        // Serde unit variants ignore extra map entries even with the enum's
        // deny_unknown_fields. Empty struct variants enforce the same strict
        // wire contract without changing the ergonomic public intent variants.
        #[derive(Deserialize)]
        #[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
        enum Wire {
            RequestPermission {},
            Read {
                window: CalendarWindow,
            },
            Save {
                request_id: String,
                save: CalendarSave,
            },
            Delete {
                request_id: String,
                delete: CalendarDelete,
            },
        }
        Ok(match Wire::deserialize(deserializer)? {
            Wire::RequestPermission {} => Self::RequestPermission,
            Wire::Read { window } => Self::Read { window },
            Wire::Save { request_id, save } => Self::Save { request_id, save },
            Wire::Delete { request_id, delete } => Self::Delete { request_id, delete },
        })
    }
}

impl CalendarCommand {
    pub fn validate(&self) -> Result<(), CalendarError> {
        match self {
            Self::Read { window } => window.validate(),
            Self::RequestPermission => Ok(()),
            Self::Save { .. } | Self::Delete { .. } => self
                .write_refusal()
                .map_or(Ok(()), |_| Err(CalendarError::InvalidValue)),
        }
    }

    /// Why a write cannot be sent, by field and with its numbers; `None` for
    /// a write that may go, and for every command that is not a write.
    pub fn write_refusal(&self) -> Option<CalendarWriteRefusal> {
        let checked = match self {
            Self::Save { request_id, save } => {
                crate::calendar::validate_write_request_id(request_id)
                    .and_then(|()| save.validate())
            }
            Self::Delete { request_id, delete } => {
                crate::calendar::validate_write_request_id(request_id)
                    .and_then(|()| delete.validate())
            }
            Self::RequestPermission | Self::Read { .. } => Ok(()),
        };
        checked.err()
    }
}

/// Which write a record is about.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ts",
    ts(export_to = "../../apps/desktop/src/bridge/generated/")
)]
pub enum CalendarWriteOperation {
    Create,
    Update,
    Delete,
}

/// Where one write is. A write waiting for its turn and one already with
/// EventKit never share a word, and neither is ever waiting for consent:
/// there is none to wait for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ts",
    ts(export_to = "../../apps/desktop/src/bridge/generated/")
)]
pub enum CalendarWriteStage {
    /// Accepted by the daemon, waiting for the single EventKit slot.
    Queued,
    /// Sent to the helper; EventKit is applying it.
    Writing,
    Saved,
    Deleted,
    /// Not applied, and the store is known to be unchanged: `refusal` says why.
    Refused,
    /// Its answer was lost: it may or may not be in the store. The agenda is
    /// read again; the write is never sent a second time.
    OutcomeUnknown,
}

/// One write's outcome, under the identifier its client gave it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CalendarWriteView {
    pub request_id: String,
    pub operation: CalendarWriteOperation,
    pub stage: CalendarWriteStage,
    /// The occurrence as it is after a save, the same shape a read gives.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub occurrence: Option<Box<CalendarOccurrence>>,
    /// Why it was refused, or why its outcome is unknown, with its numbers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refusal: Option<CalendarWriteRefusal>,
    pub detail: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CalendarView {
    pub authorization: CalendarAuthorization,
    /// Every calendar on the Mac, from the last complete listing. `None` means
    /// not known yet — never «none».
    pub calendars: Option<Vec<CalendarEntry>>,
    /// P9-21: the writes waiting, in progress and recently finished, oldest
    /// first. A finished write drops out when newer ones need its place; its
    /// absence then says nothing about the store.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub writes: Vec<CalendarWriteView>,
}

impl Default for CalendarView {
    fn default() -> Self {
        Self {
            authorization: CalendarAuthorization::Unknown,
            calendars: None,
            writes: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_and_restart_do_not_restore_calendar_content() -> Result<(), serde_json::Error> {
        let view = CalendarView::default();
        assert!(view.calendars.is_none());
        let mut saved = serde_json::to_value(crate::State::default())?;
        saved["calendar"] = serde_json::to_value(view)?;
        let state: crate::State = serde_json::from_value(saved)?;
        assert!(
            serde_json::to_value(state.recovered())?
                .get("calendar")
                .is_none()
        );
        assert_eq!(
            serde_json::to_value(CalendarCommand::RequestPermission)?,
            serde_json::json!({"action":"request_permission"})
        );
        Ok(())
    }

    #[test]
    fn ref10_a2_a_read_takes_any_positive_window() -> Result<(), serde_json::Error> {
        let year = serde_json::json!({
            "action": "read",
            "window": {"start_unix_ms": 1_767_225_600_000_i64, "end_unix_ms": 1_798_761_600_000_i64}
        });
        let command: CalendarCommand = serde_json::from_value(year)?;
        assert!(
            command.validate().is_ok(),
            "a year is a window like any other"
        );
        let empty = CalendarCommand::Read {
            window: CalendarWindow {
                start_unix_ms: 10,
                end_unix_ms: 10,
            },
        };
        assert!(empty.validate().is_err());
        Ok(())
    }
}
