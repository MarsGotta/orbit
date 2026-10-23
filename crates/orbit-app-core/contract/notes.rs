//! The notes as the screen paints them. The meetings, their turns,
//! speakers and notes are the hub's; app-core only puts them in the screen's
//! words (`projection::note`, `projection::notes`).
use super::*;

app_type! {
    #[serde(rename_all = "snake_case")]
    pub enum NoteListState {
        /// The meeting is being recorded on this Mac now.
        Recording,
        /// Recorded; its audio is still on its way to the hub.
        Uploading,
        Transcribing,
        Ready,
        Failed,
    }
}

app_type! {
    #[serde(rename_all = "snake_case")]
    pub enum NoteDayGroup {
        Today,
        Yesterday,
        ThisWeek,
        /// Seven to twenty-nine days back.
        #[serde(rename = "last_30_days")]
        Last30Days,
        Before,
    }
}

app_type! {
    pub struct NoteListItem {
        pub recording_id: String,
        /// The meeting's title (decided 2026-10-17): the same in the list and
        /// in the open note.
        pub title: String,
        #[ts(type = "number")]
        pub started_at_ms: i64,
        #[ts(type = "number")]
        pub duration_ms: i64,
        pub state: NoteListState,
        pub group: NoteDayGroup,
        /// Where its note stands, or how its short summary begins.
        /// Absent when the hub has no note for it.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        pub headline: Option<NoteHeadlineSnapshot>,
    }
}

app_type! {
    /// One line of the notes list. `transcribing` and `writing` are
    /// in progress; `not_configured` waits for a model on the hub.
    #[serde(tag = "phase", rename_all = "snake_case")]
    pub enum NoteHeadlineSnapshot {
        Transcribing {},
        Writing {},
        Ready { preview: String },
        Failed {},
        NotConfigured {},
    }
}

app_type! {
    /// The calendar event the recording was started from.
    pub struct NoteEventSnapshot {
        pub attendees: Vec<orbit_core::notes::EventAttendee>,
        pub title: String,
        #[ts(type = "number")]
        pub start_ms: i64,
        #[ts(type = "number")]
        pub end_ms: i64,
    }
}

app_type! {
    /// One task of the note the hub wrote. `id` is stable across
    /// regenerations that keep the task, so a done mark follows it.
    pub struct NoteActionSnapshot {
        pub id: String,
        pub owner: Option<String>,
        pub text: String,
        pub due: Option<String>,
        pub done: bool,
    }
}

app_type! {
    pub struct NoteSectionSnapshot {
        pub name: String,
        pub markdown: String,
        /// The text comes from the user, not from the model.
        pub edited: bool,
    }
}

app_type! {
    /// The note the hub wrote, with the user's edits applied.
    pub struct NoteSummarySnapshot {
        pub short_summary: String,
        pub sections: Vec<NoteSectionSnapshot>,
        pub template_id: String,
        pub template_name: String,
        #[ts(type = "number")]
        pub generated_at_ms: i64,
        pub model: String,
        /// A regeneration set the user's earlier text aside; it can come back.
        pub superseded_available: bool,
        /// How the template was chosen (`requested`, `series` or
        /// `default`); absent in notes written before.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        pub template_source: Option<String>,
    }
}

app_type! {
    /// Whether a list the hub keeps is known: never read yet, read, not
    /// readable without the hub (not paired, offline or privacy stop), or its
    /// read failed. An empty list is only ever `available`.
    #[serde(rename_all = "snake_case")]
    pub enum NoteTemplatesState {
        Unknown,
        Available,
        NeedsHub,
        Failed,
    }
}

app_type! {
    /// A step in progress (`active`) and a step waiting for something
    /// (`waiting`) never share a name.
    #[serde(rename_all = "snake_case")]
    pub enum NoteStepState {
        Active,
        Waiting,
        Done,
        Failed,
    }
}

