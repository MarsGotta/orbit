//! Writes to the Mac's calendars, decided before they reach the helper.
//!
//! The user creates, moves, edits and deletes events from Orbit as they would in
//! Calendar. The macOS permission (full access) is the whole decision: there
//! is no confirmation and no second consent, and a delete is undone in the
//! app, not asked about.
//!
//! An occurrence is named by `calendar_item_id` and `occurrence_start_ms`, the
//! original date of that occurrence — never by `observation_id`, which is new
//! on every read, nor by the EventKit event identifier alone, which may be
//! absent. EventKit offers two spans, `this` and `future`; the whole series is
//! `future` from its first occurrence, and no third value exists.
//!
//! Everything here is pure. Every refusal names its field and carries its
//! numbers (bytes and limit, start and end) all the way to whoever asked.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use super::{
    validation::{ascii_control, identifier},
    *,
};

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ts",
    ts(export_to = "../../apps/desktop/src/bridge/generated/")
)]
pub enum CalendarAvailability {
    Busy,
    Free,
    Tentative,
    Unavailable,
    /// The calendar does not keep availability (EventKit's `notSupported`).
    /// Read only: a write never asks for it.
    NotSupported,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ts",
    ts(export_to = "../../apps/desktop/src/bridge/generated/")
)]
pub enum CalendarFrequency {
    Daily,
    Weekly,
    Monthly,
    Yearly,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ts",
    ts(export_to = "../../apps/desktop/src/bridge/generated/")
)]
pub enum CalendarWeekday {
    Sunday,
    Monday,
    Tuesday,
    Wednesday,
    Thursday,
    Friday,
    Saturday,
}

/// One day a rule repeats on. `week` (−53…53, never 0) picks one week of the
/// month or year — «the 2nd Wednesday» is `{day:wednesday, week:2}`, «the
/// last Friday» `{day:friday, week:-1}` — and is absent for every week.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ts",
    ts(export_to = "../../apps/desktop/src/bridge/generated/")
)]
pub struct CalendarRecurrenceDay {
    pub day: CalendarWeekday,
    #[serde(
        default,
        deserialize_with = "super::wire::present",
        skip_serializing_if = "Option::is_none"
    )]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub week: Option<i8>,
}

/// Where a rule stops: on a date (`until_ms`) or after a number of
/// occurrences (`count`), exactly one of them. Absent end: it never stops.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ts",
    ts(export_to = "../../apps/desktop/src/bridge/generated/")
)]
pub struct CalendarRecurrenceEnd {
    #[serde(
        default,
        deserialize_with = "super::wire::present",
        skip_serializing_if = "Option::is_none"
    )]
    #[cfg_attr(feature = "ts", ts(type = "number", optional))]
    pub until_ms: Option<i64>,
    #[serde(
        default,
        deserialize_with = "super::wire::present",
        skip_serializing_if = "Option::is_none"
    )]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub count: Option<u32>,
}

/// One repetition rule, as a write asks for it.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ts",
    ts(export_to = "../../apps/desktop/src/bridge/generated/")
)]
pub struct CalendarRecurrenceRule {
    pub frequency: CalendarFrequency,
    pub interval: u32,
    #[serde(
        default,
        deserialize_with = "super::wire::present",
        skip_serializing_if = "Option::is_none"
    )]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub days_of_week: Option<Vec<CalendarRecurrenceDay>>,
    #[serde(
        default,
        deserialize_with = "super::wire::present",
        skip_serializing_if = "Option::is_none"
    )]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub end: Option<CalendarRecurrenceEnd>,
}

/// How an event read from the Mac repeats: its first rule, and `rules_count`
/// when EventKit holds more than one (absent with exactly one).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ts",
    ts(export_to = "../../apps/desktop/src/bridge/generated/")
)]
pub struct CalendarRecurrence {
    pub frequency: CalendarFrequency,
    pub interval: u32,
    #[serde(
        default,
        deserialize_with = "super::wire::present",
        skip_serializing_if = "Option::is_none"
    )]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub days_of_week: Option<Vec<CalendarRecurrenceDay>>,
    #[serde(
        default,
        deserialize_with = "super::wire::present",
        skip_serializing_if = "Option::is_none"
    )]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub end: Option<CalendarRecurrenceEnd>,
    #[serde(
        default,
        deserialize_with = "super::wire::present",
        skip_serializing_if = "Option::is_none"
    )]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub rules_count: Option<u32>,
}

/// Which occurrences of a repeating event a change or a delete reaches.
/// EventKit's `.thisEvent` and `.futureEvents`; the whole series is `future`
/// from its first occurrence.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ts",
    ts(export_to = "../../apps/desktop/src/bridge/generated/")
)]
pub enum CalendarSpan {
    This,
    Future,
}

