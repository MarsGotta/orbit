//! Pure decisions and shared contracts. Effects are descriptions, never execution.
pub mod actions;
pub mod agent_work;
pub mod agents;
pub mod assistant_files;
pub mod audio_gap;
pub mod audio_signal;
pub mod authority;
#[cfg(test)]
pub(crate) mod authority_fixture;
pub mod calendar;
pub mod calendar_control;
pub mod call_detection;
pub mod computer_control;
pub mod device_bridge;
pub mod failure;
pub mod limits;
pub mod meeting_audio;
pub mod meeting_question;
pub mod model;
pub mod notes;
pub mod notice;
pub mod orbit_bundle;
pub mod privacy_stop;
pub mod process_listing;
pub mod protocol;
pub mod reads;
pub mod refusal;
pub mod sources;
pub mod topics;
pub mod transition;
pub mod update;
pub mod voice;

pub use model::*;
pub use transition::{Decision, Effect, Event, decide};
pub use voice::{CallPhase, CallState};

pub mod wire;

/// Maximum wait for the native helper to confirm a capture drain before the
/// recording is considered abandoned. The 3-second native ACK budget is
/// doubled and padded so a transient queue delay does not turn a healthy
/// closing pipeline into an interrupted meeting with a missing tail.
pub const FINALIZING_TIMEOUT_MS: i64 = 15_000;

pub mod features;
