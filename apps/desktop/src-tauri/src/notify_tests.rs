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
        attachments: Vec::new(),
        work: None,
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
        deletes: None,
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
        deletes: None,
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

/// The daemon may still carry the last turn's work when the next one ends
/// (a turn without tools sends no `chat.item`): a two-second answer after a
/// one-minute turn with tools must not say «Trabajó 1 min».
#[test]
fn a_short_turn_after_a_long_one_is_not_timed_by_the_long_ones_work() {
    let mut previous = AgentWork::new("turno-0", 1_000);
    previous.end(61_000);
    let mut watcher = Watcher::default();
    let mut asking = chat(Some(AssistantActivity::Thinking));
    asking.turn[0].at_ms = 100_000;
    asking.work = Some(previous.clone());
    assert_eq!(watcher.observe(&asking, true, 100_500), None);
    let mut answered = chat(None);
    answered.turn[0].at_ms = 100_000;
    answered.work = Some(previous);
    assert_eq!(
        watcher.observe(&answered, true, 102_000),
        None,
        "the turn worked 2 s; the notice is timed by the previous turn's work"
    );
}

#[test]
fn silence_questions_notify_once_including_the_foreground_without_content() {
    use orbit_core::notice::{Notice, NoticeCode};
    let mut watcher = Watcher::default();
    let mut call = Notice::new(NoticeCode::CallSilenceQuestion);
    call.sequence = Some(8);
    let first = watcher.observe_system_notices(&[call.clone()]);
    assert_eq!(first.len(), 1);
    assert_eq!(
        first[0].body,
        "¿Sigues ahí? Si no dices nada, cuelgo en 15 segundos"
    );
    assert!(watcher.observe_system_notices(&[call]).is_empty());
    let mut meeting = Notice::new(NoticeCode::RecordingSilenceQuestion);
    meeting.sequence = Some(9);
    assert_eq!(
        watcher.observe_system_notices(&[meeting])[0].body,
        "¿Sigue la reunión? La termino y la guardo en 30 s"
    );
}

#[test]
fn recording_notices_post_in_front_once_without_meeting_content() {
    use orbit_core::notice::{Notice, NoticeCode, NoticeKey};
    let mut watcher = Watcher::default();
    let mut cut = Notice::new(NoticeCode::RecordingCutWaiting);
    cut.sequence = Some(1);
    let mut resumed =
        Notice::new(NoticeCode::RecordingResumedAfterCut).with(NoticeKey::ElapsedMs, 12_000_u64);
    resumed.sequence = Some(2);
    let mut closed = Notice::new(NoticeCode::RecordingClosedAfterCut)
        .with_token(NoticeKey::Cause, "helper_lost");
    closed.sequence = Some(3);
    // This channel is called even in front, just like silence questions.
    let notices = watcher.observe_system_notices(&[cut.clone(), resumed.clone(), closed.clone()]);
    assert_eq!(
        notices.iter().map(|n| n.body.as_str()).collect::<Vec<_>>(),
        [
            "La grabación se ha cortado. Orbit intenta seguir grabando.",
            "La grabación sigue. Faltan 12 s.",
            "La grabación se ha cerrado: el ayudante del Mac se desconectó. Lo grabado está guardado.",
        ]
    );
    assert!(notices.iter().all(|n| n.title == "Orbit"));
    assert!(
        watcher
            .observe_system_notices(&[cut, resumed, closed])
            .is_empty()
    );
}

#[test]
fn a_brief_cut_and_a_new_meeting_do_not_reuse_an_earlier_cut_notification() {
    use orbit_core::notice::{Notice, NoticeCode};
    let mut watcher = Watcher::default();
    let mut resumed = Notice::new(NoticeCode::RecordingResumedAfterCut);
    resumed.sequence = Some(1);
    assert!(
        watcher
            .observe_system_notices(&[resumed.clone()])
            .is_empty()
    );
    let mut cut = Notice::new(NoticeCode::RecordingCutWaiting);
    cut.sequence = Some(2);
    assert_eq!(watcher.observe_system_notices(&[cut]).len(), 1);
    let mut started = Notice::new(NoticeCode::CaptureStarting);
    started.sequence = Some(3);
    resumed.sequence = Some(4);
    assert!(
        watcher
            .observe_system_notices(&[started, resumed])
            .is_empty()
    );
}