/// A new event. Without `calendar_id` it goes to the calendar macOS uses for
/// new events.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ts",
    ts(export_to = "../../apps/desktop/src/bridge/generated/")
)]
pub struct CalendarEventDraft {
    #[serde(
        default,
        deserialize_with = "super::wire::present",
        skip_serializing_if = "Option::is_none"
    )]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub calendar_id: Option<String>,
    pub title: String,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub start_ms: i64,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub end_ms: i64,
    pub all_day: bool,
    #[serde(
        default,
        deserialize_with = "super::wire::present",
        skip_serializing_if = "Option::is_none"
    )]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub time_zone: Option<String>,
    #[serde(
        default,
        deserialize_with = "super::wire::present",
        skip_serializing_if = "Option::is_none"
    )]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub location: Option<String>,
    #[serde(
        default,
        deserialize_with = "super::wire::present",
        skip_serializing_if = "Option::is_none"
    )]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub notes: Option<String>,
    #[serde(
        default,
        deserialize_with = "super::wire::present",
        skip_serializing_if = "Option::is_none"
    )]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub url: Option<String>,
    #[serde(
        default,
        deserialize_with = "super::wire::present",
        skip_serializing_if = "Option::is_none"
    )]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub availability: Option<CalendarAvailability>,
    #[serde(
        default,
        deserialize_with = "super::wire::present",
        skip_serializing_if = "Option::is_none"
    )]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub recurrence: Option<CalendarRecurrenceRule>,
    /// Minutes before the start each alarm sounds (0: at the start).
    #[serde(
        default,
        deserialize_with = "super::wire::present",
        skip_serializing_if = "Option::is_none"
    )]
    #[cfg_attr(feature = "ts", ts(optional))]
    #[cfg_attr(feature = "ts", ts(type = "Array<number>", optional))]
    pub alarms_minutes: Option<Vec<u64>>,
}

/// What a change sets. Every field is optional and absent leaves it as it
/// is. An empty `location`, `notes` or `url` clears it; `remove_recurrence`
/// makes the event stop repeating; `alarms_minutes` replaces every alarm (an
/// empty list removes them).
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ts",
    ts(export_to = "../../apps/desktop/src/bridge/generated/")
)]
pub struct CalendarEventChanges {
    #[serde(
        default,
        deserialize_with = "super::wire::present",
        skip_serializing_if = "Option::is_none"
    )]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub calendar_id: Option<String>,
    #[serde(
        default,
        deserialize_with = "super::wire::present",
        skip_serializing_if = "Option::is_none"
    )]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub title: Option<String>,
    #[serde(
        default,
        deserialize_with = "super::wire::present",
        skip_serializing_if = "Option::is_none"
    )]
    #[cfg_attr(feature = "ts", ts(type = "number", optional))]
    pub start_ms: Option<i64>,
    #[serde(
        default,
        deserialize_with = "super::wire::present",
        skip_serializing_if = "Option::is_none"
    )]
    #[cfg_attr(feature = "ts", ts(type = "number", optional))]
    pub end_ms: Option<i64>,
    #[serde(
        default,
        deserialize_with = "super::wire::present",
        skip_serializing_if = "Option::is_none"
    )]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub all_day: Option<bool>,
    #[serde(
        default,
        deserialize_with = "super::wire::present",
        skip_serializing_if = "Option::is_none"
    )]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub time_zone: Option<String>,
    #[serde(
        default,
        deserialize_with = "super::wire::present",
        skip_serializing_if = "Option::is_none"
    )]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub location: Option<String>,
    #[serde(
        default,
        deserialize_with = "super::wire::present",
        skip_serializing_if = "Option::is_none"
    )]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub notes: Option<String>,
    #[serde(
        default,
        deserialize_with = "super::wire::present",
        skip_serializing_if = "Option::is_none"
    )]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub url: Option<String>,
    #[serde(
        default,
        deserialize_with = "super::wire::present",
        skip_serializing_if = "Option::is_none"
    )]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub availability: Option<CalendarAvailability>,
    #[serde(
        default,
        deserialize_with = "super::wire::present",
        skip_serializing_if = "Option::is_none"
    )]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub recurrence: Option<CalendarRecurrenceRule>,
    #[serde(
        default,
        deserialize_with = "super::wire::present",
        skip_serializing_if = "Option::is_none"
    )]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub remove_recurrence: Option<bool>,
    #[serde(
        default,
        deserialize_with = "super::wire::present",
        skip_serializing_if = "Option::is_none"
    )]
    #[cfg_attr(feature = "ts", ts(optional))]
    #[cfg_attr(feature = "ts", ts(type = "Array<number>", optional))]
    pub alarms_minutes: Option<Vec<u64>>,
}

