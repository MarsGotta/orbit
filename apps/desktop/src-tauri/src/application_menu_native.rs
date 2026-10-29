use super::{commands, current};

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
    let state = current(app);
    let items = build_items(app, &state, &label)?;
    let undo: tauri::menu::MenuItemKind<R> = if state.editing {
        tauri::menu::MenuItemKind::Predefined(Item::undo(app, Some(label("undo")))?)
    } else {
        tauri::menu::MenuItemKind::MenuItem(items["undo"].clone())
    };
    let menu = Menu::with_items(
        app,
        &[
            &orbit_menu(app, copy, &items["settings"])?,
            &Submenu::with_items(
                app,
                label("file"),
                true,
                &[
                    &items["record"],
                    &items["finish"],
                    &items["new_conversation"],
                    &Item::separator(app)?,
                    &items["stop"],
                    &Item::separator(app)?,
                    &Item::close_window(app, Some(label("close")))?,
                ],
            )?,
            &edit_menu(app, &label, &items, &undo)?,
            &view_menu(app, &label, &items)?,
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

fn orbit_menu<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    copy: &crate::copy::Copy,
    settings: &tauri::menu::MenuItem<R>,
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
            settings,
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

fn edit_menu<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    label: &impl Fn(&str) -> &'static str,
    items: &std::collections::HashMap<&str, tauri::menu::MenuItem<R>>,
    undo: &tauri::menu::MenuItemKind<R>,
) -> tauri::Result<tauri::menu::Submenu<R>> {
    use tauri::menu::{PredefinedMenuItem as Item, Submenu};
    Submenu::with_items(
        app,
        label("edit"),
        true,
        &[
            undo,
            &Item::redo(app, Some(label("redo")))?,
            &Item::separator(app)?,
            &Item::cut(app, Some(label("cut")))?,
            &Item::copy(app, Some(label("copy")))?,
            &Item::paste(app, Some(label("paste")))?,
            &Item::select_all(app, Some(label("select_all")))?,
            &Item::separator(app)?,
            &items["find"],
        ],
    )
}

fn view_menu<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    label: &impl Fn(&str) -> &'static str,
    items: &std::collections::HashMap<&str, tauri::menu::MenuItem<R>>,
) -> tauri::Result<tauri::menu::Submenu<R>> {
    use tauri::menu::{PredefinedMenuItem as Item, Submenu};
    Submenu::with_items(
        app,
        label("view"),
        true,
        &[
            &items["sidebar"],
            &Item::separator(app)?,
            &items["bigger"],
            &items["smaller"],
            &items["actual_size"],
            &Item::separator(app)?,
            &Item::fullscreen(app, Some(label("fullscreen")))?,
        ],
    )
}

fn build_items<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    state: &super::MenuState,
    label: &impl Fn(&str) -> &'static str,
) -> tauri::Result<std::collections::HashMap<&'static str, tauri::menu::MenuItem<R>>> {
    commands(state)
        .into_iter()
        .map(|command| {
            let accelerator = (!command.accelerator.is_empty()).then_some(command.accelerator);
            tauri::menu::MenuItem::with_id(
                app,
                format!("orbit-menu-{}", command.id),
                label(command.key),
                command.enabled,
                accelerator,
            )
            .map(|item| (command.id, item))
        })
        .collect::<tauri::Result<std::collections::HashMap<_, _>>>()
}
