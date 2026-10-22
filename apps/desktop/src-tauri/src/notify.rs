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
    // Absent under Tauri's mock runtime, where the plugin is not installed.
    let Some(notifications) = app.try_state::<tauri_plugin_notification::Notification<R>>() else {
        return;
    };
    if let Err(error) = notifications
        .builder()
        .title(notice.title)
        .body(notice.body)
        .show()
    {
        eprintln!("No se pudo mostrar el aviso de macOS: {error}");
    }
}

#[cfg(test)]
#[path = "notify_tests.rs"]
mod tests;