/// A change to one occurrence, and through `span` to the ones after it.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ts",
    ts(export_to = "../../apps/desktop/src/bridge/generated/")
)]
pub struct CalendarEventChange {
    pub calendar_item_id: String,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub occurrence_start_ms: i64,
    pub span: CalendarSpan,
    pub changes: CalendarEventChanges,
}

/// `calendar_save`: a new event, or a change to an existing occurrence.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ts",
    ts(export_to = "../../apps/desktop/src/bridge/generated/")
)]
pub enum CalendarSave {
    // Boxed: an event's fields would make every command that carries a
    // write as large as the write.
    Create(Box<CalendarEventDraft>),
    Change(Box<CalendarEventChange>),
}

/// `calendar_delete`: one occurrence, or it and the ones after it.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ts",
    ts(export_to = "../../apps/desktop/src/bridge/generated/")
)]
pub struct CalendarDelete {
    pub calendar_item_id: String,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub occurrence_start_ms: i64,
    pub span: CalendarSpan,
}

/// The helper's answer to a `calendar_save`: the occurrence as it is now, in
/// the same shape a read gives, so the app can draw it before the agenda is
/// read again.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CalendarSaveResult {
    pub write_id: String,
    pub native_instance_id: String,
    pub privacy_generation: u64,
    pub system_timezone: String,
    pub occurrence: CalendarOccurrence,
}

/// The helper's answer to a `calendar_delete`: what it removed.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CalendarDeleteResult {
    pub write_id: String,
    pub native_instance_id: String,
    pub privacy_generation: u64,
    pub calendar_item_id: String,
    pub occurrence_start_ms: i64,
    pub span: CalendarSpan,
}

/// Why an event cannot be changed from Orbit.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CalendarNotEditable {
    /// The calendar does not admit changes (a subscription, birthdays, a
    /// shared calendar with read access).
    CalendarReadOnly,
    /// Someone else organized it: an invitation is moved by whoever sent it.
    OrganizedBySomeoneElse,
}

/// What is wrong with one field of a write.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CalendarChangeProblem {
    /// Empty or only blanks where a value is required (`title`).
    Empty,
    /// Longer than its limit: `bytes` against `limit`.
    TooLarge,
    /// More items than the limit: `count` against `limit`.
    TooMany,
    /// The end comes before the start: `start_ms` and `end_ms`.
    EndBeforeStart,
    /// Not a date Orbit can represent (years 1–9999): `value`.
    InvalidDate,
    /// Not an identifier: empty, too long or with control characters.
    InvalidIdentifier,
    /// Malformed: control characters, a value out of its range (`value`), or
    /// two ways of saying the same thing.
    InvalidValue,
    /// No calendar with that identifier on this Mac.
    UnknownCalendar,
    /// macOS has no default calendar for new events.
    NoDefaultCalendar,
    /// Not a time zone macOS knows.
    UnknownTimeZone,
    /// Not a URL macOS can parse.
    InvalidUrl,
    /// A change of the repetition reaches this occurrence and the ones after
    /// it (`future`), never this one alone.
    RecurrenceNeedsFutureSpan,
    /// A change that sets nothing.
    NothingToChange,
}

/// One field of a write that cannot be applied, and why, with its numbers.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CalendarInvalidChange {
    pub field: String,
    pub problem: CalendarChangeProblem,
    #[serde(
        default,
        deserialize_with = "super::wire::present",
        skip_serializing_if = "Option::is_none"
    )]
    pub bytes: Option<u64>,
    #[serde(
        default,
        deserialize_with = "super::wire::present",
        skip_serializing_if = "Option::is_none"
    )]
    pub count: Option<u64>,
    #[serde(
        default,
        deserialize_with = "super::wire::present",
        skip_serializing_if = "Option::is_none"
    )]
    pub limit: Option<u64>,
    #[serde(
        default,
        deserialize_with = "super::wire::present",
        skip_serializing_if = "Option::is_none"
    )]
    pub value: Option<i64>,
    #[serde(
        default,
        deserialize_with = "super::wire::present",
        skip_serializing_if = "Option::is_none"
    )]
    pub start_ms: Option<i64>,
    #[serde(
        default,
        deserialize_with = "super::wire::present",
        skip_serializing_if = "Option::is_none"
    )]
    pub end_ms: Option<i64>,
}

impl CalendarInvalidChange {
    fn problem(field: &str, problem: CalendarChangeProblem) -> Self {
        Self {
            field: field.into(),
            problem,
            bytes: None,
            count: None,
            limit: None,
            value: None,
            start_ms: None,
            end_ms: None,
        }
    }

