use super::*;
use orbit_app_core::{AppSnapshot, ChatMessage, ChatRole, DaemonConnection};
use orbit_core::agent_work::AgentWork;

fn chat(activity: Option<AssistantActivity>) -> ChatSnapshot {
    let mut chat = AppSnapshot::empty(1, DaemonConnection::Ready).chat;
    chat.activity = activity;
    chat.turn = vec![ChatMessage {
        id: "turno-1".into(),
        role: ChatRole::User,
        text: "Pregunta sintética".into(),
        at_ms: 1_000,
        streaming: false,
        voice: None,
        receipts: Vec::new(),
    }];
    chat
}

fn worked(started_at_ms: u64, ended_at_ms: u64) -> ChatSnapshot {
    let mut done = chat(None);
    let mut work = AgentWork::new("turno-1", started_at_ms);
    work.end(ended_at_ms);
    done.work = Some(work);
    done
}

#[test]
fn a_long_turn_that_ends_in_the_background_says_how_long_it_worked() {
    let mut watcher = Watcher::default();
    assert_eq!(
        watcher.observe(&chat(Some(AssistantActivity::Thinking)), true, 2_000),
        None
    );
    assert_eq!(
        watcher.observe(&worked(1_000, 73_000), true, 80_000),
        Some(TurnNotice {
            title: "Respuesta lista".into(),
            body: "Trabajó 1 min 12 s".into(),
        })
    );
    assert_eq!(
        watcher.observe(&worked(1_000, 73_000), true, 81_000),
        None,
        "an ended turn is said once"
    );
}

#[test]
fn a_short_turn_or_one_seen_in_the_foreground_says_nothing() {
    let mut short = Watcher::default();
    short.observe(&chat(Some(AssistantActivity::Responding)), true, 2_000);
    assert_eq!(
        short.observe(&worked(1_000, 1_000 + LONG_TURN_MS - 1), true, 30_000),
        None
    );
    let mut watched = Watcher::default();
    watched.observe(&chat(Some(AssistantActivity::Thinking)), false, 2_000);
    assert_eq!(watched.observe(&worked(1_000, 90_000), false, 90_000), None);
}

#[test]
fn without_its_work_a_turn_is_timed_from_its_question_to_now() {
    let mut watcher = Watcher::default();
    watcher.observe(&chat(Some(AssistantActivity::Thinking)), true, 2_000);
    assert_eq!(
        watcher.observe(&chat(None), true, 41_000),
        Some(TurnNotice {
            title: "Respuesta lista".into(),
            body: "Trabajó 40 s".into(),
        })
    );
}

#[test]
fn a_question_from_the_agent_in_the_background_is_said_once_by_its_kind() {
    let mut watcher = Watcher::default();
    let mut asking = chat(Some(AssistantActivity::Thinking));
    asking.pending_contract = Some(ChatPending::Approval {
        id: "permiso-1".into(),
        text: "Borrar la carpeta de pruebas".into(),
        choices: vec!["once".into(), "deny".into()],
    });
    let notice = watcher.observe(&asking, true, 2_000);
    assert_eq!(
        notice,
        Some(TurnNotice {
            title: "Necesita tu permiso".into(),
            body: "Pide permiso para seguir".into(),
        })
    );
    assert!(
        notice.is_some_and(|notice| !notice.body.contains("carpeta")),
        "the request's text stays out of the Notification Center"
    );
    assert_eq!(watcher.observe(&asking, true, 3_000), None);
    asking.pending_contract = Some(ChatPending::Clarify {
        id: "pregunta-2".into(),
        text: "¿Qué sala?".into(),
        choices: Vec::new(),
        questions: Vec::new(),
    });
    assert_eq!(
        watcher
            .observe(&asking, true, 4_000)
            .map(|notice| notice.title),
        Some("Tiene una pregunta".into())
    );
}

#[test]
fn a_question_asked_while_orbit_is_in_front_is_not_said_later() {
    let mut watcher = Watcher::default();
    let mut asking = chat(Some(AssistantActivity::Thinking));
    asking.pending_contract = Some(ChatPending::Approval {
        id: "permiso-1".into(),
        text: "Permiso".into(),
        choices: Vec::new(),
    });
    assert_eq!(watcher.observe(&asking, false, 2_000), None);
    assert_eq!(watcher.observe(&asking, true, 3_000), None);
}

#[test]
fn durations_read_as_the_chat_writes_them() {
    assert_eq!(duration_text(40_999), "40 s");
    assert_eq!(duration_text(72_000), "1 min 12 s");
    assert_eq!(duration_text(120_000), "2 min");
}
