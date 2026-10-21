use crate::{ChatPending, ChatRole, DaemonConnection, features::FeatureInput, topics::TopicCache};
use orbit_core::topics::{Knowledge, Pending, TopicData, TopicFrame, assistant};

#[test]
fn the_turn_and_approval_are_projected_without_assuming_unknown_roles() {
    let value = assistant::Value {
        session_id: Some("session-1".into()),
        turn: Some(Box::new(orbit_core::Turn {
            user: orbit_core::Message {
                id: "1".into(),
                role: "user".into(),
                text: "Hola".into(),
                complete: true,
                at_ms: 0,
                voice: None,
            },
            answer: Some(orbit_core::Message {
                id: "2".into(),
                role: "future-role".into(),
                text: "Dato".into(),
                complete: false,
                at_ms: 0,
                voice: None,
            }),
        })),
        pending: Some(orbit_core::Approval {
            request_id: "approval-1".into(),
            description: "Permitir la acción".into(),
            clarification: false,
        }),
        voice_active: false,
        turn_in_flight: true,
        activity: None,
        voice: assistant::VoiceAvailability::Unknown {
            why: Pending::NotObservedYet,
        },
        call: Knowledge::Unknown {
            why: Pending::NotObservedYet,
        },
    };
    let mut topics = TopicCache::default();
    topics.observe_at(
        TopicFrame {
            epoch: "assistant-test".into(),
            revision: 1,
            data: TopicData::Assistant(Knowledge::Present {
                value,
                as_of_ms: None,
            }),
        },
        Some(1),
    );
    let local = crate::AppSnapshot::empty(8, DaemonConnection::Ready);
    let chat = crate::features::assistant::project(&FeatureInput::new(&local, &topics));
    assert_eq!(chat.session_id.as_deref(), Some("session-1"));
    assert!(chat.turn.iter().any(|message| message.streaming));
    assert_eq!(chat.turn[0].role, ChatRole::User);
    assert_eq!(chat.turn[0].at_ms, 0);
    assert_eq!(chat.turn[1].role, ChatRole::System);
    assert_eq!(
        chat.pending_contract,
        Some(ChatPending::Approval {
            id: "approval-1".into(),
            text: "Permitir la acción".into()
        })
    );
}