    fn too_large(field: &str, bytes: usize, limit: usize) -> Self {
        Self {
            bytes: Some(u64::try_from(bytes).unwrap_or(u64::MAX)),
            limit: Some(u64::try_from(limit).unwrap_or(u64::MAX)),
            ..Self::problem(field, CalendarChangeProblem::TooLarge)
        }
    }

    fn value(field: &str, problem: CalendarChangeProblem, value: i64) -> Self {
        Self {
            value: Some(value),
            ..Self::problem(field, problem)
        }
    }
}

/// A write the helper (or Orbit, before sending it) did not apply, or whose
/// outcome nobody can know. The `code` is the native error code; the rest is
/// the error's `detail`.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "code", deny_unknown_fields)]
pub enum CalendarWriteRefusal {
    /// No occurrence of that event starts (originally) at that date.
    #[serde(rename = "calendar_event_not_found")]
    EventNotFound {
        calendar_item_id: String,
        occurrence_start_ms: i64,
    },
    #[serde(rename = "calendar_not_editable")]
    NotEditable {
        reason: CalendarNotEditable,
        calendar_id: String,
    },
    /// Boxed: the field's numbers make it the largest refusal by far.
    #[serde(rename = "calendar_invalid_change")]
    InvalidChange(Box<CalendarInvalidChange>),
    /// The macOS permission as EventKit reports it now.
    #[serde(rename = "calendar_permission_denied")]
    PermissionDenied {
        authorization: CalendarAuthorization,
    },
    /// EventKit refused the save or the removal: its error domain and code,
    /// never its text.
    #[serde(rename = "calendar_save_failed")]
    SaveFailed {
        error_domain: String,
        error_code: i64,
    },
    /// The write may or may not have reached the store: its answer was lost.
    /// Orbit reads the agenda again and never sends it a second time.
    #[serde(rename = "calendar_write_outcome_unknown")]
    OutcomeUnknown { cause: String },
    /// Known not to have been written: refused before EventKit was asked
    /// (`code` says why: `privacy_stopped`, `calendar_busy`…).
    #[serde(rename = "calendar_not_written")]
    NotWritten { cause: String },
    /// The daemon already follows `limit` writes waiting or with EventKit;
    /// this one was not sent (the daemon's own refusal, never the helper's).
    #[serde(rename = "calendar_write_queue_full")]
    QueueFull { unfinished: u64, limit: u64 },
}

/// The codes that are write refusals with a typed detail.
const WRITE_REFUSAL_CODES: [&str; 6] = [
    "calendar_event_not_found",
    "calendar_not_editable",
    "calendar_invalid_change",
    "calendar_permission_denied",
    "calendar_save_failed",
    "calendar_write_outcome_unknown",
];

impl std::fmt::Display for CalendarWriteRefusal {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.notice())
    }
}

/// A refusal is also the error a daemon command returns: its text is the
/// notice, with every number.
impl std::error::Error for CalendarWriteRefusal {}

