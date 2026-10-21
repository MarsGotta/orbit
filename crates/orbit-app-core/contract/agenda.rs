use super::*;

app_type! {
    #[serde(rename_all = "snake_case")]
    pub enum CalendarAuthorizationSnapshot {
        Unknown,
        NotDetermined,
        Denied,
        Granted,
    }
}

app_type! {
    /// One calendar on the Mac. REF-10: every one of them is read; none is
    /// selected, so there is no `selected` flag to show.
    pub struct CalendarItemSnapshot {
        pub id: String,
        pub title: String,
        pub source_id: String,
        #[ts(optional)]
        pub source_title: Option<String>,
        /// P9-21: the calendar's own color in macOS, `#RRGGBB`. Only when
        /// macOS gave none (or the helper predates it) is this a palette
        /// token (`accent`, `blue`, `orange`, `green`) picked from the id:
        /// a value that starts with `#` is the real color, one that does not
        /// is a placeholder.
        pub color: String,
        /// Whether events in this calendar can be created, changed or
        /// deleted. Absent: not known yet — never «no».
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        pub can_edit: Option<bool>,
        /// Whether macOS puts new events here. Absent: not known.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        pub is_default: Option<bool>,
    }
}

app_type! {
    pub struct CalendarWindowSnapshot {
        #[ts(type = "number")]
        pub start_ms: i64,
        #[ts(type = "number")]
        pub end_ms: i64,
        pub timezone: String,
    }
}

app_type! {
    /// How someone answered an invitation, read from EventKit's participant
    /// status (P9-20). `None` is EventKit holding no answer at all
    /// (`unknown`, `delegated`, `completed`, `in_process`): it is not a
    /// refusal and the card draws it like an accepted event.
    #[serde(rename_all = "snake_case")]
    #[derive(Copy)]
    pub enum CalendarResponse {
        Accepted,
        Tentative,
        Declined,
        /// Invited and not answered yet (`pending`).
        NeedsAction,
        None,
    }
}

app_type! {
    pub struct CalendarAttendeeSnapshot {
        pub name: String,
        pub email: Option<String>,
        /// P9-20: this guest's answer. Absent when the helper sent no status.
        #[serde(default)]
        #[ts(optional = nullable)]
        pub response: Option<CalendarResponse>,
        /// P9-11: the guest EventKit marks as Mars (`is_current_user`).
        #[serde(default, skip_serializing_if = "is_false")]
        #[ts(as = "Option<bool>", optional)]
        pub is_self: bool,
    }
}

app_type! {
    #[serde(rename_all = "snake_case")]
    pub enum CalendarEventStatus {
        Confirmed,
        Tentative,
        Cancelled,
    }
}

app_type! {
    pub struct CalendarEventSnapshot {
        /// None means the recording topic has not established an answer.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        pub recording: Option<BriefingRecordingState>,
        pub id: String,
        #[ts(optional = nullable)]
        pub calendar_id: Option<String>,
        pub title: String,
        #[ts(type = "number")]
        pub start_ms: i64,
        #[ts(type = "number")]
        pub end_ms: i64,
        #[ts(optional = nullable)]
        pub all_day: Option<bool>,
        #[ts(optional = nullable)]
        pub attendees: Option<Vec<CalendarAttendeeSnapshot>>,
        #[ts(optional = nullable)]
        pub url: Option<String>,
        #[ts(optional = nullable)]
        pub location: Option<String>,
        #[ts(optional = nullable)]
        pub status: Option<CalendarEventStatus>,
        /// P9-20: Mars's own answer, from the guest EventKit marks as her
        /// (`is_current_user`). Absent means the event is not an invitation
        /// to her — no guest list, or she is not on it — never «sin
        /// responder».
        #[serde(default)]
        #[ts(optional = nullable)]
        pub self_response: Option<CalendarResponse>,
        /// P9-20: the event's link when it is a known video call (Google
        /// Meet, Zoom, Microsoft Teams, Webex, Whereby, FaceTime), which the
        /// detail offers as «Unirse». Any other link stays in `url` only.
        #[serde(default)]
        #[ts(optional = nullable)]
        pub conference_url: Option<String>,
        /// P9-21: what names this occurrence for a change or a delete —
        /// its event and the original date of the occurrence. `id` is a new
        /// observation on every read and names nothing.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        pub calendar_item_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional, type = "number")]
        pub occurrence_start_ms: Option<i64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        pub notes: Option<String>,
        /// Notes longer than Orbit carries (`MAX_NOTES_BYTES`): their bytes.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional, type = "number")]
        pub notes_left_out_bytes: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        pub organizer: Option<CalendarAttendeeSnapshot>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        pub availability: Option<CalendarAvailability>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        pub recurrence: Option<CalendarRecurrence>,
        /// Whether Orbit may change or delete it: its calendar admits changes
        /// and it was not organized by someone else. Absent: not known.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        pub can_edit: Option<bool>,
        /// P9-22: its reminders Orbit edits, in minutes before the start.
        /// Absent: not read (a helper that predates it, or too many), never
        /// «none».
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional, type = "Array<number>")]
        pub alarms_minutes: Option<Vec<u64>>,
        /// P9-22: reminders Orbit does not edit (at a date, by place, after
        /// the start); a change of reminders keeps them.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        pub alarms_not_editable: Option<u32>,
    }
}

