use serde::{Deserialize, Serialize};

use super::{CalendarAvailability, CalendarRecurrence};

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CalendarAuthorization {
    NotDetermined,
    Restricted,
    Denied,
    FullAccess,
    WriteOnly,
    Unavailable,
    Unknown,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum CalendarFailure {
    #[serde(rename = "calendar_permission_required")]
    PermissionRequired,
    #[serde(rename = "invalid_calendar_request")]
    InvalidRequest,
    #[serde(rename = "calendar_busy")]
    Busy,
    #[serde(rename = "calendar_read_expired")]
    Expired,
    #[serde(rename = "calendar_invalidated")]
    Invalidated,
    #[serde(rename = "calendar_selection_unavailable")]
    SelectionUnavailable,
    #[serde(rename = "calendar_limit_exceeded")]
    LimitExceeded,
    #[serde(rename = "calendar_read_failed")]
    ReadFailed,
    #[serde(rename = "calendar_clock_unavailable")]
    ClockUnavailable,
    #[serde(rename = "privacy_stopped")]
    PrivacyStopped,
    /// REF-10: one event whose own fields do not fit the largest page the
    /// helper may send (`MAX_NATIVE_PAGE_BYTES`), in practice thousands of
    /// invitees. Named, never trimmed; the helper reports it in its status too.
    #[serde(rename = "calendar_event_too_large")]
    EventTooLarge,
}

/// What the helper says about the calendar without reading it: the macOS
/// permission and whether an EventKit call is still running. REF-10 removed the
/// selection (`connected`, `scope`) it used to carry: nothing is selected.
///
/// P9-21 removed `read_only`: the helper now writes events too, and what it
/// can do is the `calendar` capability in `ready`, not a field here. A helper
/// from before still sends `read_only:true`; it is read and dropped, never
/// sent again.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct CalendarStatus {
    pub authorization: CalendarAuthorization,
    pub physical_read_pending: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure: Option<CalendarFailure>,
}

impl<'de> Deserialize<'de> for CalendarStatus {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Received {
            authorization: CalendarAuthorization,
            physical_read_pending: bool,
            #[serde(default, deserialize_with = "super::wire::present")]
            failure: Option<CalendarFailure>,
            // Before P9-21 every helper said `read_only:true`. Accepted so an
            // older status still reads; it means nothing now.
            #[serde(
                default,
                rename = "read_only",
                deserialize_with = "super::wire::present"
            )]
            _read_only: Option<bool>,
        }
        let received = Received::deserialize(deserializer)?;
        Ok(Self {
            authorization: received.authorization,
            physical_read_pending: received.physical_read_pending,
            failure: received.failure,
        })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CalendarReference {
    pub calendar_id: String,
    pub source_id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CalendarEntry {
    pub reference: CalendarReference,
    pub title: String,
    pub source_title: String,
    pub calendar_type: String,
    /// The calendar's own color in macOS, `#RRGGBB` (P9-21). Absent when the
    /// helper predates it or macOS gave a color it could not express in sRGB:
    /// unknown, never a default.
    #[serde(
        default,
        deserialize_with = "super::wire::present",
        skip_serializing_if = "Option::is_none"
    )]
    pub color: Option<String>,
    /// Whether macOS lets events in this calendar be created, changed or
    /// deleted (`allowsContentModifications`). Absent: not known.
    #[serde(
        default,
        deserialize_with = "super::wire::present",
        skip_serializing_if = "Option::is_none"
    )]
    pub can_edit: Option<bool>,
    /// Whether this is the calendar macOS puts new events in. Absent: not known.
    #[serde(
        default,
        deserialize_with = "super::wire::present",
        skip_serializing_if = "Option::is_none"
    )]
    pub is_default: Option<bool>,
}

/// One page of the list of every calendar on the Mac, as the helper sends it.
///
/// REF-10: the list no longer offers anything to choose from — every calendar
/// the permission covers is read. It exists so the app can say how many
/// calendars there are and name the one an event belongs to. It is paged for
/// the same reason a read is: a Mac with a great many calendars must not need
/// one oversized message, and no count of calendars is refused.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CalendarInventoryPage {
    pub inventory_id: String,
    pub native_instance_id: String,
    pub privacy_generation: u64,
    pub clock_domain: String,
    pub observed_monotonic_ns: u64,
    pub completed_monotonic_ns: u64,
    pub calendars: Vec<CalendarEntry>,
    pub after_index: u32,
    pub total: u32,
    #[serde(
        default,
        deserialize_with = "super::wire::present",
        skip_serializing_if = "Option::is_none"
    )]
    pub next_after_index: Option<u32>,
}

/// The half-open window `[start_unix_ms, end_unix_ms)` a read covers.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ts",
    ts(export_to = "../../apps/desktop/src/bridge/generated/")
)]
pub struct CalendarWindow {
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub start_unix_ms: i64,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub end_unix_ms: i64,
}

