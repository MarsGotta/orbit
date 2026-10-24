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

//!
//! The pieces: this file holds what a write asks and what the helper answers;
//! `refusal` is why a write was not applied, and `checks` what a write and its
//! answer must be.

use serde::{Deserialize, Serialize};

use super::*;

mod checks;
mod refusal;

pub use checks::{CalendarWriteFacts, validate_write_request_id};
pub use refusal::{
    CalendarChangeProblem, CalendarInvalidChange, CalendarNotEditable, CalendarWriteRefusal,
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

#[cfg(test)]
mod tests;