app_type! {
    /// Where one step stands, as the screen says it (counts and the
    /// hub's codes stay in the log).
    #[serde(tag = "kind", rename_all = "snake_case")]
    pub enum NoteStepCause {
        /// Being recorded on this Mac.
        Recording {},
        /// Chunks recorded and not uploaded yet; `None` when not counted.
        Uploading {
            #[ts(type = "number | null")]
            chunks: Option<u64>,
        },
        /// Chunks the hub received and has not transcribed yet.
        Transcribing {
            #[ts(type = "number")]
            chunks: u64,
        },
        TranscriptFailed {
            #[ts(type = "number")]
            groups: u64,
        },
        /// The note is written when the recording ends.
        AfterRecording {},
        /// The hub has no note for this recording.
        NotRequested {},
        WaitingTranscript {},
        Writing {},
        /// The hub's attempt failed and it will try again.
        Retrying {},
        /// The hub has no processing model: not the user's fault, nor the Mac's.
        NotConfigured {},
        Failed {},
        /// The hub could not be read.
        HubUnavailable {},
    }
}

app_type! {
    pub struct NoteStepSnapshot {
        pub state: NoteStepState,
        pub cause: Option<NoteStepCause>,
    }
}

app_type! {
    /// Upload → Transcript → Summary, each with its cause.
    pub struct NoteProgressSnapshot {
        pub upload: NoteStepSnapshot,
        pub transcript: NoteStepSnapshot,
        pub summary: NoteStepSnapshot,
    }
}

app_type! {
    /// One turn of the transcript as the hub computed it: its time from the
    /// start of the recording, who speaks (a key of `speakers`) and the text.
    pub struct NoteTurnSnapshot {
        pub index: usize,
        #[ts(type = "number")]
        pub at_ms: u64,
        pub speaker_key: String,
        pub text: String,
    }
}

app_type! {
    pub struct NoteTranscriptSnapshot {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        pub final_pass: Option<NoteFinalPassSnapshot>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional, type = "number")]
        pub unmatched_speaker_names: Option<u64>,
        /// The language the hub transcribed in (`es`, `en`…); `null` while
        /// nothing is transcribed.
        pub language: Option<String>,
        pub turns: Vec<NoteTurnSnapshot>,
        /// Chunks the hub has not transcribed yet.
        #[ts(type = "number")]
        pub pending_chunks: u64,
        /// Groups the hub gave up on: «Reintentar» asks again.
        #[ts(type = "number")]
        pub failed_groups: u64,
        /// The turns could not be read, with the cause (for the log).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        pub error: Option<String>,
        /// A change of language is transcribing the meeting again.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        pub retranscribing: Option<NoteRetranscribingSnapshot>,
        /// Stretches of audio the hub never got (chunks it refused): the
        /// transcript says each one, with its time when known.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        pub missing_audio: Option<Vec<NoteMissingAudioSnapshot>>,
    }
}

app_type! {
    /// One hole of a track: from `at_ms` of the meeting, for `duration_ms`;
    /// either is `null` when the hub could not tell.
    pub struct NoteMissingAudioSnapshot {
        pub track: String,
        #[ts(type = "number | null")]
        pub at_ms: Option<u64>,
        #[ts(type = "number | null")]
        pub duration_ms: Option<u64>,
    }
}

app_type! {
    /// «Transcribiendo de nuevo…»: `done` of the `total` groups the change
    /// of language sent again, in `language`.
    pub struct NoteRetranscribingSnapshot {
        pub language: String,
        #[ts(type = "number")]
        pub done: u64,
        #[ts(type = "number")]
        pub total: u64,
    }
}

app_type! {
    /// One speaker: the hub's key (what renaming names), its track and the
    /// name the hub resolved.
    pub struct NoteSpeakerSnapshot {
        pub key: String,
        pub track: Option<String>,
        pub name: String,
        pub color: String,
        /// Share of the turns, in percent.
        pub share_pct: u8,
    }
}

app_type! {
    pub struct OpenNoteSnapshot {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        pub dictionary: Option<orbit_core::notes::MeetingDictionary>,
        /// Absent in older snapshots: absence does not mean closed.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        pub capture: Option<orbit_core::notes::Capture>,
        pub recording_id: String,
        pub title: String,
        /// How the note names the user: the name of their microphone.
        pub owner_name: String,
        #[ts(type = "number")]
        pub started_at_ms: i64,
        #[ts(type = "number")]
        pub ended_at_ms: i64,
        pub event: Option<NoteEventSnapshot>,
        /// The note the hub wrote; absent until it is written.
        pub summary: Option<NoteSummarySnapshot>,
        /// The hub's revision of the note: an edit is made on it.
        #[ts(type = "number | null")]
        pub note_revision: Option<u64>,
        pub actions: Vec<NoteActionSnapshot>,
        pub decisions: Vec<String>,
        pub open_topics: Vec<String>,
        /// The user's private notes (markdown).
        pub private_notes: String,
        #[ts(type = "number")]
        pub private_notes_revision: u64,
        pub progress: NoteProgressSnapshot,
        pub transcript: NoteTranscriptSnapshot,
        pub speakers: Vec<NoteSpeakerSnapshot>,
        /// The series of the note's event, when it has one.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        pub series: Option<NoteSeriesLinkSnapshot>,
    }
}

