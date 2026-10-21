//! P9-11: the floating panel's window (`panel` in `tauri.conf.json`).
//!
//! A second webview of the same interface, small, borderless, always on top
//! and never focusable, so it can say «Llamada en Zoom» over the call without
//! taking the keyboard from it. It is shown only while the recording frame
//! has something to say (`call_panel`), and P9-12 reuses it for Deimos's
//! answers. The frames reach it by the same `orbit://feature/<id>` events.

use orbit_app_core::features::FeatureFrame;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::Manager;

/// P9-12: Mars is writing a question in the panel: it keeps the keyboard
/// and the room for the field until she sends or cancels it.
static ASKING: AtomicBool = AtomicBool::new(false);

/// The panel window's label, named by its capability (`capabilities/panel.json`).
pub const PANEL_LABEL: &str = "panel";

/// The panel's width in logical points; mirrors `width` of the panel window
/// in `tauri.conf.json`, which the configuration test reads.
const PANEL_WIDTH: f64 = 360.0;

/// How far in from the top and right edges of the work area (below the menu
/// bar) the panel sits, in logical points, as macOS notifications do.
const PANEL_MARGIN: f64 = 16.0;

/// The panel's height for a call alone, in logical points; mirrors `height`
/// of the panel window in `tauri.conf.json`.
pub const PANEL_HEIGHT: f64 = 120.0;

/// P9-12: the panel's height while it shows a question to Deimos and its
/// answer: the question, a few lines of answer (it scrolls past them) and
/// «Cerrar». It is not a limit on the answer, which stays whole in the
/// conversation.
pub const PANEL_ANSWER_HEIGHT: f64 = 300.0;

/// Whether the panel must be visible after `frame`: only the recording frame
/// decides it — a call to answer (P9-11) or a question to Deimos (P9-12).
pub fn panel_visible(frame: &FeatureFrame) -> Option<bool> {
    match frame {
        FeatureFrame::Recording { view, .. } => {
            Some(view.meeting.call_panel.is_some() || view.meeting.question.is_some())
        }
        _ => None,
    }
}

/// The panel's height after `frame`.
pub fn panel_height(frame: &FeatureFrame) -> Option<f64> {
    match frame {
        FeatureFrame::Recording { view, .. } => Some(if view.meeting.question.is_some() {
            PANEL_ANSWER_HEIGHT
        } else {
            PANEL_HEIGHT
        }),
        _ => None,
    }
}

/// P9-12: the panel takes the keyboard only while Mars writes a question;
/// otherwise it never takes it from the call. A failure is said and the
/// field stays visible.
pub fn focus<R: tauri::Runtime>(app: &tauri::AppHandle<R>, focus: bool) {
    ASKING.store(focus, Ordering::Relaxed);
    let Some(window) = app.get_webview_window(PANEL_LABEL) else {
        return;
    };
    if focus
        && let Err(error) =
            window.set_size(tauri::LogicalSize::new(PANEL_WIDTH, PANEL_ANSWER_HEIGHT))
    {
        eprintln!("No se pudo ajustar el tamaño del panel flotante: {error}");
    }
    if let Err(error) = window.set_focusable(focus) {
        eprintln!("No se pudo cambiar el foco del panel flotante: {error}");
        return;
    }
    if focus && let Err(error) = window.set_focus() {
        eprintln!("No se pudo dar el foco al panel flotante: {error}");
    }
}

/// Physical pixels of a point on the desktop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Placement {
    pub x: i32,
    pub y: i32,
}

/// Where the panel goes: the top-right corner of the monitor's work area,
/// `margin` logical points in from both edges. Monitor values are physical pixels,
/// panel values logical points scaled by the monitor's `scale`.
pub fn top_right(
    monitor_x: i32,
    monitor_y: i32,
    monitor_width: u32,
    scale: f64,
    panel_width: f64,
    margin: f64,
) -> Placement {
    let right = f64::from(monitor_x) + f64::from(monitor_width);
    let x = right - (panel_width + margin) * scale;
    let y = f64::from(monitor_y) + margin * scale;
    Placement {
        x: x.round() as i32,
        y: y.round() as i32,
    }
}

/// Shows or hides the panel window. A failure is said and never stops the
/// frames: the panel is a convenience over what the main window shows.
///
/// Only a change acts: the recording frame arrives several times a second
/// with the audio meters, and showing an already visible window again would
/// order it to the front each time.
pub fn apply<R: tauri::Runtime>(app: &tauri::AppHandle<R>, visible: bool, height: f64) {
    let Some(window) = app.get_webview_window(PANEL_LABEL) else {
        return;
    };
    let shown = window.is_visible().unwrap_or(false);
    let height = if ASKING.load(Ordering::Relaxed) {
        height.max(PANEL_ANSWER_HEIGHT)
    } else {
        height
    };
    if !visible {
        ASKING.store(false, Ordering::Relaxed);
        if shown {
            if let Err(error) = window.hide() {
                eprintln!("No se pudo ocultar el panel de llamadas: {error}");
            }
            // Hidden, it gives the keyboard back.
            if let Err(error) = window.set_focusable(false) {
                eprintln!("No se pudo quitar el foco del panel flotante: {error}");
            }
        }
        return;
    }
    let size = window
        .inner_size()
        .ok()
        .zip(window.scale_factor().ok())
        .map(|(size, scale)| f64::from(size.height) / scale);
    if size.is_none_or(|current| (current - height).abs() > 0.5)
        && let Err(error) = window.set_size(tauri::LogicalSize::new(PANEL_WIDTH, height))
    {
        eprintln!("No se pudo ajustar el tamaño del panel flotante: {error}");
    }
    if shown {
        return;
    }
    let monitor = window
        .current_monitor()
        .ok()
        .flatten()
        .or_else(|| window.primary_monitor().ok().flatten());
    if let Some(monitor) = monitor {
        // The work area: the screen without the menu bar and the Dock, so
        // the panel never sits over the menu bar's status items.
        let area = monitor.work_area();
        let placement = top_right(
            area.position.x,
            area.position.y,
            area.size.width,
            monitor.scale_factor(),
            PANEL_WIDTH,
            PANEL_MARGIN,
        );
        if let Err(error) =
            window.set_position(tauri::PhysicalPosition::new(placement.x, placement.y))
        {
            eprintln!("No se pudo colocar el panel de llamadas: {error}");
        }
    }
    if let Err(error) = window.show() {
        eprintln!("No se pudo mostrar el panel de llamadas: {error}");
    }
}

#[cfg(test)]
#[path = "panel_tests.rs"]
mod tests;
