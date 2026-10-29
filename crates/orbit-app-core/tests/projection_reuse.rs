//! Retained typed views must survive unrelated publications.
use orbit_app_core::{
    AppSnapshot, DaemonConnection, NoteDayGroup, NoteListItem, NoteListState,
    features::{
        FeatureFrame, FeatureId, FeatureInput, FeatureProjection, FrameRevision, project_all,
    },
    topics::TopicCache,
};
use orbit_core::topics::{TopicData, TopicFrame, TopicId};

#[test]
fn recording_updates_reuse_notes_and_match_a_complete_projection()
-> Result<(), Box<dyn std::error::Error>> {
    let mut snapshot = AppSnapshot::empty(1, DaemonConnection::Ready);
    snapshot
        .notes
        .as_mut()
        .ok_or("notes")?
        .list
        .push(NoteListItem {
            recorded_on: None,
            recording_id: "meeting-one".into(),
            title: "Budget review".into(),
            started_at_ms: 0,
            duration_ms: 60_000,
            state: NoteListState::Ready,
            group: NoteDayGroup::Today,
            headline: None,
        });
    let mut topics = TopicCache::default();
    topics.observe_at(
        TopicFrame {
            epoch: "fixture".into(),
            revision: 1,
            data: TopicData::not_observed(TopicId::Recording),
        },
        Some(0),
    );
    let first = FeatureInput::new(&snapshot, &topics);
    let mut projected = FeatureProjection::new(
        &first,
        FrameRevision {
            epoch: 1,
            counter: 0,
        },
    );
    let retained_notes = notes_address(projected.frames()).ok_or("notes frame")?;
    let mut next_topics = topics.clone();
    next_topics.observe_at(
        TopicFrame {
            epoch: "fixture".into(),
            revision: 2,
            data: TopicData::not_observed(TopicId::Recording),
        },
        Some(250),
    );
    let next = FeatureInput::new(&snapshot, &next_topics);
    let mut counter = 0;
    let changed = projected.update(&next, || {
        counter += 1;
        FrameRevision { epoch: 1, counter }
    });
    assert!(
        !changed
            .iter()
            .any(|frame| frame.feature() == FeatureId::Notes)
    );
    assert_eq!(notes_address(projected.frames()), Some(retained_notes));
    let complete = project_all(
        &next,
        FrameRevision {
            epoch: 1,
            counter: 0,
        },
    );
    for (actual, expected) in projected.frames().iter().zip(complete) {
        let mut actual_snapshot = AppSnapshot::default();
        let mut expected_snapshot = AppSnapshot::default();
        actual.apply(&mut actual_snapshot);
        expected.apply(&mut expected_snapshot);
        assert_eq!(actual_snapshot, expected_snapshot, "{:?}", actual.feature());
    }
    Ok(())
}

fn notes_address(frames: &[FeatureFrame]) -> Option<*const Option<orbit_app_core::NotesSnapshot>> {
    frames.iter().find_map(|frame| match frame {
        FeatureFrame::Notes { view, .. } => Some(std::ptr::from_ref(&**view)),
        _ => None,
    })
}

#[test]
fn retained_views_follow_every_topic_epoch_and_connection_change() {
    let snapshot = AppSnapshot::empty(1, DaemonConnection::Ready);
    let mut topics = TopicCache::default();
    let revision = FrameRevision {
        epoch: 1,
        counter: 0,
    };
    let mut projected = FeatureProjection::new(&FeatureInput::new(&snapshot, &topics), revision);
    for (index, id) in TopicId::ALL.iter().copied().enumerate() {
        topics.observe_at(
            TopicFrame {
                epoch: "fixture".into(),
                revision: index as u64 + 1,
                data: TopicData::absent(
                    id,
                    orbit_core::refusal::Refusal::PrivacyStopped {
                        since_ms: Some(1_000),
                    },
                ),
            },
            Some(1_000),
        );
        projected.update(&FeatureInput::new(&snapshot, &topics), || revision);
        assert_complete(&projected, &FeatureInput::new(&snapshot, &topics), revision);
    }
    topics.ended("fixture daemon stopped".into());
    projected.update(&FeatureInput::new(&snapshot, &topics), || revision);
    assert_complete(&projected, &FeatureInput::new(&snapshot, &topics), revision);
    topics.observe_at(
        TopicFrame {
            epoch: "restarted".into(),
            revision: 1,
            data: TopicData::not_observed(TopicId::Status),
        },
        Some(2_000),
    );
    projected.update(&FeatureInput::new(&snapshot, &topics), || revision);
    assert_complete(&projected, &FeatureInput::new(&snapshot, &topics), revision);
}

fn assert_complete(
    projected: &FeatureProjection,
    input: &FeatureInput<'_>,
    revision: FrameRevision,
) {
    let mut actual = AppSnapshot::default();
    let mut expected = AppSnapshot::default();
    for frame in projected.frames() {
        frame.apply(&mut actual);
    }
    for frame in project_all(input, revision) {
        frame.apply(&mut expected);
    }
    assert_eq!(actual, expected);
}