impl CalendarWriteRefusal {
    pub fn code(&self) -> &'static str {
        match self {
            Self::EventNotFound { .. } => "calendar_event_not_found",
            Self::NotEditable { .. } => "calendar_not_editable",
            Self::InvalidChange(_) => "calendar_invalid_change",
            Self::PermissionDenied { .. } => "calendar_permission_denied",
            Self::SaveFailed { .. } => "calendar_save_failed",
            Self::OutcomeUnknown { .. } => "calendar_write_outcome_unknown",
            Self::NotWritten { .. } => "calendar_not_written",
            Self::QueueFull { .. } => "calendar_write_queue_full",
        }
    }

    /// Whether the store is known to be unchanged by this write.
    pub fn known_not_written(&self) -> bool {
        !matches!(self, Self::OutcomeUnknown { .. })
    }

    /// The refusal a helper error (`code` and its `detail`) stands for. A code
    /// that is not a write refusal — `privacy_stopped`, `calendar_busy`,
    /// `calendar_invalidated` before admission, `invalid_calendar_request` —
    /// was refused before EventKit was asked, and is kept by name. A refusal
    /// whose detail does not match its code is `Err`: the reply broke the
    /// contract and is not a fact about the store.
    pub fn from_native(
        code: &str,
        detail: Option<&serde_json::Value>,
    ) -> Result<Self, CalendarError> {
        if !WRITE_REFUSAL_CODES.contains(&code) {
            if code.is_empty() || code.len() > 64 || ascii_control(code) {
                return Err(CalendarError::InvalidWire);
            }
            return Ok(Self::NotWritten { cause: code.into() });
        }
        let mut object = match detail {
            Some(serde_json::Value::Object(fields)) => fields.clone(),
            None => serde_json::Map::new(),
            Some(_) => return Err(CalendarError::InvalidWire),
        };
        if object.contains_key("code") {
            return Err(CalendarError::InvalidWire);
        }
        object.insert("code".into(), serde_json::Value::from(code));
        let refusal: Self = serde_json::from_value(serde_json::Value::Object(object))
            .map_err(|_| CalendarError::InvalidWire)?;
        refusal.validate()?;
        Ok(refusal)
    }

    /// The detail object that travels next to the code on the native wire.
    pub fn detail(&self) -> serde_json::Value {
        let mut value = serde_json::to_value(self).unwrap_or(serde_json::Value::Null);
        if let Some(fields) = value.as_object_mut() {
            fields.remove("code");
        }
        value
    }

    pub fn validate(&self) -> Result<(), CalendarError> {
        let valid = match self {
            Self::EventNotFound {
                calendar_item_id,
                occurrence_start_ms,
            } => identifier(calendar_item_id) && crate::wire::unix_ms(*occurrence_start_ms),
            Self::NotEditable { calendar_id, .. } => identifier(calendar_id),
            Self::InvalidChange(change) => {
                !change.field.is_empty()
                    && change.field.len() <= 64
                    && !ascii_control(&change.field)
            }
            Self::PermissionDenied { authorization } => {
                *authorization != CalendarAuthorization::FullAccess
            }
            Self::SaveFailed { error_domain, .. } => {
                !error_domain.is_empty()
                    && error_domain.len() <= 256
                    && !ascii_control(error_domain)
            }
            Self::OutcomeUnknown { cause } | Self::NotWritten { cause } => {
                !cause.is_empty() && cause.len() <= 64 && !ascii_control(cause)
            }
            Self::QueueFull { unfinished, limit } => unfinished >= limit,
        };
        if valid {
            Ok(())
        } else {
            Err(CalendarError::InvalidValue)
        }
    }

    /// What the user and Deimos read: Spanish, with every number the refusal has.
    pub fn notice(&self) -> String {
        match self {
            Self::EventNotFound {
                calendar_item_id,
                occurrence_start_ms,
            } => format!(
                "El evento {calendar_item_id} no tiene ninguna ocurrencia que empiece en {occurrence_start_ms} (ms Unix) en este Mac: se movió o se borró fuera de Orbit. No se cambió nada; la agenda se vuelve a leer."
            ),
            Self::NotEditable {
                reason: CalendarNotEditable::CalendarReadOnly,
                calendar_id,
            } => format!(
                "El calendario {calendar_id} es de sólo lectura en macOS (una suscripción, los cumpleaños o un calendario compartido sin permiso de escritura). No se cambió nada."
            ),
            Self::NotEditable {
                reason: CalendarNotEditable::OrganizedBySomeoneElse,
                calendar_id,
            } => format!(
                "Este evento del calendario {calendar_id} lo organizó otra persona: sólo quien lo organizó puede moverlo o cambiarlo. No se cambió nada."
            ),
            Self::InvalidChange(change) => change.notice(),
            Self::PermissionDenied { authorization } => format!(
                "macOS no da a Orbit acceso completo a Calendarios (estado de EventKit: {}). Concédelo en Ajustes del Sistema > Privacidad y seguridad > Calendarios. No se cambió nada.",
                serde_json::to_value(authorization)
                    .ok()
                    .and_then(|value| value.as_str().map(str::to_owned))
                    .unwrap_or_else(|| "desconocido".into())
            ),
            Self::SaveFailed {
                error_domain,
                error_code,
            } => format!(
                "EventKit rechazó el cambio (error {error_domain} {error_code}). No se cambió nada."
            ),
            Self::OutcomeUnknown { cause } => format!(
                "No se sabe si el cambio llegó al calendario ({cause}): su respuesta se perdió. Orbit vuelve a leer la agenda y no lo repite."
            ),
            Self::NotWritten { cause } if cause == "calendar_helper_read_only" => {
                "El helper de este Mac anuncia la capacidad `calendar_read_only`: lee el calendario pero no escribe en él. Actualiza Orbit en este Mac. No se cambió nada.".into()
            }
            Self::NotWritten { cause } => {
                format!("El cambio no se hizo ({cause}); el calendario sigue como estaba.")
            }
            Self::QueueFull { unfinished, limit } => format!(
                "Hay {unfinished} cambios del calendario esperando o en curso, el máximo que Orbit sigue a la vez ({limit}); éste no se envió. El calendario sigue como estaba."
            ),
        }
    }
}

