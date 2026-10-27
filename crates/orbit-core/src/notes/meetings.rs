//! The hub's meetings, transcripts and notes as the Mac reads them.
//! What the hub answers is read tolerantly (no `deny_unknown_fields`): the hub
//! is published before the Mac, so a field it adds must not make an older Mac
//! refuse its meetings. Fields marked «Mac» are added by the daemon, never by
//! the hub.
use super::{EventAttendee, MeetingDictionary, Section};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ts",
    ts(export_to = "../../apps/desktop/src/bridge/generated/")
)]
pub enum Capture {
    Open,
    Closed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TranscriptState {
    None,
    Pending,
    Ready,
    Failed,
}

/// Where the hub's note stands. A state this build does not know is
/// `Unknown`, never one of the others.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NoteState {
    /// The hub has no processing model: nothing the user or the Mac can repair.
    NotConfigured,
    WaitingTranscript,
    Queued,
    Generating,
    Ready,
    Failed,
    #[serde(other)]
    Unknown,
}

/// The line of a meeting's note in the list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NoteHeadline {
    pub state: NoteState,
    #[serde(default)]
    pub short_summary: Option<String>,
}

/// «Mac»: a recording of this Mac the hub may not have whole yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LocalPhase {
    /// Being recorded now.
    Recording,
    /// Recorded; its chunks or its closure are still on their way.
    Uploading,
}

/// «Mac»: what this Mac knows of a recording the hub does not have whole.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalRecording {
    pub phase: LocalPhase,
    /// Chunks recorded and not yet uploaded; `None` when not counted.
    #[serde(default)]
    pub pending_chunks: Option<u64>,
}

/// The original recording Mac, retained even after it is revoked.
/// Absent from older hubs and when only one Mac has ever been paired.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ts",
    ts(export_to = "../../apps/desktop/src/bridge/generated/")
)]
pub struct RecordedOn {
    pub device_id: String,
    pub name: String,
}

/// One meeting of `GET /v1/recordings`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MeetingRow {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recorded_on: Option<RecordedOn>,
    pub recording_id: String,
    pub title: String,
    pub started_at_ms: i64,
    pub duration_ms: i64,
    pub capture: Capture,
    pub transcript: TranscriptState,
    #[serde(default)]
    pub note: Option<NoteHeadline>,
    /// «Mac».
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local: Option<LocalRecording>,
}

/// One item of the notes list read (`ReadId::NotesPage`): a meeting, or why
/// the hub's part of the list could not be read (the Mac's own recordings
/// are still listed).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ListItem {
    HubRefused {
        hub_refusal: crate::refusal::Refusal,
    },
    Meeting(MeetingRow),
}

/// `GET /v1/recordings?before_ms=&limit=`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MeetingList {
    pub meetings: Vec<MeetingRow>,
    pub more: bool,
}

/// One speaker of a meeting: its key (the recogniser's label, else the
/// track), its track and the name the hub resolved.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Speaker {
    pub key: String,
    #[serde(default)]
    pub track: Option<String>,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FailedGroup {
    #[serde(default)]
    pub track: Option<String>,
    #[serde(default)]
    pub cause: Option<String>,
    #[serde(default)]
    pub attempts: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TranscriptProgress {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub final_pass: Option<FinalPass>,
    #[serde(default)]
    pub unmatched_speaker_names: u64,
    pub state: TranscriptState,
    #[serde(default)]
    pub pending_chunks: u64,
    #[serde(default)]
    pub failed_groups: Vec<FailedGroup>,
    /// A change of language is repeating the meeting.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retranscribing: Option<Retranscribing>,
    /// Audio the hub will never get (chunks it refused, declared at the
    /// closure): where and how long each hole is. Empty from a hub up to
    /// 0.10.1, which does not say.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub missing_audio: Vec<MissingAudio>,
    /// Where the person muted the microphone («Silenciar»), from the
    /// meeting's start, as the closure declared it. Empty from a hub that
    /// does not say.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub muted_microphone: Vec<MutedMicrophone>,
}

/// One stretch of a muted microphone, in milliseconds from the meeting's start.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MutedMicrophone {
    pub start_ms: u64,
    pub end_ms: u64,
}

/// One hole of a track: `at_ms` from the meeting's start and `duration_ms`
/// are unknown (`None`) when the chunks around it do not tell.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MissingAudio {
    pub track: String,
    #[serde(default)]
    pub at_ms: Option<u64>,
    #[serde(default)]
    pub duration_ms: Option<u64>,
    #[serde(default)]
    pub chunks: u64,
}

/// The hub's durable final pass, separate from draft group progress.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FinalPass {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<crate::transcription_config::TranscriptionModel>,
    pub state: FinalPassState,
    pub error: Option<HubCause>,
    /// How far a running pass got. `None`: not running, not measured yet,
    /// or a hub up to 0.10.1, which does not say.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress: Option<FinalPassProgress>,
    /// The live draft's spans a completed pass kept, where the new text lost
    /// them or looped. `None`: the new text throughout, as from every hub up
    /// to 0.11.0, which never mixed the two.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kept_draft: Option<KeptDraft>,
}

/// How many draft spans a final pass kept, and their summed length.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeptDraft {
    pub spans: u64,
    pub duration_ms: u64,
}

/// `done_ms` of the `total_ms` of audio transcribed again; `remaining_ms`
/// is the hub's estimate once a window is done (unknown before).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FinalPassProgress {
    pub done_ms: u64,
    pub total_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remaining_ms: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ts",
    ts(export_to = "../../apps/desktop/src/bridge/generated/")
)]
pub enum FinalPassState {
    Queued,
    Running,
    Completed,
    Failed,
}

