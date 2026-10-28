//! macOS notifications for the chat: a long turn ended, or the agent asks for
//! an answer, while Orbit is in the background. The macOS permission for
//! Orbit's notifications is the only decision: no setting of Orbit's own sits
//! on top of it (AGENTS.md, «A second layer of consent»).
//!
//! The decision follows the assistant frames (`Watcher`); posting goes
//! through `tauri-plugin-notification`. Neither text carries the conversation:
//! a notification stays in the Notification Center.

use orbit_app_core::{AssistantActivity, ChatPending, ChatSnapshot};
use std::sync::{Mutex, PoisonError};
use tauri::Manager;

/// The main window's label (`tauri.conf.json`).
const MAIN_LABEL: &str = "main";

/// The app's one watcher: frames reach it in order from `events::forward_frames`.
static WATCHER: Mutex<Watcher> = Mutex::new(Watcher {
    in_flight: false,
    pending: None,
    last_notice_sequence: 0,
    recording_cut_notified: false,
});

/// A turn that worked at least this long ends with a notification when Orbit
/// is in the background. Twenty seconds: a shorter answer arrives while the
/// user still waits for it; past that they have likely gone to another app.
/// Not measured with users; it bounds nothing, it only chooses what is said.
pub const LONG_TURN_MS: u64 = 20_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnNotice {
    pub title: String,
    pub body: String,
}

/// «40 s», «1 min 12 s», «2 min»: the duration as the chat writes it.
pub fn duration_text(ms: u64) -> String {
    let seconds = ms / 1_000;
    let (minutes, seconds) = (seconds / 60, seconds % 60);
    match (minutes, seconds) {
        (0, seconds) => format!("{seconds} s"),
        (minutes, 0) => format!("{minutes} min"),
        (minutes, seconds) => format!("{minutes} min {seconds} s"),
    }
}

/// What the assistant frames said last, to tell what changed.
#[derive(Debug, Default)]
pub struct Watcher {
    last_notice_sequence: u64,
    recording_cut_notified: bool,
    in_flight: bool,
    pending: Option<String>,
}

fn pending_id(pending: &ChatPending) -> &str {
    match pending {
        ChatPending::Approval { id, .. } | ChatPending::Clarify { id, .. } => id,
    }
}

impl Watcher {
    /// The notification `chat` calls for, if any. `background`: Orbit's
    /// window is hidden or another app has the focus.
    pub fn observe(
        &mut self,
        chat: &ChatSnapshot,
        background: bool,
        now_ms: u64,
    ) -> Option<TurnNotice> {
        let in_flight = matches!(
            chat.activity,
            Some(AssistantActivity::Thinking | AssistantActivity::Responding)
        );
        let pending = chat.pending_contract.as_ref();
        let asked = pending.filter(|pending| Some(pending_id(pending)) != self.pending.as_deref());
        let ended = self.in_flight && !in_flight;
        self.in_flight = in_flight;
        self.pending = pending.map(|pending| pending_id(pending).to_owned());
        if !background {
            return None;
        }
        if let Some(pending) = asked {
            return Some(match pending {
                ChatPending::Approval { .. } => TurnNotice {
                    title: "Necesita tu permiso".into(),
                    body: "Pide permiso para seguir".into(),
                },
                ChatPending::Clarify { .. } => TurnNotice {
                    title: "Tiene una pregunta".into(),
                    body: "Necesita tu respuesta para seguir".into(),
                },
            });
        }
        if !ended {
            return None;
        }
        // Only the ended turn's own work times it: the daemon may still
        // carry the last turn's.
        let question = chat.turn.first();
        let work = chat
            .work
            .as_ref()
            .filter(|work| question.is_some_and(|question| question.id == work.turn_id));
        let started_at_ms = work
            .map(|work| work.started_at_ms)
            .or_else(|| question.and_then(|message| u64::try_from(message.at_ms).ok()))?;
        let ended_at_ms = work.and_then(|work| work.ended_at_ms).unwrap_or(now_ms);
        let worked = ended_at_ms.saturating_sub(started_at_ms);
        (worked >= LONG_TURN_MS).then(|| TurnNotice {
            title: "Respuesta lista".into(),
            body: format!("Trabajó {}", duration_text(worked)),
        })
    }
}

/// Orbit is in the background: its window is hidden (closed to the menu
/// bar) or another app has the focus. The floating panel never takes it.
fn in_background<R: tauri::Runtime>(app: &tauri::AppHandle<R>) -> bool {
    app.get_webview_window(MAIN_LABEL).is_none_or(|window| {
        !window.is_visible().unwrap_or(false) || !window.is_focused().unwrap_or(false)
    })
}

/// Follows one assistant frame and posts what it calls for. A failure is
/// said and changes nothing else: the chat shows the same.
pub fn follow<R: tauri::Runtime>(app: &tauri::AppHandle<R>, chat: &ChatSnapshot) {
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| {
            u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX)
        });
    let notice = WATCHER
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .observe(chat, in_background(app), now_ms);
    let Some(notice) = notice else {
        return;
    };
    post(app, notice, false);
}

