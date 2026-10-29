//! Localized macOS application menus, preserving the native responder actions
//! (including Quit, which must reach the orderly daemon closure).
//!
//! Tauri builds menus through `run_on_main_thread`, which Tauri's mock runtime
//! runs on the test's own thread, where macOS's menu library refuses to create
//! one. Tests therefore use the stub below on every platform; the native menus
//! are checked by running the app.
#[path = "application_menu_state.rs"]
mod state;
use state::commands;
pub use state::{MenuContext, MenuState};
use std::sync::atomic::Ordering;
use tauri::{Emitter, Manager};

fn current<R: tauri::Runtime>(app: &tauri::AppHandle<R>) -> MenuState {
    app.try_state::<MenuContext>()
        .map(|context| {
            let mut state = context
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone();
            state.editing |= context.panel_editing.load(Ordering::Relaxed);
            state
        })
        .unwrap_or_default()
}

/// Update only when availability, focus or local presentation choices changed.
#[tauri::command]
pub fn orbit_menu_state<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    state: MenuState,
) -> Result<(), String> {
    let context = app.state::<MenuContext>();
    {
        let mut held = context
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if *held == state {
            return Ok(());
        }
        *held = state;
    }
    install(
        &app,
        crate::copy::for_language(crate::language::current(&app)),
    )
    .map_err(|error| error.to_string())
}

/// The panel's existing keyboard ownership also determines native text undo.
/// Keep it separate so a main-window update cannot overwrite the panel's focus.
pub fn panel_editing<R: tauri::Runtime>(app: &tauri::AppHandle<R>, editing: bool) {
    let Some(context) = app.try_state::<MenuContext>() else {
        return;
    };
    if context.panel_editing.swap(editing, Ordering::Relaxed) != editing
        && let Err(error) = install(
            app,
            crate::copy::for_language(crate::language::current(app)),
        )
    {
        eprintln!("Could not update text undo for the floating panel: {error}");
    }
}

pub fn handle_menu<R: tauri::Runtime>(app: &tauri::AppHandle<R>, event: tauri::menu::MenuEvent) {
    let Some(id) = event.id().as_ref().strip_prefix("orbit-menu-") else {
        return;
    };
    if !commands(&current(app))
        .iter()
        .any(|item| item.id == id && item.enabled)
    {
        return;
    }
    // A hidden main window stays alive. Show it before invoking its existing handler.
    crate::tray::show_window(app);
    if let Err(error) = app.emit_to("main", "orbit://menu-action", id) {
        eprintln!("Could not dispatch application menu action: {error}");
    }
}

// Typecheck the builder on Linux too, without constructing a native menu in tests.
#[cfg_attr(any(not(target_os = "macos"), test), allow(dead_code))]
#[path = "application_menu_native.rs"]
mod native;
#[cfg(all(target_os = "macos", not(test)))]
pub use native::install;

#[cfg(any(not(target_os = "macos"), test))]
pub fn install<R: tauri::Runtime>(
    _app: &tauri::AppHandle<R>,
    copy: &crate::copy::Copy,
) -> tauri::Result<()> {
    // The product's application menu is macOS-only; retain catalogue coverage
    // in Linux builds without creating GTK menus in the mock runtime.
    let _ = copy.application_menu;
    Ok(())
}

#[cfg(test)]
#[path = "application_menu_tests.rs"]
mod tests;
