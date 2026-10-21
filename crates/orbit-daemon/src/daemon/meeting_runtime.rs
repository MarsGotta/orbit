//! Audio inventory requests and their completeness, kept in RAM.
use std::collections::BTreeMap;

#[derive(Default)]
pub(super) struct MeetingRuntime {
    pub(super) audio_source_requests: BTreeMap<String, Vec<String>>,
    /// An empty inventory is a fact only after every helper reply is verified.
    pub(super) audio_source_observed: bool,
}