app_type! {
    /// Which calendar view the range belongs to (P9-20).
    #[serde(rename_all = "snake_case")]
    #[derive(Copy, Default)]
    pub enum CalendarViewKind {
        Day,
        /// A grid of `days` columns (1–14).
        Days,
        #[default]
        Week,
        /// The weeks the month touches, whole.
        Month,
        /// A list of days, 31 either side of the anchor.
        Agenda,
    }
}

app_type! {
    /// The day a week begins on: Mars's local preference (P9-20).
    #[serde(rename_all = "snake_case")]
    #[derive(Copy, Default)]
    pub enum CalendarWeekStart {
        #[default]
        Monday,
        Sunday,
    }
}

app_type! {
    /// The period the calendar shows, decided here from the view, the anchor
    /// and Mars's zone (P9-20). The read behind it may be wider: it also
    /// covers today, so the Briefing never loses its day (`window`).
    #[derive(Default)]
    pub struct CalendarRangeSnapshot {
        pub view: CalendarViewKind,
        /// How many civil days the range covers: 1 for a day, N for N days,
        /// 7 for a week, 35 or 42 for a month, 63 for the agenda.
        pub days: u8,
        pub week_start: CalendarWeekStart,
        /// The local midnight of the day the view was asked to show.
        #[ts(type = "number")]
        pub anchor_ms: i64,
        #[ts(type = "number")]
        pub start_ms: i64,
        #[ts(type = "number")]
        pub end_ms: i64,
    }
}

app_type! {
    /// CAL-09: whether Deimos can read her agenda right now — the question the
    /// Agenda screen headlines. REF-10 retired the stored copy it used to be
    /// confused with: the live read crosses the device bridge and needs the
    /// macOS permission, the pairing and a reachable Mac, and nothing else.
    ///
    /// `Unknown` is not a hedge. It is the permission the helper has not
    /// reported yet: never shown as granted and never as denied (REF-10 A8).
    #[serde(rename_all = "snake_case")]
    pub enum CalendarReadingState {
        Available,
        Unavailable,
        Unknown,
    }
}

app_type! {
    /// Which condition of the live read is missing. Information, never a
    /// control: the macOS permission, local privacy stop, helper, pairing and
    /// Mac connection each name their own cause. Nothing selects calendars.
    #[serde(rename_all = "snake_case")]
    pub enum CalendarReadingBlocker {
        CalendarPermission,
        PrivacyStopped,
        HelperNotRunning,
        NotPaired,
        /// Paired, but the hub cannot reach the Mac. Deliberately distinct from
        /// `NotPaired`: a Mac that is merely asleep is not a Mac she has to
        /// pair again, and telling her so would be the same class of false
        /// statement this spec removes.
        MacDisconnected,
    }
}

app_type! {
    pub struct CalendarReadingSnapshot {
        pub state: CalendarReadingState,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        pub blocker: Option<CalendarReadingBlocker>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        pub detail: Option<String>,
    }
}

app_type! {
    /// The agenda in the app: the visible range, read from every calendar.
    pub struct CalendarSnapshot {
        pub authorization: CalendarAuthorizationSnapshot,
        /// Every calendar on the Mac, from its last listing; empty until the
        /// helper has listed them (Ajustes says «desconocido» then).
        pub calendars: Vec<CalendarItemSnapshot>,
        /// What was read: the visible range, widened to cover today.
        pub window: CalendarWindowSnapshot,
        /// What the calendar shows (P9-20). `events` holds the events of
        /// this range only.
        #[serde(default)]
        pub range: CalendarRangeSnapshot,
        pub events: Vec<CalendarEventSnapshot>,
        /// Can Deimos read her agenda. CAL-09: the screen's headline.
        pub reading: CalendarReadingSnapshot,
        pub stale: bool,
        /// P9-22: the latest writes the agenda head carries, oldest first.
        /// A write missing here says nothing about the calendar.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        #[ts(as = "Option<Vec<WriteOutcome>>", optional)]
        pub writes: Vec<WriteOutcome>,
    }
}

pub use orbit_core::calendar::{
    CalendarAvailability, CalendarEventChanges, CalendarEventDraft, CalendarFrequency,
    CalendarRecurrence, CalendarRecurrenceDay, CalendarRecurrenceEnd, CalendarRecurrenceRule,
    CalendarSpan, CalendarWeekday,
};
pub use orbit_core::calendar_control::{CalendarWriteOperation, CalendarWriteStage};
pub use orbit_core::topics::agenda::{WriteOutcome, WriteTarget};
