use super::*;
use tauri::Listener;

#[test]
fn application_menu_shows_main_and_dispatches_only_available_handlers()
-> Result<(), Box<dyn std::error::Error>> {
    let app = tauri::test::mock_builder()
        .manage(MenuContext::default())
        .build(tauri::test::mock_context(tauri::test::noop_assets()))?;
    let main = tauri::WebviewWindowBuilder::new(&app, "main", tauri::WebviewUrl::default())
        .visible(false)
        .build()?;
    let received = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let events = received.clone();
    main.listen("orbit://menu-action", move |event| {
        if let Ok(mut events) = events.lock() {
            events.push(event.payload().to_string());
        }
    });
    let values = || {
        received
            .lock()
            .map(|events| events.clone())
            .map_err(|_| "menu event collection is poisoned")
    };
    let select = |id: &str| {
        handle_menu(
            app.handle(),
            tauri::menu::MenuEvent {
                id: format!("orbit-menu-{id}").into(),
            },
        )
    };
    select("settings");
    assert!(main.is_visible()?);
    assert_eq!(values()?, vec!["\"settings\""]);
    select("finish");
    assert_eq!(values()?.len(), 1, "unavailable commands do not dispatch");

    let state = MenuState {
        record: true,
        new_conversation: true,
        stop: true,
        sidebar: false,
        bigger: false,
        smaller: true,
        actual_size: true,
        undo: true,
        ..Default::default()
    };
    orbit_menu_state(app.handle().clone(), state.clone())?;
    for id in [
        "record",
        "new_conversation",
        "stop",
        "sidebar",
        "smaller",
        "actual_size",
        "undo",
        "find",
        "finish",
        "bigger",
    ] {
        select(id);
    }
    assert_eq!(
        values()?,
        [
            "settings",
            "record",
            "new_conversation",
            "stop",
            "sidebar",
            "smaller",
            "actual_size",
            "undo"
        ]
        .map(|id| format!("\"{id}\""))
    );
    orbit_menu_state(
        app.handle().clone(),
        MenuState {
            editing: true,
            ..state.clone()
        },
    )?;
    select("undo");
    assert_eq!(
        values()?.len(),
        8,
        "text undo stays with the native responder"
    );
    orbit_menu_state(app.handle().clone(), state)?;
    let _panel = tauri::WebviewWindowBuilder::new(
        &app,
        crate::panel::PANEL_LABEL,
        tauri::WebviewUrl::default(),
    )
    .visible(true)
    .build()?;
    crate::panel::focus(app.handle(), true);
    select("undo");
    assert_eq!(
        values()?.len(),
        8,
        "the floating question field also keeps text undo"
    );
    crate::panel::focus(app.handle(), false);
    select("undo");
    assert_eq!(
        values()?.len(),
        9,
        "leaving the field restores template undo"
    );
    Ok(())
}