impl CalendarInvalidChange {
    fn notice(&self) -> String {
        let field = &self.field;
        let numbers = |text: &str| {
            format!(
                "{text}{}{}{}",
                self.bytes.map_or(String::new(), |bytes| format!(
                    " ({bytes} bytes; caben {})",
                    self.limit.unwrap_or_default()
                )),
                self.count.map_or(String::new(), |count| format!(
                    " ({count}; caben {})",
                    self.limit.unwrap_or_default()
                )),
                self.value
                    .map_or(String::new(), |value| format!(" (valor {value})")),
            )
        };
        let what = match self.problem {
            CalendarChangeProblem::Empty => format!("«{field}» está vacío"),
            CalendarChangeProblem::TooLarge => numbers(&format!("«{field}» es demasiado largo")),
            CalendarChangeProblem::TooMany => {
                numbers(&format!("«{field}» tiene demasiados elementos"))
            }
            CalendarChangeProblem::EndBeforeStart => format!(
                "el fin ({}) es anterior al inicio ({})",
                self.end_ms.unwrap_or_default(),
                self.start_ms.unwrap_or_default()
            ),
            CalendarChangeProblem::InvalidDate => {
                numbers(&format!("«{field}» no es una fecha válida"))
            }
            CalendarChangeProblem::InvalidIdentifier => {
                format!("«{field}» no es un identificador válido")
            }
            CalendarChangeProblem::InvalidValue => numbers(&format!("«{field}» no es válido")),
            CalendarChangeProblem::UnknownCalendar => "ese calendario no existe en este Mac".into(),
            CalendarChangeProblem::NoDefaultCalendar => {
                "macOS no tiene un calendario por defecto para eventos nuevos".into()
            }
            CalendarChangeProblem::UnknownTimeZone => "macOS no conoce esa zona horaria".into(),
            CalendarChangeProblem::InvalidUrl => "la URL no es válida".into(),
            CalendarChangeProblem::RecurrenceNeedsFutureSpan => {
                "cambiar la repetición afecta a este evento y a los siguientes, no sólo a este"
                    .into()
            }
            CalendarChangeProblem::NothingToChange => "el cambio no modifica nada".into(),
        };
        format!("No se guardó el evento: {what}. No se cambió nada.")
    }
}

type Checked = Result<(), CalendarWriteRefusal>;

fn refuse(change: CalendarInvalidChange) -> Checked {
    Err(CalendarWriteRefusal::InvalidChange(Box::new(change)))
}

fn within_limit(field: &str, value: &str, limit: usize) -> Checked {
    if value.len() > limit {
        return refuse(CalendarInvalidChange::too_large(field, value.len(), limit));
    }
    Ok(())
}

fn check_date(field: &str, value: i64) -> Checked {
    if !crate::wire::unix_ms(value) {
        return refuse(CalendarInvalidChange::value(
            field,
            CalendarChangeProblem::InvalidDate,
            value,
        ));
    }
    Ok(())
}

fn id(field: &str, value: &str) -> Checked {
    if !identifier(value) {
        return refuse(CalendarInvalidChange::problem(
            field,
            CalendarChangeProblem::InvalidIdentifier,
        ));
    }
    Ok(())
}

fn ordered(start_ms: i64, end_ms: i64) -> Checked {
    if end_ms < start_ms {
        return refuse(CalendarInvalidChange {
            start_ms: Some(start_ms),
            end_ms: Some(end_ms),
            ..CalendarInvalidChange::problem("end_ms", CalendarChangeProblem::EndBeforeStart)
        });
    }
    Ok(())
}

fn title(value: &str) -> Checked {
    if value.trim().is_empty() {
        return refuse(CalendarInvalidChange::problem(
            "title",
            CalendarChangeProblem::Empty,
        ));
    }
    within_limit("title", value, MAX_TITLE_BYTES)
}

fn time_zone(value: &str) -> Checked {
    if value.is_empty() || ascii_control(value) {
        return refuse(CalendarInvalidChange::problem(
            "time_zone",
            CalendarChangeProblem::InvalidValue,
        ));
    }
    within_limit("time_zone", value, MAX_TIMEZONE_BYTES)
}

// An edit can clear a URL; a new event omits the field instead.
fn url(value: &str, allows_empty: bool) -> Checked {
    within_limit("url", value, MAX_URL_BYTES)?;
    if ascii_control(value) || (!allows_empty && value.is_empty()) {
        return refuse(CalendarInvalidChange::problem(
            "url",
            CalendarChangeProblem::InvalidUrl,
        ));
    }
    Ok(())
}