/// Recording interruptions and silence questions use the macOS channel even
/// while Orbit is in front. Only the system notification permission applies.
pub fn follow_system_notices<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    notices: &[orbit_core::notice::Notice],
) {
    let pending = WATCHER
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .observe_system_notices(notices);
    for notice in pending {
        post(app, notice, true);
    }
}

fn post<R: tauri::Runtime>(app: &tauri::AppHandle<R>, notice: TurnNotice, sound: bool) {
    // Absent under Tauri's mock runtime, where the plugin is not installed.
    let Some(notifications) = app.try_state::<tauri_plugin_notification::Notification<R>>() else {
        return;
    };
    let builder = notifications
        .builder()
        .title(notice.title)
        .body(notice.body);
    let builder = if sound {
        builder.sound("NSUserNotificationDefaultSoundName")
    } else {
        builder
    };
    if let Err(error) = builder.show() {
        eprintln!("No se pudo mostrar el aviso de macOS: {error}");
    }
}

impl Watcher {
    pub fn observe_system_notices(
        &mut self,
        notices: &[orbit_core::notice::Notice],
    ) -> Vec<TurnNotice> {
        use orbit_core::notice::NoticeCode;
        let mut result = Vec::new();
        for notice in notices {
            let Some(sequence) = notice
                .sequence
                .filter(|seq| *seq > self.last_notice_sequence)
            else {
                continue;
            };
            if let Some(body) = self.recording_notice(notice) {
                result.push(TurnNotice {
                    title: "Orbit".into(),
                    body,
                });
                continue;
            }
            let (body, resolved) = match notice.code {
                NoticeCode::CallSilenceQuestion => (
                    "¿Sigues ahí? Si no dices nada, cuelgo en 15 segundos",
                    NoticeCode::CallSilenceResolved,
                ),
                NoticeCode::RecordingSilenceQuestion => (
                    "¿Sigue la reunión? La termino y la guardo en 30 s",
                    NoticeCode::RecordingSilenceResolved,
                ),
                _ => continue,
            };
            if notices.iter().any(|later| {
                later.sequence.is_some_and(|seq| seq > sequence) && later.code == resolved
            }) {
                continue;
            }
            result.push(TurnNotice {
                title: "Orbit".into(),
                body: body.into(),
            });
        }
        self.last_notice_sequence = notices
            .iter()
            .filter_map(|n| n.sequence)
            .max()
            .unwrap_or(self.last_notice_sequence)
            .max(self.last_notice_sequence);
        result
    }
}

impl Watcher {
    fn recording_notice(&mut self, notice: &orbit_core::notice::Notice) -> Option<String> {
        use orbit_core::notice::{DataValue, NoticeCode, NoticeKey};
        match notice.code {
            NoticeCode::CaptureStarting
            | NoticeCode::RecordingFinishing
            | NoticeCode::RecordingFinished
            | NoticeCode::RecordingDiscarded => {
                self.recording_cut_notified = false;
                None
            }
            NoticeCode::RecordingCutWaiting => {
                self.recording_cut_notified = true;
                Some("La grabación se ha cortado. Orbit intenta seguir grabando.".into())
            }
            NoticeCode::RecordingResumedAfterCut => {
                let notified = std::mem::take(&mut self.recording_cut_notified);
                if !notified {
                    return None;
                }
                let duration = match notice.data.get(&NoticeKey::ElapsedMs) {
                    Some(DataValue::Int(ms)) if *ms >= 0 => duration_text(*ms as u64),
                    _ => "un tiempo que no se pudo medir".into(),
                };
                Some(format!("La grabación sigue. Faltan {duration}."))
            }
            NoticeCode::RecordingClosedAfterCut => {
                self.recording_cut_notified = false;
                let cause = match notice.data.get(&NoticeKey::Cause) {
                    Some(DataValue::Token(code)) => recording_cause(code.as_str()),
                    _ => "no se pudo recuperar la captura".into(),
                };
                Some(format!(
                    "La grabación se ha cerrado: {cause}. Lo grabado está guardado."
                ))
            }
            _ => None,
        }
    }
}

fn recording_cause(code: &str) -> String {
    match code {
        "helper_lost" => "el ayudante del Mac se desconectó".into(),
        "session_inactive" => "el Mac se durmió o cambió de usuario".into(),
        "restart" => "Orbit se reinició".into(),
        "daemon_heartbeat_expired" => "el servicio de Orbit dejó de responder".into(),
        "privacy_stop" => "la parada de privacidad".into(),
        "capture_finalize_timed_out" => "el Mac no confirmó el final".into(),
        "cause_not_saved" => "la causa no quedó guardada".into(),
        _ => format!("no se pudo recuperar la captura ({code})"),
    }
}

#[cfg(test)]
#[path = "notify_tests.rs"]
mod tests;
