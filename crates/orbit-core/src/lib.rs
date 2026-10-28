//! Pure decisions and shared contracts. Effects are descriptions, never execution.
pub mod actions;
pub mod agent_config;
pub mod agent_work;
pub mod agents;
pub mod ai_config;
pub mod assistant_files;
pub mod attachments;
pub mod audio_gap;
pub mod audio_signal;
pub mod authority;
#[cfg(test)]
pub(crate) mod authority_fixture;
pub mod calendar;
pub mod calendar_control;
pub mod call_detection;
mod catalogue;
pub mod computer_control;
pub mod device_bridge;
pub mod failure;
pub mod hub_storage;
pub mod language;
pub mod limits;
pub mod meeting_audio;
pub mod meeting_question;
pub mod microphone_mute;
pub mod model;
pub mod note_outbox;
pub mod notes;
pub mod notice;
pub mod orbit_bundle;
mod pending_voice;
pub mod privacy_stop;
pub mod process_listing;
pub mod protocol;
pub mod reads;
pub mod refusal;
pub mod reminders;
pub mod series_title;
pub mod sources;
pub mod speech;
pub mod topics;
pub mod transcription_config;
pub mod transition;
pub mod update;
pub mod vault;
pub mod voice;
pub mod window_text;

pub use model::*;
pub use pending_voice::{
    PendingVoiceNote, SHORT_REFUSED_VOICE_MS, VoiceNoteConversation, omit_voice_failure,
};
pub use transition::{Decision, Effect, Event, decide};
pub use voice::{CallOrigin, CallPhase, CallState};

pub mod wire;

/// Maximum wait for the native helper to confirm a capture drain before the
/// recording is considered abandoned. The 3-second native ACK budget is
/// doubled and padded so a transient queue delay does not turn a healthy
/// closing pipeline into an interrupted meeting with a missing tail.
pub const FINALIZING_TIMEOUT_MS: i64 = 15_000;

pub mod features;

pub mod silence;