fn alarms(values: &[u64]) -> Checked {
    if values.len() > MAX_EVENT_ALARMS {
        return refuse(CalendarInvalidChange {
            count: Some(u64::try_from(values.len()).unwrap_or(u64::MAX)),
            limit: Some(MAX_EVENT_ALARMS as u64),
            ..CalendarInvalidChange::problem("alarms_minutes", CalendarChangeProblem::TooMany)
        });
    }
    if let Some(far) = values.iter().find(|minutes| **minutes > MAX_ALARM_MINUTES) {
        return refuse(CalendarInvalidChange {
            value: Some(i64::try_from(*far).unwrap_or(i64::MAX)),
            limit: Some(MAX_ALARM_MINUTES),
            ..CalendarInvalidChange::problem("alarms_minutes", CalendarChangeProblem::InvalidValue)
        });
    }
    Ok(())
}

fn days(values: &[CalendarRecurrenceDay]) -> bool {
    let mut seen = HashSet::new();
    values.iter().all(|day| {
        day.week
            .is_none_or(|week| week != 0 && (-53..=53).contains(&week))
            && seen.insert(*day)
    })
}

fn rule_shape(
    interval: u32,
    days_of_week: Option<&[CalendarRecurrenceDay]>,
    end: Option<&CalendarRecurrenceEnd>,
    start_ms: Option<i64>,
) -> Checked {
    let invalid = || {
        refuse(CalendarInvalidChange::problem(
            "recurrence",
            CalendarChangeProblem::InvalidValue,
        ))
    };
    if interval == 0 || days_of_week.is_some_and(|values| !days(values)) {
        return invalid();
    }
    if let Some(end) = end {
        match (end.until_ms, end.count) {
            (Some(until), None) => {
                check_date("recurrence", until)?;
                if start_ms.is_some_and(|start| until < start) {
                    return refuse(CalendarInvalidChange {
                        start_ms,
                        end_ms: Some(until),
                        ..CalendarInvalidChange::problem(
                            "recurrence",
                            CalendarChangeProblem::EndBeforeStart,
                        )
                    });
                }
            }
            (None, Some(count)) if count > 0 => {}
            _ => return invalid(),
        }
    }
    Ok(())
}

impl CalendarRecurrenceRule {
    fn validate(&self, start_ms: Option<i64>) -> Checked {
        rule_shape(
            self.interval,
            self.days_of_week.as_deref(),
            self.end.as_ref(),
            start_ms,
        )
    }
}

impl CalendarRecurrence {
    /// A read rule's structure; `rules_count`, when present, counts more than one.
    pub(super) fn validate(&self) -> Result<(), CalendarError> {
        rule_shape(
            self.interval,
            self.days_of_week.as_deref(),
            self.end.as_ref(),
            None,
        )
        .map_err(|_| CalendarError::InvalidValue)?;
        if self.rules_count.is_some_and(|count| count < 2) {
            return Err(CalendarError::InvalidValue);
        }
        Ok(())
    }
}

fn availability(value: CalendarAvailability) -> Checked {
    if value == CalendarAvailability::NotSupported {
        return refuse(CalendarInvalidChange::problem(
            "availability",
            CalendarChangeProblem::InvalidValue,
        ));
    }
    Ok(())
}

impl CalendarEventDraft {
    pub fn validate(&self) -> Checked {
        if let Some(calendar) = &self.calendar_id {
            id("calendar_id", calendar)?;
        }
        title(&self.title)?;
        check_date("start_ms", self.start_ms)?;
        check_date("end_ms", self.end_ms)?;
        ordered(self.start_ms, self.end_ms)?;
        if let Some(zone) = &self.time_zone {
            time_zone(zone)?;
        }
        if let Some(location) = &self.location {
            within_limit("location", location, MAX_TITLE_BYTES)?;
        }
        if let Some(notes) = &self.notes {
            within_limit("notes", notes, MAX_NOTES_BYTES)?;
        }
        if let Some(value) = &self.url {
            url(value, false)?;
        }
        if let Some(value) = self.availability {
            availability(value)?;
        }
        if let Some(rule) = &self.recurrence {
            rule.validate(Some(self.start_ms))?;
        }
        if let Some(values) = &self.alarms_minutes {
            alarms(values)?;
        }
        Ok(())
    }
}

impl CalendarEventChanges {
    fn is_empty(&self) -> bool {
        self == &Self::default()
    }

