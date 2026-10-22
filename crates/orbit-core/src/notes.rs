//! What the Mac still decides about a meeting's note. The hub keeps the
//! meetings, their turns, speakers, notes and templates; the Mac passes them
//! through and only owns three things: the name of the account that labels
//! the user's microphone, the user's private notes until the hub confirms
//! them, and the orders they give about notes and templates.
//!
//! Pure: no clock, network, file or process.
pub mod dictionaries;
pub use dictionaries::*;

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

// Copies of the hub's bounds (`server/orbit_hub/notes/{models,templates,
// custom_templates}.py`), registered in `limits.rs` and compared by
// `scripts/tests/test_constants_copies.py`.

/// Private notes typed during a meeting: about ten thousand words, far
/// beyond what anyone types live. The Mac checks it before keeping them in
/// its outbox: a text the hub would always refuse would otherwise wait there
/// for ever. It protects the hub's request and the outbox record.
pub const MAX_PRIVATE_NOTES_CHARS: usize = 64 * 1024;
/// Sections of a template; past twelve it stops being a summary. The
/// template editor shows it.
pub const MAX_SECTIONS: usize = 12;
/// A section's or a template's name. The template editor shows it.
pub const MAX_NAME_CHARS: usize = 80;
/// A person's name (`meetings/bounds.py`, `MAX_PERSON_NAME_CHARS`): the
/// account name sent as the owner of this Mac and a name the user gives a voice.
/// It protects the hub's stored rows and every note's prompt from a name
/// that is really a paragraph; 128 characters hold any real full name.
pub const MAX_PERSON_NAME_CHARS: usize = 128;
/// What the meeting is and what matters in it: about a page of text.
pub const MAX_CONTEXT_CHARS: usize = 4000;
/// One section's instructions to the model: a long paragraph.
pub const MAX_INSTRUCTIONS_CHARS: usize = 2000;
/// The user's own templates per device: six built-ins plus 32 of theirs is more
/// kinds of meeting than anyone keeps a template for.
pub const MAX_CUSTOM_TEMPLATES: usize = 32;

/// How the note names the user when the account has no full name.
pub const DEFAULT_OWNER_NAME: &str = "Yo";

/// Meetings per page of `GET /v1/recordings` (`views.MAX_LIST_PAGE`): the
/// hub's largest page, so a list of fifty notes takes one request; a larger
/// `limit` is refused. `ReadId::NotesPage` is sized for it.
pub const LIST_PAGE: usize = 50;
/// Bytes of turns per page of `GET /v1/recordings/{id}/turns`, as the hub
/// measures them (`views.MAX_TURN_PAGE_BYTES`): about two hours of meeting,
/// so a long one arrives in a few requests. `ReadId::NoteTranscript` is
/// sized to carry one such page to the app.
pub const TURN_PAGE_BYTES: usize = 256 * 1024;

// The calendar event a closing manifest carries: copies of
// the hub's `EventContext` bounds (`notes/models.py`). The finalize route
// refuses a manifest past them, and a refused closure is a meeting that
// never closes, so the Mac cuts to them; it never refuses the user's
// recording for their calendar's sake.

/// The event's title (`MAX_EVENT_TITLE_CHARS`): context for the prompt, cut
/// past it.
pub const MAX_EVENT_TITLE_CHARS: usize = 512;
/// Invitees (`MAX_ATTENDEES`): past two hundred an event is a broadcast; the
/// note names who spoke from the transcript, and the list only helps put
/// names to voices. The user goes first.
pub const MAX_EVENT_ATTENDEES: usize = 200;
/// An invitee's name (`Attendee.name`, `MAX_NAME_CHARS * 2`), cut past it.
pub const MAX_ATTENDEE_NAME_CHARS: usize = 2 * MAX_NAME_CHARS;
/// An invitee's address (`MAX_EMAIL_CHARS`): RFC 5321's longest address. A
/// longer one is not an address and is left out, never cut.
pub const MAX_ATTENDEE_EMAIL_CHARS: usize = 254;

/// One invitee as the hub's `Attendee` admits it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ts",
    ts(export_to = "../../apps/desktop/src/bridge/generated/")
)]
pub struct EventAttendee {
    pub name: String,
    pub email: String,
    pub is_self: bool,
}

