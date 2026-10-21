//! The daemon's compact recording facts, including an independently known inventory.
use super::Knowledge;
use crate::{Recording, audio_signal::RecordingSignalView, call_detection::CallPreference};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Application {
    pub name: String,
    pub pid: i32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Value {
    // Keep the shared TopicData enum compact: an inline recording head made
    // this variant 472 bytes versus 40 bytes for the next largest one.
    pub recording: Option<Box<Recording>>,
    pub recording_signal: Option<Box<RecordingSignalView>>,
    pub pending_jobs: u64,
    pub offline: bool,
    /// The call detection. Absent from a daemon that predates it, or
    /// when its preference could not be read: unknown, never «no».
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub call: Option<Box<Call>>,
    pub applications: Knowledge<Vec<Application>>,
    /// The last question to Deimos during the meeting and where its
    /// turn is; the answer is the conversation's message `{id}:assistant`.
    /// Absent: nothing to show (an older daemon, or none asked).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub question: Option<Box<crate::meeting_question::MeetingQuestion>>,
    /// Whether «Oye Deimos» listens during the recording and, when
    /// not, why. Absent outside a recording (nothing to say then).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub voice_trigger: Option<crate::meeting_question::VoiceStatus>,
}

/// What the call detection says: the user's preference and what the floating
/// panel has to say, if anything.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Call {
    /// «Al detectar una llamada»; `None` while it could not be read (the
    /// detection then only offers, as «Avisarme»).
    #[serde(default)]
    pub preference: Option<CallPreference>,
    /// Why the preference could not be read from the store, when it could
    /// not: distinct from a preference not published yet (both `None`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preference_unreadable: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub panel: Option<CallPanel>,
}

/// The floating panel's content. Each state names what its button
/// answers: the offer's `call_id`, or the recording's id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum CallPanel {
    /// «Llamada en {application}»: Grabar / Ahora no.
    Offer {
        call_id: String,
        application: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        event_title: Option<String>,
    },
    /// A recording that started on its own: Parar.
    Recording {
        recording_id: String,
        application: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        event_title: Option<String>,
    },
    /// «La llamada terminó · termino de grabar en N s»: Seguir grabando.
    /// `ends_at_ms` is Unix milliseconds; the app counts down to it.
    Ending {
        recording_id: String,
        application: String,
        ends_at_ms: i64,
    },
}