    pub fn validate(&self, span: CalendarSpan) -> Checked {
        if self.is_empty()
            || self.remove_recurrence == Some(false) && self.without_removal().is_empty()
        {
            return refuse(CalendarInvalidChange::problem(
                "changes",
                CalendarChangeProblem::NothingToChange,
            ));
        }
        if let Some(calendar) = &self.calendar_id {
            id("calendar_id", calendar)?;
        }
        if let Some(value) = &self.title {
            title(value)?;
        }
        if let Some(start) = self.start_ms {
            check_date("start_ms", start)?;
        }
        if let Some(end) = self.end_ms {
            check_date("end_ms", end)?;
        }
        if let (Some(start), Some(end)) = (self.start_ms, self.end_ms) {
            ordered(start, end)?;
        }
        if let Some(zone) = &self.time_zone {
            time_zone(zone)?;
        }
        if let Some(location) = &self.location {
            within_limit("location", location, MAX_TITLE_BYTES)?;
        }
        if let Some(notes) = &self.notes {
            within_limit("notes", notes, MAX_NOTES_BYTES)?;
        }
        if let Some(value) = &self.url {
            url(value, true)?;
        }
        if let Some(value) = self.availability {
            availability(value)?;
        }
        if self.recurrence.is_some() && self.remove_recurrence == Some(true) {
            return refuse(CalendarInvalidChange::problem(
                "recurrence",
                CalendarChangeProblem::InvalidValue,
            ));
        }
        if let Some(rule) = &self.recurrence {
            rule.validate(self.start_ms)?;
        }
        if (self.recurrence.is_some() || self.remove_recurrence == Some(true))
            && span == CalendarSpan::This
        {
            return refuse(CalendarInvalidChange::problem(
                "recurrence",
                CalendarChangeProblem::RecurrenceNeedsFutureSpan,
            ));
        }
        if let Some(values) = &self.alarms_minutes {
            alarms(values)?;
        }
        Ok(())
    }

    fn without_removal(&self) -> Self {
        Self {
            remove_recurrence: None,
            ..self.clone()
        }
    }
}

fn target(calendar_item_id: &str, occurrence_start_ms: i64) -> Checked {
    id("calendar_item_id", calendar_item_id)?;
    check_date("occurrence_start_ms", occurrence_start_ms)
}

impl CalendarEventChange {
    pub fn validate(&self) -> Checked {
        target(&self.calendar_item_id, self.occurrence_start_ms)?;
        self.changes.validate(self.span)
    }
}

impl CalendarSave {
    pub fn validate(&self) -> Checked {
        match self {
            Self::Create(draft) => draft.validate(),
            Self::Change(change) => change.validate(),
        }
    }
}

impl CalendarDelete {
    pub fn validate(&self) -> Checked {
        target(&self.calendar_item_id, self.occurrence_start_ms)
    }
}

/// The identifier a client gives its write, to find its outcome later.
pub fn validate_write_request_id(value: &str) -> Checked {
    id("request_id", value)
}

impl CalendarSaveResult {
    /// Structure only: the occurrence on its own terms (no window: a saved
    /// event may lie anywhere).
    pub fn validate(&self) -> Result<(), CalendarError> {
        if !crate::wire::canonical_uuid(&self.write_id)
            || !crate::wire::canonical_uuid(&self.native_instance_id)
            || self.system_timezone.is_empty()
            || self.system_timezone.len() > MAX_TIMEZONE_BYTES
        {
            return Err(CalendarError::InvalidValue);
        }
        self.occurrence.validate_alone(&self.system_timezone)?;
        super::wire::size(self)
    }
}

impl CalendarDeleteResult {
    pub fn validate(&self) -> Result<(), CalendarError> {
        if !crate::wire::canonical_uuid(&self.write_id)
            || !crate::wire::canonical_uuid(&self.native_instance_id)
            || !identifier(&self.calendar_item_id)
            || !crate::wire::unix_ms(self.occurrence_start_ms)
        {
            return Err(CalendarError::InvalidValue);
        }
        Ok(())
    }
}

/// Facts the daemon holds about the write it sent, to admit its answer.
pub struct CalendarWriteFacts<'a> {
    pub native_instance_id: &'a str,
    pub privacy_generation: u64,
}

impl CalendarSaveResult {
    pub fn validate_response(&self, facts: &CalendarWriteFacts<'_>) -> Result<(), CalendarError> {
        self.validate()?;
        if self.native_instance_id != facts.native_instance_id
            || self.privacy_generation != facts.privacy_generation
        {
            return Err(CalendarError::ScopeChanged);
        }
        Ok(())
    }
}

impl CalendarDeleteResult {
    pub fn validate_response(
        &self,
        request: &CalendarDelete,
        facts: &CalendarWriteFacts<'_>,
    ) -> Result<(), CalendarError> {
        self.validate()?;
        if self.native_instance_id != facts.native_instance_id
            || self.privacy_generation != facts.privacy_generation
            || self.calendar_item_id != request.calendar_item_id
            || self.occurrence_start_ms != request.occurrence_start_ms
            || self.span != request.span
        {
            return Err(CalendarError::ScopeChanged);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