/// The event of a closing manifest, as the hub's `EventContext` admits it
/// (strict: these fields and no others).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventContext {
    pub title: String,
    pub starts_at_ms: Option<i64>,
    pub ends_at_ms: Option<i64>,
    pub attendees: Vec<EventAttendee>,
    /// The series key: the template assigned to it writes the note.
    pub calendar_item_id: String,
}

/// The event a recording was started from, within the hub's bounds above.
pub fn event_context(linked: &crate::call_detection::LinkedEvent) -> EventContext {
    let cut = |text: &str, max: usize| text.chars().take(max).collect::<String>();
    let instant = |value: i64| (value >= 0).then_some(value);
    let mut attendees = linked.attendees.iter().collect::<Vec<_>>();
    // Stable: the user first, everyone else in the calendar's order.
    attendees.sort_by_key(|attendee| !attendee.is_self);
    EventContext {
        title: cut(&linked.title, MAX_EVENT_TITLE_CHARS),
        starts_at_ms: instant(linked.starts_at_ms),
        ends_at_ms: instant(linked.ends_at_ms),
        attendees: attendees
            .into_iter()
            .take(MAX_EVENT_ATTENDEES)
            .map(|attendee| EventAttendee {
                name: cut(&attendee.name, MAX_ATTENDEE_NAME_CHARS),
                email: if attendee.email.chars().count() > MAX_ATTENDEE_EMAIL_CHARS {
                    String::new()
                } else {
                    attendee.email.clone()
                },
                is_self: attendee.is_self,
            })
            .collect(),
        calendar_item_id: linked.calendar_item_id.clone(),
    }
}

/// Why a note order was refused on the Mac, with its numbers.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum NoteCommandError {
    #[error(
        "Las notas privadas tienen {characters} caracteres; el máximo es {max_characters}. No se guardó nada."
    )]
    PrivateNotesTooLong {
        characters: usize,
        max_characters: usize,
    },
    #[error("Identificador de grabación no válido")]
    InvalidRecording,
    #[error(
        "El nombre del hablante tiene {characters} caracteres; debe tener entre 1 y {max_characters}"
    )]
    SpeakerName {
        characters: usize,
        max_characters: usize,
    },
    #[error(
        "El título tiene {characters} caracteres; como mucho {max_characters}. No se cambió el nombre."
    )]
    MeetingTitleTooLong {
        characters: usize,
        max_characters: usize,
    },
    #[error("El título no puede tener saltos de línea ni caracteres de control")]
    MeetingTitleControl,
    #[error("Idioma no válido: se espera un código de dos letras, como «es» o «en»")]
    MeetingLanguage,
}

pub fn check_private_notes(markdown: &str) -> Result<(), NoteCommandError> {
    let characters = markdown.chars().count();
    if characters > MAX_PRIVATE_NOTES_CHARS {
        return Err(NoteCommandError::PrivateNotesTooLong {
            characters,
            max_characters: MAX_PRIVATE_NOTES_CHARS,
        });
    }
    Ok(())
}

/// A name the user gives a speaker: trimmed, 1 to [`MAX_PERSON_NAME_CHARS`] characters,
/// no control characters. `None` gives the speaker back its default name.
pub fn speaker_name(name: Option<&str>) -> Result<Option<String>, NoteCommandError> {
    let Some(name) = name else {
        return Ok(None);
    };
    let name = name.trim();
    let characters = name.chars().count();
    if characters == 0 || characters > MAX_PERSON_NAME_CHARS || name.chars().any(char::is_control) {
        return Err(NoteCommandError::SpeakerName {
            characters,
            max_characters: MAX_PERSON_NAME_CHARS,
        });
    }
    Ok(Some(name.to_owned()))
}

/// A meeting's name as the user types it: trimmed, one line, within the
/// title a meeting already has ([`crate::MAX_MANIFEST_TITLE_CHARACTERS`],
/// the hub's `MAX_RECORDING_TITLE_CHARACTERS`). Empty gives the meeting back
/// its own title.
pub fn meeting_title(title: &str) -> Result<String, NoteCommandError> {
    if title.chars().any(char::is_control) {
        return Err(NoteCommandError::MeetingTitleControl);
    }
    let title = title.trim();
    let characters = title.chars().count();
    if characters > crate::MAX_MANIFEST_TITLE_CHARACTERS {
        return Err(NoteCommandError::MeetingTitleTooLong {
            characters,
            max_characters: crate::MAX_MANIFEST_TITLE_CHARACTERS,
        });
    }
    Ok(title.to_owned())
}

