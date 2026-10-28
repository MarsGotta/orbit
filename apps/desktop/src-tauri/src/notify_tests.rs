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
    let mut watcher = Watcher::new(crate::copy::spanish());
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
    let mut short = Watcher::new(crate::copy::spanish());
    short.observe(&chat(Some(AssistantActivity::Responding)), true, 2_000);
    assert_eq!(
        short.observe(&worked(1_000, 1_000 + LONG_TURN_MS - 1), true, 30_000),
        None
    );
    let mut watched = Watcher::new(crate::copy::spanish());
    watched.observe(&chat(Some(AssistantActivity::Thinking)), false, 2_000);
    assert_eq!(watched.observe(&worked(1_000, 90_000), false, 90_000), None);
}

#[test]
fn without_its_work_a_turn_is_timed_from_its_question_to_now() {
    let mut watcher = Watcher::new(crate::copy::spanish());
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
    let mut watcher = Watcher::new(crate::copy::spanish());
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
        watcher.observe(&asking, true, 4_000),
        Some(TurnNotice {
            title: "Tiene una pregunta".into(),
            body: "Necesita tu respuesta para seguir".into(),
        })
    );
}

#[test]
fn a_question_asked_while_orbit_is_in_front_is_not_said_later() {
    let mut watcher = Watcher::new(crate::copy::spanish());
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
    let mut watcher = Watcher::new(crate::copy::spanish());
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
    let mut watcher = Watcher::new(crate::copy::spanish());
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
    let mut watcher = Watcher::new(crate::copy::spanish());
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
    let mut watcher = Watcher::new(crate::copy::spanish());
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

#[test]
fn english_chat_notifications_and_live_language_changes() {
    let mut watcher = Watcher::new(crate::copy::english());
    watcher.observe(&chat(Some(AssistantActivity::Thinking)), true, 2_000);
    assert_eq!(
        watcher.observe(&worked(1_000, 73_000), true, 80_000),
        Some(TurnNotice {
            title: "Answer ready".into(),
            body: "Worked for 1 min 12 s".into(),
        })
    );
    let mut asking = chat(None);
    asking.pending_contract = Some(ChatPending::Approval {
        id: "approval-1".into(),
        text: "Private request".into(),
        choices: vec![],
        deletes: None,
    });
    assert_eq!(
        watcher.observe(&asking, true, 81_000),
        Some(TurnNotice {
            title: "Needs your permission".into(),
            body: "Asks for permission to continue".into(),
        })
    );
    asking.pending_contract = Some(ChatPending::Clarify {
        id: "question-2".into(),
        text: "Private question".into(),
        choices: vec![],
        questions: vec![],
    });
    assert_eq!(
        watcher.observe(&asking, true, 82_000),
        Some(TurnNotice {
            title: "Has a question".into(),
            body: "Needs your answer to continue".into(),
        })
    );
    watcher.set_copy(crate::copy::spanish());
    asking.pending_contract = Some(ChatPending::Clarify {
        id: "question-3".into(),
        text: "Private question".into(),
        choices: vec![],
        questions: vec![],
    });
    assert_eq!(
        watcher.observe(&asking, true, 83_000).map(|n| n.title),
        Some("Tiene una pregunta".into())
    );
}

#[test]
fn english_recording_and_silence_notifications() {
    use orbit_core::notice::{Notice, NoticeCode, NoticeKey};
    let codes = [
        NoticeCode::RecordingCutWaiting,
        NoticeCode::RecordingResumedAfterCut,
        NoticeCode::RecordingClosedAfterCut,
        NoticeCode::CallSilenceQuestion,
        NoticeCode::RecordingSilenceQuestion,
    ];
    let notices: Vec<_> = codes
        .into_iter()
        .enumerate()
        .map(|(i, code)| {
            let mut notice = Notice::new(code)
                .with(NoticeKey::ElapsedMs, 12_000_u64)
                .with_token(NoticeKey::Cause, "helper_lost");
            notice.sequence = Some(i as u64 + 1);
            notice
        })
        .collect();
    let mut watcher = Watcher::new(crate::copy::english());
    assert_eq!(
        watcher
            .observe_system_notices(&notices)
            .into_iter()
            .map(|n| n.body)
            .collect::<Vec<_>>(),
        [
            "Recording was interrupted. Orbit is trying to keep recording.",
            "Recording is back. Missing 12 s.",
            "Recording was closed: the Mac helper disconnected. Your recording is saved.",
            "Are you still there? I’ll hang up in 15 seconds if you don’t respond",
            "Is the meeting still going? I’ll end it and save it in 30 s",
        ]
    );
    assert!(watcher.observe_system_notices(&notices).is_empty());
}

#[test]
fn recording_notifications_use_the_new_language_without_replaying_the_cut() {
    use orbit_core::notice::{Notice, NoticeCode};
    let mut watcher = Watcher::new(crate::copy::spanish());
    let mut cut = Notice::new(NoticeCode::RecordingCutWaiting);
    cut.sequence = Some(1);
    assert!(
        watcher.observe_system_notices(&[cut.clone()])[0]
            .body
            .starts_with("La grabación")
    );
    watcher.set_copy(crate::copy::english());
    assert!(watcher.observe_system_notices(&[cut]).is_empty());
    let mut resumed = Notice::new(NoticeCode::RecordingResumedAfterCut);
    resumed.sequence = Some(2);
    assert_eq!(
        watcher.observe_system_notices(&[resumed])[0].body,
        "Recording is back. Missing an unmeasured amount of time."
    );
}

#[test]
fn each_recording_closure_cause_is_catalogued_in_both_languages() {
    use orbit_core::notice::{Notice, NoticeCode, NoticeKey};
    for (code, es, en) in [
        (
            "helper_lost",
            "el ayudante del Mac se desconectó",
            "the Mac helper disconnected",
        ),
        (
            "session_inactive",
            "el Mac se durmió o cambió de usuario",
            "the Mac went to sleep or switched users",
        ),
        ("restart", "Orbit se reinició", "Orbit restarted"),
        (
            "daemon_heartbeat_expired",
            "el servicio de Orbit dejó de responder",
            "the Orbit service stopped responding",
        ),
        ("privacy_stop", "la parada de privacidad", "privacy stop"),
        (
            "capture_finalize_timed_out",
            "el Mac no confirmó el final",
            "the Mac did not confirm the end",
        ),
        (
            "cause_not_saved",
            "la causa no quedó guardada",
            "the cause was not saved",
        ),
        (
            "future_cause",
            "no se pudo recuperar la captura (future_cause)",
            "capture could not be recovered (future_cause)",
        ),
    ] {
        for (copy, cause) in [(crate::copy::spanish(), es), (crate::copy::english(), en)] {
            let mut watcher = Watcher::new(copy);
            let mut notice =
                Notice::new(NoticeCode::RecordingClosedAfterCut).with_token(NoticeKey::Cause, code);
            notice.sequence = Some(1);
            let shown = watcher.observe_system_notices(&[notice]);
            assert_eq!(shown.len(), 1);
            assert!(shown[0].body.contains(cause), "{}", shown[0].body);
        }
    }
}