/// What the daemon asks the helper for: every calendar, one window, from one
/// position in its ordered events. `after_index` is 0 for the first page.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CalendarReadQuery {
    pub start_unix_ms: i64,
    pub end_unix_ms: i64,
    pub after_index: u32,
}

/// One page of one window of every calendar, as the helper sends it.
///
/// The helper orders the window's events the same way on every read (start,
/// end, calendar, item, occurrence) and fills a page with whole events up to
/// `NATIVE_PAGE_EVENT_BYTES`, always at least one. `total` counts every event
/// of the window, `next_after_index` is where the next page begins and is
/// absent exactly on the last one. `omitted` counts, for the whole window, the
/// occurrences EventKit returned that are not events Orbit can describe (no
/// title, outside the window) — never events left out for size or count.
/// `oversized` names, for the whole window, each event that could not travel
/// (decision 4): absent when there is none, so the page before it is unchanged.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CalendarReadPage {
    pub read_id: String,
    pub native_instance_id: String,
    pub privacy_generation: u64,
    pub window: CalendarWindow,
    pub clock_domain: String,
    pub capture_started_monotonic_ns: u64,
    pub capture_completed_monotonic_ns: u64,
    pub observed_at_unix_ms: i64,
    pub system_timezone: String,
    pub provider_sync_status: String,
    pub enumeration: String,
    pub events: Vec<CalendarOccurrence>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub omitted: Vec<CalendarOmission>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub oversized: Vec<CalendarOversizedEvent>,
    pub after_index: u32,
    pub total: u32,
    #[serde(
        default,
        deserialize_with = "super::wire::present",
        skip_serializing_if = "Option::is_none"
    )]
    pub next_after_index: Option<u32>,
}

/// The calendars a read covered and what it read of each event.
///
/// Before REF-10 this was a person's selection, confirmed by the helper. Now
/// nothing is selected: the daemon builds it for the read it assembled, listing
/// the calendars the read's events came from, with invitees and event URL
/// always read, because the macOS permission is the only consent. It is kept as
/// a type because a meeting association records it as provenance.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CalendarScope {
    pub scope_id: String,
    pub native_instance_id: String,
    pub privacy_generation: u64,
    pub calendars: Vec<CalendarReference>,
    pub include_invitees: bool,
    pub include_event_url: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CalendarQuery {
    pub scope_id: String,
    pub start_unix_ms: i64,
    pub end_unix_ms: i64,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CalendarFieldStatus {
    NotRequested,
    NotProvided,
    Provided,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CalendarInvitee {
    #[serde(
        default,
        deserialize_with = "super::wire::present",
        skip_serializing_if = "Option::is_none"
    )]
    pub name: Option<String>,
    pub participant_url: String,
    #[serde(
        default,
        deserialize_with = "super::wire::present",
        skip_serializing_if = "Option::is_none"
    )]
    pub email: Option<String>,
    pub role: String,
    pub response_status: String,
    pub participant_type: String,
    pub is_current_user: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CalendarOccurrence {
    pub observation_id: String,
    pub calendar: CalendarReference,
    pub calendar_item_id: String,
    #[serde(
        default,
        deserialize_with = "super::wire::present",
        skip_serializing_if = "Option::is_none"
    )]
    pub event_id: Option<String>,
    #[serde(
        default,
        deserialize_with = "super::wire::present",
        skip_serializing_if = "Option::is_none"
    )]
    pub original_occurrence_unix_ms: Option<i64>,
    pub start_unix_ms: i64,
    pub end_unix_ms: i64,
    #[serde(
        default,
        deserialize_with = "super::wire::present",
        skip_serializing_if = "Option::is_none"
    )]
    pub event_timezone: Option<String>,
    pub interpreted_timezone: String,
    pub is_floating: bool,
    pub is_all_day: bool,
    pub is_detached: bool,
    pub title: String,
    pub status: String,
    #[serde(
        default,
        deserialize_with = "super::wire::present",
        skip_serializing_if = "Option::is_none"
    )]
    pub last_modified_unix_ms: Option<i64>,
    #[serde(
        default,
        deserialize_with = "super::wire::present",
        skip_serializing_if = "Option::is_none"
    )]
    pub invitees: Option<Vec<CalendarInvitee>>,
    pub invitees_status: CalendarFieldStatus,
    #[serde(
        default,
        deserialize_with = "super::wire::present",
        skip_serializing_if = "Option::is_none"
    )]
    pub event_url: Option<String>,
    pub event_url_status: CalendarFieldStatus,
    /// P9-21. The fields below are absent from a helper that predates them;
    /// absent means not read, never empty or «no».
    #[serde(
        default,
        deserialize_with = "super::wire::present",
        skip_serializing_if = "Option::is_none"
    )]
    pub location: Option<String>,
    /// The event's notes, up to `MAX_NOTES_BYTES`.
    #[serde(
        default,
        deserialize_with = "super::wire::present",
        skip_serializing_if = "Option::is_none"
    )]
    pub notes: Option<String>,
    /// Notes longer than `MAX_NOTES_BYTES` do not travel; this says so, with
    /// their bytes and the limit. The event itself arrives.
    #[serde(
        default,
        deserialize_with = "super::wire::present",
        skip_serializing_if = "Option::is_none"
    )]
    pub notes_left_out: Option<CalendarFieldLeftOut>,
    /// Who organized the event, as EventKit names any participant.
    #[serde(
        default,
        deserialize_with = "super::wire::present",
        skip_serializing_if = "Option::is_none"
    )]
    pub organizer: Option<CalendarInvitee>,
    #[serde(
        default,
        deserialize_with = "super::wire::present",
        skip_serializing_if = "Option::is_none"
    )]
    pub availability: Option<CalendarAvailability>,
    /// The event's first repetition rule; absent when it does not repeat.
    #[serde(
        default,
        deserialize_with = "super::wire::present",
        skip_serializing_if = "Option::is_none"
    )]
    pub recurrence: Option<CalendarRecurrence>,
    /// Whether Orbit may change or delete this event: its calendar admits
    /// changes **and** it was not organized by someone else (an invitation is
    /// moved by whoever sent it, as in Calendar).
    #[serde(
        default,
        deserialize_with = "super::wire::present",
        skip_serializing_if = "Option::is_none"
    )]
    pub can_edit: Option<bool>,
    /// P9-22: the event's reminders Orbit can edit — alerts relative to the
    /// start, at it or before it — as minutes before the start, at most
    /// `MAX_EVENT_ALARMS`, each at most `MAX_ALARM_MINUTES`. Absent: not read
    /// (a helper that predates it, or more of them than fit), never «none».
    #[serde(
        default,
        deserialize_with = "super::wire::present",
        skip_serializing_if = "Option::is_none"
    )]
    pub alarms_minutes: Option<Vec<u64>>,
    /// P9-22: how many reminders Orbit does not edit — at a date, on
    /// arriving or leaving a place, after the start, or past the limits
    /// above. A write of `alarms_minutes` keeps them. Absent: none.
    #[serde(
        default,
        deserialize_with = "super::wire::present",
        skip_serializing_if = "Option::is_none"
    )]
    pub alarms_not_editable: Option<u32>,
}