app_type! {
    /// `needs_hub`: the hub cannot be asked now (not paired, offline or
    /// privacy stop); the list holds this Mac's own recordings only, and
    /// nothing failed.
    #[serde(rename_all = "snake_case")]
    pub enum NotesIndexState {
        Loading,
        Available,
        NeedsHub,
        Failed,
    }
}

app_type! {
    /// Whether the list was read. `failed` with rows still shows this Mac's
    /// own recordings: only the hub's part is missing.
    pub struct NotesIndexSnapshot {
        pub state: NotesIndexState,
        /// The hub has more meetings than are listed: «Cargar más».
        pub more: bool,
        pub detail: Option<String>,
    }
}

app_type! {
    #[serde(rename_all = "snake_case")]
    pub enum NoteReadState {
        Loading,
        Available,
        Failed,
    }
}

app_type! {
    pub struct NoteReadSnapshot {
        pub recording_id: String,
        /// The note's change counter when it was read (`topics::notes`).
        #[ts(type = "number")]
        pub revision: u64,
        pub state: NoteReadState,
        pub detail: Option<String>,
        #[ts(type = "number | null")]
        pub read_at_ms: Option<i64>,
    }
}

app_type! {
    pub struct NotesSnapshot {
        pub list: Vec<NoteListItem>,
        #[serde(skip)]
        #[ts(skip)]
        pub(crate) projection_clock: Option<Box<(i64, crate::LocalZone)>>,
        /// The last read of the next meeting's series (`client::series`),
        /// which Hoy projects (`briefing.next.previous_actions`).
        #[serde(skip)]
        #[ts(skip)]
        pub(crate) series_previous: Option<Box<SeriesPreviousRead>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        pub index: Option<Box<NotesIndexSnapshot>>,
        pub open: Option<OpenNoteSnapshot>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        pub open_read: Option<NoteReadSnapshot>,
        /// Templates, assignments and refusals.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        pub library: Option<Box<NoteLibrarySnapshot>>,
        /// The last note this app saw being written and then read ready:
        /// «Tu nota está lista · Abrir», apart from the recording's end.
        /// Absent until one is; a note already written when the app
        /// starts never is.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        pub ready: Option<NoteReadySnapshot>,
    }
}

app_type! {
    /// A note whose summary the app saw finish. It stays until another
    /// one finishes: `sequence` moves with each, so the interface says
    /// each once, and a view opened later says nothing for the one it
    /// finds.
    pub struct NoteReadySnapshot {
        pub recording_id: String,
        pub title: String,
        pub sequence: u32,
    }
}

pub use orbit_core::notes::Section;

/// One read of a series' earlier meeting: what it asked and what came back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeriesPreviousRead {
    pub calendar_item_id: String,
    pub before_ms: i64,
    /// The hub's answer, or why it could not be read.
    pub answer: Result<orbit_core::notes::SeriesPrevious, String>,
}

app_type! {
    pub struct NoteFinalPassSnapshot {
        pub state: orbit_core::notes::FinalPassState,
        /// The hub's failure code; diagnostic details stay on the hub.
        pub error: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional, type = "number")]
        pub timeout_ms: Option<u64>,
        /// How far a running pass got, as the hub measures it; absent while
        /// queued, before it measured, or from a hub up to 0.10.1.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        pub progress: Option<NoteFinalPassProgressSnapshot>,
    }
}

app_type! {
    /// `done_ms` of the `total_ms` of audio transcribed again, and the hub's
    /// estimate of the time left once a window is done.
    pub struct NoteFinalPassProgressSnapshot {
        #[ts(type = "number")]
        pub done_ms: u64,
        #[ts(type = "number")]
        pub total_ms: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional, type = "number")]
        pub remaining_ms: Option<u64>,
    }
}