/// How far a change of language got: `done` of the `total` groups it sent
/// to be transcribed again, in `language`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Retranscribing {
    pub language: String,
    pub done: u64,
    pub total: u64,
}

/// The hub's cause, with whatever numbers it carried.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HubCause {
    pub code: String,
    #[serde(flatten)]
    pub details: std::collections::BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionItem {
    pub id: String,
    #[serde(default)]
    pub owner: Option<String>,
    pub text: String,
    #[serde(default)]
    pub due: Option<String>,
    #[serde(default)]
    pub done: bool,
}

/// The note as the user sees it: what the model wrote with their edits applied.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeenNote {
    #[serde(default)]
    pub short_summary: String,
    #[serde(default)]
    pub sections: Vec<Section>,
    #[serde(default)]
    pub action_items: Vec<ActionItem>,
    #[serde(default)]
    pub decisions: Vec<String>,
    #[serde(default)]
    pub open_topics: Vec<String>,
    #[serde(default)]
    pub template_id: String,
    #[serde(default)]
    pub template_name: String,
    #[serde(default)]
    pub template_source: Option<String>,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub generated_at_ms: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Edited {
    #[serde(default)]
    pub sections: bool,
}

/// The note of `GET /v1/recordings/{id}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NoteDocument {
    pub state: NoteState,
    pub revision: u64,
    #[serde(default)]
    pub attempts: u64,
    #[serde(default)]
    pub next_attempt_ms: Option<i64>,
    #[serde(default)]
    pub error: Option<HubCause>,
    #[serde(default)]
    pub template_id: Option<String>,
    #[serde(default)]
    pub note: Option<SeenNote>,
    #[serde(default)]
    pub edited: Edited,
    #[serde(default)]
    pub superseded_available: bool,
    #[serde(default)]
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrivateNotes {
    pub text: String,
    pub revision: u64,
    /// «Mac»: the text is this Mac's, still on its way to the hub.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub pending: bool,
}

/// A rename applies to the voice, including future turns, or to the current
/// turn identified by its start and the command's speaker key, never its index.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ts",
    ts(export_to = "../../apps/desktop/src/bridge/generated/")
)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum SpeakerRenameScope {
    #[default]
    Voice,
    Turn {
        #[cfg_attr(feature = "ts", ts(type = "number"))]
        at_ms: u64,
    },
}

/// The calendar event the recording was started from, read from the hub
/// or supplied locally while this Mac is still uploading its manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MeetingEvent {
    #[serde(default)]
    pub attendees: Vec<EventAttendee>,
    pub title: String,
    pub starts_at_ms: i64,
    pub ends_at_ms: i64,
    /// The series key the note's template can be assigned to.
    pub calendar_item_id: String,
}

/// `GET /v1/recordings/{id}`, with what this Mac adds to it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MeetingDocument {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recorded_on: Option<RecordedOn>,
    #[serde(default)]
    pub dictionary: Option<MeetingDictionary>,
    pub recording_id: String,
    pub title: String,
    pub started_at_ms: i64,
    pub duration_ms: i64,
    pub capture: Capture,
    /// `None`: nothing transcribed yet.
    #[serde(default)]
    pub language: Option<String>,
    #[serde(default)]
    pub speakers: Vec<Speaker>,
    pub transcript: TranscriptProgress,
    #[serde(default)]
    pub note: Option<NoteDocument>,
    #[serde(default)]
    pub private_notes: PrivateNotes,
    /// «Mac».
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local: Option<LocalRecording>,
    /// The shared calendar context, or this Mac's pending context.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event: Option<MeetingEvent>,
    /// «Mac»: the hub could not be read and this document is what the Mac
    /// knows of a recording of its own, with the hub's refusal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hub_refusal: Option<crate::refusal::Refusal>,
    /// «Mac»: what this Mac's orders still change in the document before the
    /// hub took them (`note_outbox::pending_change`), laid over it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pending_changes: Vec<String>,
}

/// `GET /v1/series/previous-meeting`: the latest closed meeting of a
/// calendar series that started before `before_ms`. `previous: None` is the
/// hub saying the series has none — a fact, not an unknown.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeriesPrevious {
    pub calendar_item_id: String,
    pub before_ms: i64,
    pub previous: Option<PreviousMeeting>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreviousMeeting {
    pub recording_id: String,
    pub title: String,
    pub started_at_ms: i64,
    /// `None`: nobody asked for its note.
    #[serde(default)]
    pub note: Option<PreviousNote>,
}

/// The earlier meeting's note: where it stands and, once written, its tasks
/// as its Actions tab shows them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreviousNote {
    pub state: NoteState,
    /// The revision an edit of the note (a task marked done) is made on.
    pub revision: u64,
    #[serde(default)]
    pub short_summary: Option<String>,
    /// `None`: the note is not written, so its tasks are not known yet.
    #[serde(default)]
    pub action_items: Option<Vec<ActionItem>>,
}

/// One turn of `GET /v1/recordings/{id}/turns`: its time from the start of
/// the recording, who speaks (a key of `speakers`) and what was said.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Turn {
    pub at_ms: u64,
    pub speaker_key: String,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnsPage {
    pub turns: Vec<Turn>,
    #[serde(default)]
    pub next_offset: Option<u64>,
    pub total: u64,
}