/// A field that did not travel because it is longer than its limit: how
/// long it is and how long it may be, both in UTF-8 bytes.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CalendarFieldLeftOut {
    pub bytes: u64,
    pub limit: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CalendarOmission {
    pub reason: String,
    pub count: u32,
}

/// One event that could not travel, named instead of trimmed (REF-10,
/// decision 4). The read it belonged to arrives whole without it; this says,
/// by itself, which event it was — its dates and identifier — and why: the
/// field that is too large, its bytes and the bytes that fit.
///
/// `field` is one of `title`, `event_url`, `event_timezone`,
/// `interpreted_timezone`, `calendar_item_id`, `event_id`, `invitee_name`,
/// `invitee_url`, `invitees` (the guest list as a whole) or `event` (the whole
/// event, when no single field is to blame). `calendar_item_id` is absent only
/// when it is itself the field that is too large.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CalendarOversizedEvent {
    pub calendar: CalendarReference,
    #[serde(
        default,
        deserialize_with = "super::wire::present",
        skip_serializing_if = "Option::is_none"
    )]
    pub calendar_item_id: Option<String>,
    pub start_unix_ms: i64,
    pub end_unix_ms: i64,
    pub field: String,
    pub bytes: u64,
    pub limit: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CalendarReadFailure {
    pub calendar: CalendarReference,
    pub reason: CalendarFailure,
}

/// One local read of one window, joined from the helper's pages by the daemon.
///
/// It is what the app's agenda shows and what a meeting association extracts
/// from. It is never sent to Deimos as it stands and never stored whole.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CalendarSnapshot {
    pub snapshot_id: String,
    pub scope: CalendarScope,
    pub window: CalendarQuery,
    pub clock_domain: String,
    pub capture_started_monotonic_ns: u64,
    pub capture_completed_monotonic_ns: u64,
    pub observed_at_unix_ms: i64,
    pub system_timezone: String,
    pub provider_sync_status: String,
    pub enumeration: String,
    pub events: Vec<CalendarOccurrence>,
    /// Pre-omission helpers failed the whole read instead of returning a partial
    /// snapshot, so their absent field is equivalent to an empty list.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub omitted: Vec<CalendarOmission>,
    /// Calendars unavailable before enumeration. Empty keeps the previous wire
    /// representation unchanged; failures during enumeration still invalidate it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unreadable_calendars: Vec<CalendarReadFailure>,
}
