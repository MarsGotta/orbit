//! The pointer over the floating panel holds the question it shows
//! (`Command::MeetingQuestionHold`): the daemon lets a question go at its
//! `shown_until_ms`, so stopping the bar in the interface alone would lie.
//!
//! Why the pointer is read here and not from the webview: the panel is never
//! the key window (`focusable: false` keeps the keyboard with the call, and
//! tao then answers `canBecomeKeyWindow` with NO), and WKWebView tracks the
//! mouse for `mouseenter` and `mouseleave` in the key window only, unless the
//! Mac shows legacy scroll bars (WebKit's `trackingAreaOptions`). So the DOM
//! events may or may not arrive, depending on a system setting. The system's
//! pointer (`NSEvent.mouseLocation`, through Tauri's `cursor_position`) needs
//! no focus, and gives the same answer in both cases.
use orbit_app_core::{AppCommand, DaemonClient, features::FeatureFrame};
use std::{sync::Mutex, time::Duration};
use tauri::Manager;

/// How often the pointer is read while the panel shows a question: the hold
/// begins within a quarter of a second of the pointer arriving, well inside
/// the 45 s an answer is shown and the 5 s grace after it leaves. One read is
/// one system call; with no question shown nothing is read.
const POLL: Duration = Duration::from_millis(250);

/// The question the panel shows, by id, from the last recording frame.
static SHOWN: Mutex<Option<String>> = Mutex::new(None);

/// Keeps the id of the question the recording frame shows (none outside one).
pub fn follow(frame: &FeatureFrame) {
    if let FeatureFrame::Recording { view, .. } = frame {
        let shown = view
            .meeting
            .question
            .as_ref()
            .map(|question| question.id.clone());
        *SHOWN
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = shown;
    }
}

/// A rectangle of the desktop in logical points, top-left origin.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Area {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Area {
    pub fn contains(&self, x: f64, y: f64) -> bool {
        x >= self.x && x < self.x + self.width && y >= self.y && y < self.y + self.height
    }
}

/// The hold order to send: the question and whether the pointer is over it.
pub type HoldOrder = (String, bool);

/// What was last told to the daemon, so only a change is sent.
#[derive(Debug, Default)]
pub struct HoverWatch {
    told: Option<HoldOrder>,
}

impl HoverWatch {
    /// The order the pointer calls for now, if it is a change: a hold when it
    /// enters the panel showing `shown`, a release when it leaves. A question
    /// that was never held is never released, and none is sent without one.
    pub fn order(&self, shown: Option<&str>, inside: bool) -> Option<HoldOrder> {
        let id = shown?;
        match &self.told {
            Some((told, held)) if told == id => (*held != inside).then(|| (id.to_owned(), inside)),
            _ => inside.then(|| (id.to_owned(), true)),
        }
    }

    /// The daemon took `order`; a failed one is asked again on the next read.
    pub fn told(&mut self, order: HoldOrder) {
        self.told = Some(order);
    }
}

/// Reads the pointer while the panel shows a question, and tells the daemon
/// when it enters or leaves the panel. A failure is said and asked again.
pub fn watch<R: tauri::Runtime>(app: tauri::AppHandle<R>, client: DaemonClient) {
    tauri::async_runtime::spawn(async move {
        let mut hover = HoverWatch::default();
        let mut ticks = tokio::time::interval(POLL);
        ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            ticks.tick().await;
            let shown = SHOWN
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone();
            let inside = shown.is_some() && pointer_over_panel(&app);
            let Some((id, held)) = hover.order(shown.as_deref(), inside) else {
                continue;
            };
            let command = AppCommand::Daemon(Box::new(
                orbit_core::protocol::Command::MeetingQuestionHold {
                    id: id.clone(),
                    held,
                },
            ));
            match client.send(command).await {
                Ok(_) => hover.told((id, held)),
                Err(error) => eprintln!(
                    "No se pudo mantener la respuesta del panel flotante: {}",
                    error.code
                ),
            }
        }
    });
}

/// Whether the system's pointer is over the visible panel. The pointer comes
/// in the primary screen's pixels and the window in its own screen's, so both
/// are compared in points. Anything unreadable is «not over it».
fn pointer_over_panel<R: tauri::Runtime>(app: &tauri::AppHandle<R>) -> bool {
    let Some(window) = app.get_webview_window(crate::panel::PANEL_LABEL) else {
        return false;
    };
    if !window.is_visible().unwrap_or(false) {
        return false;
    }
    let (Ok(cursor), Ok(position), Ok(size), Ok(scale)) = (
        app.cursor_position(),
        window.outer_position(),
        window.outer_size(),
        window.scale_factor(),
    ) else {
        return false;
    };
    let primary = app
        .primary_monitor()
        .ok()
        .flatten()
        .map_or(scale, |monitor| monitor.scale_factor());
    let area = Area {
        x: f64::from(position.x) / scale,
        y: f64::from(position.y) / scale,
        width: f64::from(size.width) / scale,
        height: f64::from(size.height) / scale,
    };
    area.contains(cursor.x / primary, cursor.y / primary)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_panel_area_holds_its_edges_and_nothing_past_them() {
        let area = Area {
            x: 1_000.0,
            y: 16.0,
            width: 340.0,
            height: 300.0,
        };
        assert!(area.contains(1_000.0, 16.0));
        assert!(area.contains(1_339.9, 315.9));
        assert!(!area.contains(1_340.0, 100.0));
        assert!(!area.contains(999.9, 100.0));
        assert!(!area.contains(1_100.0, 316.0));
    }

    #[test]
    fn only_a_change_is_sent_and_a_question_never_held_is_never_released() {
        let mut hover = HoverWatch::default();
        assert_eq!(hover.order(Some("q-1"), false), None);
        assert_eq!(hover.order(None, true), None, "no question, no order");
        let hold = hover.order(Some("q-1"), true);
        assert_eq!(hold, Some(("q-1".into(), true)));
        // Not taken: asked again on the next read.
        assert_eq!(hover.order(Some("q-1"), true), hold);
        hover.told(("q-1".into(), true));
        assert_eq!(hover.order(Some("q-1"), true), None);
        assert_eq!(hover.order(Some("q-1"), false), Some(("q-1".into(), false)));
        hover.told(("q-1".into(), false));
        assert_eq!(hover.order(Some("q-1"), false), None);
    }

    #[test]
    fn a_new_question_under_the_pointer_is_held_and_the_old_one_is_left() {
        let mut hover = HoverWatch::default();
        hover.told(("q-1".into(), true));
        assert_eq!(hover.order(Some("q-2"), true), Some(("q-2".into(), true)));
        assert_eq!(hover.order(Some("q-2"), false), None);
    }
}
