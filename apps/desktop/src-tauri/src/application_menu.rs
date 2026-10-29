//! Localized macOS application menus, preserving the native responder actions
//! (including Quit, which must reach the orderly daemon closure).
//!
//! Tauri builds menus through `run_on_main_thread`, which Tauri's mock runtime
//! runs on the test's own thread, where macOS's menu library refuses to create
//! one. Tests therefore use the stub below on every platform; the native menus
//! are checked by running the app.
#[cfg(all(target_os = "macos", not(test)))]
pub fn install<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    copy: &crate::copy::Copy,
) -> tauri::Result<()> {
    use tauri::menu::{Menu, PredefinedMenuItem as Item, Submenu};
    let label = |key: &str| {
        copy.application_menu
            .iter()
            .find(|(name, _)| *name == key)
            .map(|(_, text)| *text)
            .unwrap_or("Orbit")
    };
    let menu = Menu::with_items(
        app,
        &[
            &orbit_menu(app, copy)?,
            &Submenu::with_items(
                app,
                label("file"),
                true,
                &[&Item::close_window(app, Some(label("close")))?],
            )?,
            &Submenu::with_items(
                app,
                label("edit"),
                true,
                &[
                    &Item::undo(app, Some(label("undo")))?,
                    &Item::redo(app, Some(label("redo")))?,
                    &Item::separator(app)?,
                    &Item::cut(app, Some(label("cut")))?,
                    &Item::copy(app, Some(label("copy")))?,
                    &Item::paste(app, Some(label("paste")))?,
                    &Item::select_all(app, Some(label("select_all")))?,
                ],
            )?,
            &Submenu::with_items(
                app,
                label("view"),
                true,
                &[&Item::fullscreen(app, Some(label("fullscreen")))?],
            )?,
            &Submenu::with_id_and_items(
                app,
                tauri::menu::WINDOW_SUBMENU_ID,
                label("window"),
                true,
                &[
                    &Item::minimize(app, Some(label("minimize")))?,
                    &Item::maximize(app, Some(label("maximize")))?,
                    &Item::separator(app)?,
                    &Item::close_window(app, Some(label("close")))?,
                ],
            )?,
            &Submenu::with_id_and_items(
                app,
                tauri::menu::HELP_SUBMENU_ID,
                label("help"),
                true,
                &[],
            )?,
        ],
    )?;
    app.set_menu(menu.clone())?;
    #[cfg(feature = "pruebas")]
    crate::test_bridge::application_built(app, &menu)?;
    Ok(())
}

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

#[cfg(all(target_os = "macos", not(test)))]
fn orbit_menu<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    copy: &crate::copy::Copy,
) -> tauri::Result<tauri::menu::Submenu<R>> {
    use tauri::menu::{AboutMetadata, PredefinedMenuItem as Item, Submenu};
    let label = |key: &str| {
        copy.application_menu
            .iter()
            .find(|(name, _)| *name == key)
            .map(|(_, text)| *text)
            .unwrap_or("Orbit")
    };
    Submenu::with_items(
        app,
        "Orbit",
        true,
        &[
            &Item::about(
                app,
                Some(label("about")),
                Some(AboutMetadata {
                    name: Some("Orbit".into()),
                    version: Some(app.package_info().version.to_string()),
                    ..Default::default()
                }),
            )?,
            &Item::separator(app)?,
            &Item::services(app, Some(label("services")))?,
            &Item::separator(app)?,
            &Item::hide(app, Some(label("hide")))?,
            &Item::hide_others(app, Some(label("hide_others")))?,
            &Item::separator(app)?,
            &Item::quit(app, Some(copy.quit))?,
        ],
    )
}
