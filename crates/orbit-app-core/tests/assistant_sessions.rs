//! The open session is an assistant topic fact; a retained local value may
//! not restore an older session after a newer topic frame has arrived.
use orbit_app_core::{
    AppSnapshot, DaemonConnection,
    features::{FeatureInput, assistant},
    topics::TopicCache,
};
use orbit_core::topics::{
    Knowledge, Pending, TopicData, TopicFrame, TopicId, assistant::VoiceAvailability,
};

fn assistant_value(active: &str) -> orbit_core::topics::assistant::Value {
    orbit_core::topics::assistant::Value {
        session_id: Some(active.into()),
        agent_id: None,
        turn: None,
        pending: None,
        voice_active: false,
        turn_in_flight: false,
        activity: None,
        voice: VoiceAvailability::Unknown {
            why: Pending::NotObservedYet,
        },
        speaking: None,
        call: Knowledge::Unknown {
            why: Pending::NotObservedYet,
        },
        short_voice_note: None,
        work: None,
    }
}

fn publish_assistant(topics: &mut TopicCache, revision: u64, active: &str) {
    topics.observe_at(
        TopicFrame {
            epoch: "session-test".into(),
            revision,
            data: TopicData::Assistant(Knowledge::Present {
                value: assistant_value(active),
                as_of_ms: None,
            }),
        },
        Some(1_000),
    );
}

fn snapshot_with_a_retained_session() -> AppSnapshot {
    let mut local = AppSnapshot::empty(1, DaemonConnection::Ready);
    local.chat.session_id = Some("legacy".into());
    local
}

#[test]
fn an_older_topic_revision_cannot_restore_the_old_session() {
    let snapshot = snapshot_with_a_retained_session();
    let mut topics = TopicCache::default();
    publish_assistant(&mut topics, 3, "session-new");
    publish_assistant(&mut topics, 2, "session-a");
    let view = assistant::project(&FeatureInput::new(&snapshot, &topics));
    assert_eq!(view.session_id.as_deref(), Some("session-new"));
    assert_eq!(
        topics.topic(TopicId::Assistant).map(|topic| topic.revision),
        Some(3)
    );
}