/// A language a meeting can be transcribed in: ISO 639-1, two lowercase
/// letters, as the hub's `[meeting_transcription] language`.
pub fn meeting_language(language: &str) -> Result<(), NoteCommandError> {
    if language.len() == 2 && language.bytes().all(|byte| byte.is_ascii_lowercase()) {
        Ok(())
    } else {
        Err(NoteCommandError::MeetingLanguage)
    }
}

/// How the hub names the user's microphone: the full name of the macOS account
/// (the first field of `pw_gecos`), or «Yo» when there is none or it does
/// not fit the hub's [`MAX_PERSON_NAME_CHARS`].
pub fn owner_name(gecos: Option<&str>) -> String {
    gecos
        .and_then(|gecos| gecos.split(',').next())
        .map(str::trim)
        .filter(|name| {
            !name.is_empty()
                && name.chars().count() <= MAX_PERSON_NAME_CHARS
                && !name.chars().any(char::is_control)
        })
        .map_or_else(|| DEFAULT_OWNER_NAME.to_owned(), str::to_owned)
}

/// One section of the note as the user edits it (`PUT …/note/edits`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ts",
    ts(export_to = "../../apps/desktop/src/bridge/generated/")
)]
pub struct Section {
    pub name: String,
    pub markdown: String,
}

/// One section of a template: its name and what the model writes in it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ts",
    ts(export_to = "../../apps/desktop/src/bridge/generated/")
)]
pub struct TemplateSection {
    pub name: String,
    pub instructions: String,
}

/// `PUT /v1/note-templates/{id}`: create or replace one of the user's templates.
/// The id is chosen on the Mac, so a save sent twice replaces the same one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ts",
    ts(export_to = "../../apps/desktop/src/bridge/generated/")
)]
pub struct TemplateDraft {
    pub name: String,
    pub context: String,
    pub sections: Vec<TemplateSection>,
    /// The template this one is a copy of; the hub keeps the first origin.
    pub forked_from: Option<String>,
}

/// The user's orders about templates and series (`Command::NoteLibrary`). Each
/// goes to the hub as they give it; nothing is queued on the Mac.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ts",
    ts(export_to = "../../apps/desktop/src/bridge/generated/")
)]
pub enum LibraryRequest {
    Dictionary {
        request: DictionaryRequest,
    },
    /// Create or replace one of the user's templates under an id the app chose.
    /// Editing a built-in saves a new id with `forked_from`.
    SaveTemplate {
        template_id: String,
        draft: TemplateDraft,
    },
    DeleteTemplate {
        template_id: String,
    },
    /// «Deshacer» a delete: the template as the user had it, under the
    /// same id, and the series that used it.
    RestoreTemplate {
        /// Chosen by the app, so it knows which undo the library answers.
        request_id: String,
        template_id: String,
        draft: TemplateDraft,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        series: Option<Vec<String>>,
    },
    /// The template that writes the notes of a series; `None` sends it back
    /// to General.
    Assign {
        calendar_item_id: String,
        template_id: Option<String>,
    },
    /// «Usar lo mismo en toda la serie»: the series of this meeting takes
    /// this template and this dictionary (`None`: no dictionary), both or
    /// neither (`PUT /v1/recordings/{id}/series`). `dictionary_ids`, when
    /// given, is the whole list instead, for a hub that takes several.
    UseForSeries {
        recording_id: String,
        template_id: String,
        dictionary_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        dictionary_ids: Option<Vec<String>>,
    },
}

impl LibraryRequest {
    /// What the order is about, so two orders about the same thing reach
    /// the hub in the order the user gave them.
    pub fn target(&self) -> String {
        match self {
            Self::Dictionary { request } => request.target(),
            Self::SaveTemplate { template_id, .. }
            | Self::DeleteTemplate { template_id }
            | Self::RestoreTemplate { template_id, .. } => format!("template:{template_id}"),
            Self::Assign {
                calendar_item_id, ..
            } => format!("series:{calendar_item_id}"),
            Self::UseForSeries { recording_id, .. } => format!("meeting_series:{recording_id}"),
        }
    }
}

