use crate::{DaemonConnection, EngineSnapshot, features::FeatureInput, topics::TopicCache};
use orbit_core::{
    reads::ReadRequest,
    topics::{Knowledge, TopicData, TopicFrame, TopicId, control},
};

/// The typed topic and keyed catalogue carry no retired
/// frictionless or per-action approval field.
#[test]
fn projects_the_engine_catalogue_and_nothing_to_approve() {
    let mut topics = TopicCache::default();
    topics.observe_at(
        TopicFrame {
            epoch: "control-test".into(),
            revision: 1,
            data: TopicData::Control(Knowledge::Present {
                value: control::Value {
                    engine: Knowledge::Present {
                        value: control::Engine::Ready { tools: 2 },
                        as_of_ms: None,
                    },
                    retry_at_ms: None,
                    action: None,
                    history: Knowledge::Present {
                        value: control::History {
                            total: 0,
                            recent: Vec::new(),
                        },
                        as_of_ms: None,
                    },
                    folders: Box::new(Knowledge::Unknown {
                        why: orbit_core::topics::Pending::NotObservedYet,
                    }),
                },
                as_of_ms: None,
            }),
        },
        Some(1),
    );
    assert!(topics.remember_read(
        &ReadRequest::EngineTools { cursor: None },
        TopicId::Control,
        Knowledge::Present {
            value: orbit_core::topics::Page {
                items: vec![
                    serde_json::json!("launch_app"),
                    serde_json::json!("a_tool_invented_by_a_newer_engine")
                ],
                next: None,
                total: 2,
                revision: 1,
                store_version: None,
            },
            as_of_ms: None
        }
    ));
    let local = crate::AppSnapshot::empty(1, DaemonConnection::Ready);
    let control = crate::features::control::project(&FeatureInput::new(&local, &topics))
        .expect("the control topic projects a view");
    assert_eq!(control.engine, EngineSnapshot::Ready { tools: 2 });
    assert_eq!(
        control.engine_tools,
        vec!["launch_app", "a_tool_invented_by_a_newer_engine"]
    );
    let encoded = serde_json::to_value(&control).expect("control wire");
    for retired in [
        "frictionless",
        "frictionless_stopped_by_privacy",
        "approval_classes",
        "control_enabled",
    ] {
        assert!(
            encoded.get(retired).is_none(),
            "{retired} must not survive: {encoded}"
        );
    }
}
