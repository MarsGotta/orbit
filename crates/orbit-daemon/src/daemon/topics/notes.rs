//! What changed in the meetings and notes, so the app reads them
//! again from the hub (`orbit_core::topics::notes`).
use super::{ProjectTopic, TopicSource};
use orbit_core::topics::{Knowledge, TopicData, TopicId};

pub(super) struct Projector;

impl ProjectTopic for Projector {
    fn project(&self, source: &TopicSource<'_>) -> TopicData {
        let Some(runtime) = source.runtime else {
            return TopicData::not_observed(TopicId::Notes);
        };
        let mut value = runtime.notes.head.clone();
        value.recording = runtime.active_recording_id().map(str::to_owned);
        TopicData::Notes(Knowledge::Present {
            value,
            as_of_ms: None,
        })
    }
}
