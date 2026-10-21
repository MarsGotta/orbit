use crate::*;
use orbit_core::{
    CapturePhase,
    audio_signal::{SignalCondition, TrackSignalView},
    calendar::CalendarOccurrence,
    protocol::MeetingAudio,
};

mod control;
pub(crate) mod library;
pub(crate) mod note;
pub(crate) mod notes;
mod status;
pub(crate) use status::project_update;
mod agenda;
pub(crate) use agenda::*;
mod recording;
pub(crate) use recording::{pending_inventory_detail, project_meeting_topic};
mod assistant;

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
pub(crate) mod test_support;

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
#[path = "briefing_tests.rs"]
mod briefing_tests;
