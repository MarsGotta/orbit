use crate::{ChatPending, ChatRole, DaemonConnection, features::FeatureInput, topics::TopicCache};
use orbit_core::topics::{Knowledge, Pending, TopicData, TopicFrame, assistant};

#[test]
fn the_turn_and_approval_are_projected_without_assuming_unknown_roles() {
    let value = assistant::Value {
        session_id: Some("session-1".into()),
        agent_id: Some("atlas".into()),
        turn: Some(Box::new(orbit_core::Turn {
            user: orbit_core::Message {
                attachments: Vec::new(),
                id: "1".into(),
                role: "user".into(),
                text: "Hola".into(),
                complete: true,
                at_ms: 0,
                voice: None,
            },
            answer: Some(orbit_core::Message {
                attachments: Vec::new(),
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
            choices: vec!["once".into(), "deny".into()],
            questions: Vec::new(),
            deletes: Some(orbit_core::Deletion { paths: Some(2) }),
        }),
        waiting_sessions: Default::default(),
        history_revision: None,
        voice_active: false,
        pending_voice: Vec::new(),
        turn_in_flight: true,
        activity: None,
        voice: assistant::VoiceAvailability::Unknown {
            why: Pending::NotObservedYet,
        },
        // «Escuchar» on the answer: the screen shows «Parar» on it.
        speaking: Some("2".into()),
        call: Knowledge::Unknown {
            why: Pending::NotObservedYet,
        },
        short_voice_note: None,
        work: None,
        microphone_muted: false,
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
    assert_eq!(
        chat.agent_id.as_deref(),
        Some("atlas"),
        "the conversation's agent"
    );
    assert_eq!(chat.speaking.as_deref(), Some("2"));
    assert!(chat.turn.iter().any(|message| message.streaming));
    assert_eq!(chat.turn[0].role, ChatRole::User);
    assert_eq!(chat.turn[0].at_ms, 0);
    assert_eq!(chat.turn[1].role, ChatRole::System);
    assert_eq!(
        chat.pending_contract,
        Some(ChatPending::Approval {
            id: "approval-1".into(),
            text: "Permitir la acción".into(),
            choices: vec!["once".into(), "deny".into()],
            deletes: Some(crate::ChatDeletion { paths: Some(2) }),
        })
    );
}

#[test]
fn the_agents_questions_and_its_work_are_projected_as_they_came() {
    let mut work = orbit_core::agent_work::AgentWork::new("turno-1", 1_000);
    work.record(orbit_core::agent_work::WorkItem {
        id: "llamada-1".into(),
        kind: orbit_core::agent_work::WorkKind::Tool,
        phase: orbit_core::agent_work::WorkPhase::Started,
        label: "Leyendo el calendario".into(),
        subject: Some("el calendario".into()),
        steps: Vec::new(),
        cause: None,
    });
    let value = assistant::Value {
        session_id: Some("session-1".into()),
        agent_id: None,
        turn: None,
        pending: Some(orbit_core::Approval {
            request_id: "aclarar".into(),
            description: "Dos preguntas".into(),
            clarification: true,
            choices: vec!["Mañana".into()],
            questions: vec![orbit_core::Question {
                qid: "sala".into(),
                question: "¿Qué sala?".into(),
                choices: vec!["Norte".into(), "Sur".into()],
                multi_select: false,
            }],
            deletes: None,
        }),
        waiting_sessions: Default::default(),
        history_revision: None,
        voice_active: false,
        pending_voice: Vec::new(),
        turn_in_flight: true,
        activity: None,
        voice: assistant::VoiceAvailability::Unknown {
            why: Pending::NotObservedYet,
        },
        speaking: None,
        call: Knowledge::Unknown {
            why: Pending::NotObservedYet,
        },
        short_voice_note: None,
        work: Some(Box::new(work.clone())),
        microphone_muted: false,
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
    assert_eq!(
        chat.pending_contract,
        Some(ChatPending::Clarify {
            id: "aclarar".into(),
            text: "Dos preguntas".into(),
            choices: vec!["Mañana".into()],
            questions: vec![crate::ChatQuestion {
                qid: "sala".into(),
                question: "¿Qué sala?".into(),
                choices: vec!["Norte".into(), "Sur".into()],
                multi_select: false,
            }],
        })
    );
    assert_eq!(chat.work, Some(work));
}
