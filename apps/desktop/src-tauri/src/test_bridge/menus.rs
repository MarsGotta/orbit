//! Snapshots commit only after the shell successfully builds/applies a menu.
//! Never ask the OS menu bar or inspect another application's menus.
use serde_json::{Value, json};
use std::sync::{Mutex, PoisonError};
use tauri::Manager;
use tauri::menu::{Menu, MenuItemKind};

#[derive(Default)]
pub(crate) struct Menus(Mutex<State>);
#[derive(Default)]
struct State {
    tray: Option<Value>,
    application: Option<Value>,
}

pub(crate) fn tray_built<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    title: Option<&str>,
    rows: Option<Value>,
) {
    if let Some(menus) = app.try_state::<Menus>() {
        let mut state = menus.0.lock().unwrap_or_else(PoisonError::into_inner);
        let tray = state
            .tray
            .get_or_insert_with(|| json!({"title":"","rows":[]}));
        if let Some(rows) = rows {
            tray["rows"] = rows;
        }
        if let Some(title) = title {
            tray["title"] = json!(title);
        }
    }
}
pub(crate) fn tray_clock<R: tauri::Runtime>(app: &tauri::AppHandle<R>, title: &str) {
    tray_built(app, Some(title), None);
}

pub fn application_built<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    menu: &Menu<R>,
) -> tauri::Result<()> {
    let rows = menu
        .items()?
        .iter()
        .map(row)
        .collect::<tauri::Result<Vec<_>>>()?;
    if let Some(menus) = app.try_state::<Menus>() {
        menus
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .application = Some(json!(rows));
    }
    Ok(())
}
fn row<R: tauri::Runtime>(item: &MenuItemKind<R>) -> tauri::Result<Value> {
    Ok(match item {
        MenuItemKind::MenuItem(item) => json!({"name":item.text()?,"enabled":item.is_enabled()?}),
        // Native responder items are enabled when Rust builds them. AppKit may
        // validate them later as focus changes; this is the last built state.
        MenuItemKind::Predefined(item) => {
            let name = item.text()?;
            json!({"kind":if name.is_empty() {"separator"} else {"item"},
                "enabled":!name.is_empty(),"name":name})
        }
        MenuItemKind::Submenu(item) => json!({"name":item.text()?,"enabled":item.is_enabled()?,
            "rows":item.items()?.iter().map(row).collect::<tauri::Result<Vec<_>>>()?}),
        MenuItemKind::Check(item) => json!({"name":item.text()?,"enabled":item.is_enabled()?}),
        MenuItemKind::Icon(item) => json!({"name":item.text()?,"enabled":item.is_enabled()?}),
    })
}
pub(super) fn snapshot<R: tauri::Runtime>(app: &tauri::AppHandle<R>) -> Value {
    let menus = app.state::<Menus>();
    let state = menus.0.lock().unwrap_or_else(PoisonError::into_inner);
    match (&state.tray, &state.application) {
        (Some(tray), Some(application)) => json!({"ok":true,"tray":tray,"application":application}),
        _ => {
            json!({"ok":false,"cause":"menus_not_built","tray_built":state.tray.is_some(),"application_built":state.application.is_some()})
        }
    }
}
