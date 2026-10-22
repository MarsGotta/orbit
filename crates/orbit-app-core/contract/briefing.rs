use super::*;

app_type! {
    pub struct BriefingGreetingSnapshot {
        pub name: Option<String>,
    }
}

app_type! {
    pub struct BriefingSummarySnapshot {
        pub meetings_today: usize,
        pub focus_minutes: u16,
    }
}

app_type! {
    pub struct BriefingAttendeeSnapshot {
        pub name: String,
        pub initial: String,
        pub color: String,
    }
}

app_type! {
    pub struct BriefingPreviousSnapshot {
        pub recording_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        pub title: Option<String>,
        pub summary: Option<String>,
        pub open_items: Vec<String>,
    }
}

app_type! {
    /// One task of the series' earlier note, as its Actions tab has it.
    pub struct BriefingActionSnapshot {
        pub id: String,
        pub owner: Option<String>,
        pub text: String,
        pub due: Option<String>,
        pub done: bool,
    }
}

app_type! {
    /// «Pendiente de la última vez»: the tasks of the note of the series'
    /// earlier meeting, read from the hub (`GET /v1/series/previous-meeting`).
    #[serde(tag = "state", rename_all = "snake_case")]
    pub enum BriefingPreviousActions {
        /// Not read yet, or it could not be read, with why: never «nothing».
        Unknown { cause: String },
        /// The series has no earlier meeting: nothing is pending, a fact.
        NoPreviousMeeting,
        /// The earlier meeting has no written note (not asked for, being
        /// written or failed), so its tasks are not known.
        NoteNotWritten {
            recording_id: String,
            /// The hub's state of the note; `None` when nobody asked for it.
            note_state: Option<String>,
        },
        /// The earlier note's tasks, done or not. Marking one is
        /// `note_action_toggle` with this `recording_id`, on `note_revision`.
        Known {
            recording_id: String,
            #[ts(type = "number")]
            note_revision: u64,
            items: Vec<BriefingActionSnapshot>,
        },
    }
}

app_type! {
    #[serde(rename_all = "snake_case")]
    pub enum BriefingRecordingState {
        None,
        Recording,
    }
}

app_type! {
    pub struct BriefingNextSnapshot {
        pub event_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        pub event: Option<MeetingEventReference>,
        pub title: String,
        #[ts(type = "number")]
        pub start_ms: i64,
        #[ts(type = "number")]
        pub end_ms: i64,
        pub attendees: Vec<BriefingAttendeeSnapshot>,
        pub url: Option<String>,
        pub series_key: String,
        pub previous: Option<BriefingPreviousSnapshot>,
        /// Only on the next meeting of a calendar series: absent on the rest
        /// of the day's meetings and on an event without a series.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        pub previous_actions: Option<BriefingPreviousActions>,
        pub recording: BriefingRecordingState,
    }
}

app_type! {
    pub struct BriefingAgendaSnapshot {
        pub event_id: String,
        #[ts(type = "number")]
        pub start_ms: i64,
        pub title: String,
        pub color: String,
        pub people_label: String,
        pub minutes: u16,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        pub people_count: Option<usize>,
    }
}

app_type! {
    #[serde(tag = "state", rename_all = "snake_case")]
    pub enum BriefingSourceState {
        Present,
        Unknown { cause: String },
        Absent { cause: String },
    }
}

app_type! {
    pub struct BriefingSourcesSnapshot {
        pub agenda: BriefingSourceState,
        pub notes: BriefingSourceState,
        pub recording: BriefingSourceState,
    }
}

app_type! {
    pub struct BriefingSnapshot {
        pub greeting: BriefingGreetingSnapshot,
        pub summary: BriefingSummarySnapshot,
        pub next: Option<BriefingNextSnapshot>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        pub upcoming: Option<Vec<BriefingNextSnapshot>>,
        pub agenda: Vec<BriefingAgendaSnapshot>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        pub sources: Option<BriefingSourcesSnapshot>,
    }
}
