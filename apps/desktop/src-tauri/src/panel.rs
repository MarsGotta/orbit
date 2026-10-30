//! The floating panel's window (`panel` in `tauri.conf.json`).
//!
//! A second webview of the same interface, small, borderless, always on top
//! and never focusable, so it can say «Llamada en Zoom» over the call without
//! taking the keyboard from it. It is shown only while the recording frame
//! has something to say (`call_panel`) or a recording is in progress, to ask
//! Deimos about it; a question to Deimos during the meeting reuses it for
//! the answers. The frames reach it by the same
//! `orbit://feature/<id>` events.

use orbit_app_core::features::FeatureFrame;
use tauri::Manager;

/// The panel window's label, named by its capability (`capabilities/panel.json`).
pub const PANEL_LABEL: &str = "panel";

/// The panel's width in logical points: the window is the design's 340 point
/// card. Mirrors `width` of the panel window in `tauri.conf.json`, which the
/// configuration test reads.
const PANEL_WIDTH: f64 = 340.0;

/// How far in from the top and right edges of the work area (below the menu
/// bar) the panel sits, in logical points, as macOS notifications do.
const PANEL_MARGIN: f64 = 16.0;

/// Initial size before the webview reports its intrinsic content height.
/// Mirrors only the initial `height` in `tauri.conf.json`, not a content limit.
#[cfg(test)]
pub const PANEL_HEIGHT: f64 = 120.0;

/// Whether the panel must be visible after `frame`: only the recording frame
/// decides it — a call to answer, a question to Deimos, or a recording in
/// progress (`ask`, recording or paused), from which to ask Deimos whether
/// a call started it or the user did.
pub fn panel_visible(frame: &FeatureFrame) -> Option<bool> {
    match frame {
        FeatureFrame::Recording { view, .. } => Some(
            view.meeting.call_panel.is_some()
                || view.meeting.question.is_some()
                || view.meeting.ask.is_some(),
        ),
        _ => None,
    }
}

/// The panel takes the keyboard only while the user writes a question;
/// otherwise it never takes it from the call. A failure is said and the
/// field stays visible.
pub fn focus<R: tauri::Runtime>(app: &tauri::AppHandle<R>, focus: bool) {
    let Some(window) = app.get_webview_window(PANEL_LABEL) else {
        return;
    };
    if let Err(error) = window.set_focusable(focus) {
        eprintln!("No se pudo cambiar el foco del panel flotante: {error}");
        return;
    }
    if focus && let Err(error) = window.set_focus() {
        eprintln!("No se pudo dar el foco al panel flotante: {error}");
    }
}

/// Fits measured content inside the visible work area, leaving the same edge
/// margins as placement. The screen, not a per-state height, limits the window.
pub fn content_height(measured: f64, work_height: u32, scale: f64) -> Option<f64> {
    if !measured.is_finite() || measured <= 0.0 || !scale.is_finite() || scale <= 0.0 {
        return None;
    }
    let available = f64::from(work_height) / scale - 2.0 * PANEL_MARGIN;
    (available >= 1.0).then(|| measured.ceil().min(available.floor()))
}

/// The panel can request only its intrinsic height. Rust owns width, screen
/// limits and placement; this command changes neither focus nor recording.
#[tauri::command]
pub fn orbit_panel_resize<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    height: f64,
) -> Result<(), String> {
    let window = app
        .get_webview_window(PANEL_LABEL)
        .ok_or("The panel window is unavailable")?;
    let monitor = window
        .current_monitor()
        .map_err(|error| error.to_string())?
        .or_else(|| window.primary_monitor().ok().flatten())
        .ok_or("The panel monitor work area is unavailable")?;
    let height = content_height(
        height,
        monitor.work_area().size.height,
        monitor.scale_factor(),
    )
    .ok_or("Panel height and monitor scale must be positive finite numbers")?;
    let current = window
        .inner_size()
        .ok()
        .zip(window.scale_factor().ok())
        .map(|(size, scale)| f64::from(size.height) / scale);
    if current.is_none_or(|current| (current - height).abs() > 0.5) {
        window
            .set_size(tauri::LogicalSize::new(PANEL_WIDTH, height))
            .map_err(|error| error.to_string())?;
    }
    Ok(())
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
pub fn apply<R: tauri::Runtime>(app: &tauri::AppHandle<R>, visible: bool) {
    let Some(window) = app.get_webview_window(PANEL_LABEL) else {
        return;
    };
    let shown = window.is_visible().unwrap_or(false);
    if !visible {
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