// The hub's routes of the device, as the Mac reads them.
// What the hub answers is read tolerantly (no `deny_unknown_fields`): the hub
// is published before the Mac, so a field it adds must not make an older Mac
// refuse its meetings. Fields marked «Mac» are added by the daemon, never by
// the hub.

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

/// One meeting of `GET /v1/recordings`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MeetingRow {
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
}

/// The hub's durable final pass, separate from draft group progress.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FinalPass {
    pub state: FinalPassState,
    pub error: Option<HubCause>,
    /// How far a running pass got. `None`: not running, not measured yet,
    /// or a hub up to 0.10.1, which does not say.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress: Option<FinalPassProgress>,
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

/// «Mac»: the calendar event the recording was started from.
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
    /// «Mac».
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event: Option<MeetingEvent>,
    /// «Mac»: the hub could not be read and this document is what the Mac
    /// knows of a recording of its own, with the hub's refusal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hub_refusal: Option<crate::refusal::Refusal>,
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

/// One of the hub's templates (`GET /v1/note-templates`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Template {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub context: String,
    #[serde(default)]
    pub sections: Vec<TemplateSection>,
    #[serde(default)]
    pub builtin: bool,
    /// A copy says of which template, with its name at the time.
    #[serde(default)]
    pub forked_from: Option<TemplateOrigin>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TemplateOrigin {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TemplateList {
    pub templates: Vec<Template>,
}

/// One series with a template assigned (`GET /v1/note-template-links`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TemplateLink {
    pub calendar_item_id: String,
    pub template_id: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TemplateLinks {
    pub links: Vec<TemplateLink>,
}

/// What the library read carries, one per page item (`ReadId::NoteLibrary`).
/// The templates and assignments are the hub's; the rest is what this Mac
/// knows of the orders the user gave in this session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "item", rename_all = "snake_case")]
pub enum LibraryItem {
    Dictionary(Dictionary),
    DictionaryRefused {
        failure: DictionaryFailure,
    },
    Template(Template),
    Link(TemplateLink),
    /// A list the hub could not give, with its cause.
    Unavailable {
        list: LibraryList,
        refusal: crate::refusal::Refusal,
    },
    /// An order on its way to the hub, by what it is about.
    Pending {
        target: String,
    },
    /// An order the hub refused or could not receive.
    Refused(LibraryRefusal),
    /// An undo («Deshacer») that was answered.
    Restored {
        request_id: String,
        template_id: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LibraryList {
    Dictionaries,
    Templates,
    Links,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LibraryRefusal {
    /// `template` or `series`.
    pub target_kind: String,
    pub target_id: String,
    /// `save`, `delete`, `assign` or `unassign`.
    pub action: String,
    /// The hub's HTTP status; 0 when the order did not reach it.
    pub status: u16,
    /// The hub's code, or `hub_unreachable`.
    pub code: String,
    /// The hub's numbers for its refusal (`links`, `max_links`, `series`,
    /// `max_series`, `bytes`…); empty when it gave none.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub numbers: BTreeMap<String, u64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn owner_name_is_the_account_name_or_yo() {
        assert_eq!(owner_name(Some("Alina Prieto,,,")), "Alina Prieto");
        assert_eq!(owner_name(Some("  ")), DEFAULT_OWNER_NAME);
        assert_eq!(owner_name(None), DEFAULT_OWNER_NAME);
        assert_eq!(owner_name(Some(&"x".repeat(128))), "x".repeat(128));
        assert_eq!(owner_name(Some(&"x".repeat(129))), DEFAULT_OWNER_NAME);
        assert_eq!(owner_name(Some("Ali\u{7}na")), DEFAULT_OWNER_NAME);
    }

    #[test]
    fn an_event_is_cut_to_the_hubs_bounds_with_the_user_first() {
        let attendee = |name: &str, email: &str, is_self: bool| crate::call_detection::Attendee {
            name: name.into(),
            email: email.into(),
            is_self,
        };
        let mut attendees = vec![attendee(&"n".repeat(200), &"e".repeat(255), false)];
        attendees.extend((0..250).map(|_| attendee("Ada", "ada@example.com", false)));
        attendees.push(attendee("Alina", "alina@example.com", true));
        let linked = crate::call_detection::LinkedEvent {
            calendar_item_id: "serie".into(),
            occurrence_start_ms: 10,
            title: "t".repeat(600),
            starts_at_ms: -1,
            ends_at_ms: 20,
            attendees,
            recurring: None,
        };
        let event = event_context(&linked);
        assert_eq!(event.title.chars().count(), MAX_EVENT_TITLE_CHARS);
        assert_eq!((event.starts_at_ms, event.ends_at_ms), (None, Some(20)));
        assert_eq!(event.attendees.len(), MAX_EVENT_ATTENDEES);
        assert!(event.attendees[0].is_self);
        assert_eq!(
            event.attendees[1].name.chars().count(),
            MAX_ATTENDEE_NAME_CHARS
        );
        assert_eq!(event.attendees[1].email, "");
        assert_eq!(event.calendar_item_id, "serie");
    }

    #[test]
    fn private_notes_over_the_hub_bound_are_refused_with_their_numbers() {
        assert!(check_private_notes(&"a".repeat(MAX_PRIVATE_NOTES_CHARS)).is_ok());
        assert_eq!(
            check_private_notes(&"ñ".repeat(MAX_PRIVATE_NOTES_CHARS + 1)),
            Err(NoteCommandError::PrivateNotesTooLong {
                characters: MAX_PRIVATE_NOTES_CHARS + 1,
                max_characters: MAX_PRIVATE_NOTES_CHARS,
            })
        );
    }

    #[test]
    fn a_meeting_title_is_one_trimmed_line_within_a_meetings_title() {
        let max = crate::MAX_MANIFEST_TITLE_CHARACTERS;
        assert_eq!(
            meeting_title("  Presupuesto 2027 "),
            Ok("Presupuesto 2027".into())
        );
        // Empty gives the meeting back its own title.
        assert_eq!(meeting_title("   "), Ok(String::new()));
        assert_eq!(meeting_title(&"ñ".repeat(max)), Ok("ñ".repeat(max)));
        assert_eq!(
            meeting_title(&"ñ".repeat(max + 1)),
            Err(NoteCommandError::MeetingTitleTooLong {
                characters: max + 1,
                max_characters: max,
            })
        );
        assert_eq!(
            meeting_title("uno\ndos"),
            Err(NoteCommandError::MeetingTitleControl)
        );
    }

    #[test]
    fn a_meeting_language_is_two_lowercase_letters() {
        assert_eq!(meeting_language("en"), Ok(()));
        for wrong in ["", "auto", "EN", "e1", "es-ES", "ñe"] {
            assert_eq!(
                meeting_language(wrong),
                Err(NoteCommandError::MeetingLanguage),
                "{wrong}"
            );
        }
    }

    #[test]
    fn a_speaker_name_is_trimmed_bounded_and_null_gives_the_default_back() {
        assert_eq!(speaker_name(Some("  Ada ")), Ok(Some("Ada".into())));
        assert_eq!(speaker_name(None), Ok(None));
        assert_eq!(
            speaker_name(Some(" ")),
            Err(NoteCommandError::SpeakerName {
                characters: 0,
                max_characters: MAX_PERSON_NAME_CHARS
            })
        );
        assert!(speaker_name(Some(&"x".repeat(MAX_PERSON_NAME_CHARS))).is_ok());
        assert!(speaker_name(Some(&"x".repeat(MAX_PERSON_NAME_CHARS + 1))).is_err());
    }

    #[test]
    fn a_running_final_pass_reads_its_progress_and_an_older_hub_none()
    -> Result<(), serde_json::Error> {
        let running: FinalPass = serde_json::from_value(serde_json::json!({
            "state": "running", "error": null,
            "progress": {"done_ms": 30_000, "total_ms": 60_300, "remaining_ms": 9_000}
        }))?;
        assert_eq!(
            running.progress,
            Some(FinalPassProgress {
                done_ms: 30_000,
                total_ms: 60_300,
                remaining_ms: Some(9_000),
            })
        );
        let started: FinalPass = serde_json::from_value(serde_json::json!({
            "state": "running", "error": null, "progress": {"done_ms": 0, "total_ms": 60_000}
        }))?;
        assert_eq!(started.progress.and_then(|p| p.remaining_ms), None);
        let older: FinalPass =
            serde_json::from_value(serde_json::json!({"state": "running", "error": null}))?;
        assert_eq!(older.progress, None);
        Ok(())
    }
}
