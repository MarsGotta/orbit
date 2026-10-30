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
        recover_template: true,
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
        "recover_template",
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
            "recover_template"
        ]
        .map(|id| format!("\"{id}\""))
    );
    orbit_menu_state(app.handle().clone(), MenuState::default())?;
    select("recover_template");
    select("undo");
    assert_eq!(
        values()?.len(),
        8,
        "spent recovery and native Undo do not dispatch"
    );
    Ok(())
}

#[test]
fn application_edit_menu_keeps_native_history_with_and_without_panel_text_focus()
-> Result<(), Box<dyn std::error::Error>> {
    use state::{EditItem, edit_items};
    let app = tauri::test::mock_builder()
        .manage(MenuContext::default())
        .build(tauri::test::mock_context(tauri::test::noop_assets()))?;
    let _panel = tauri::WebviewWindowBuilder::new(
        &app,
        crate::panel::PANEL_LABEL,
        tauri::WebviewUrl::default(),
    )
    .build()?;
    for focused in [false, true, false] {
        crate::panel::focus(app.handle(), focused);
        // Test the same typed description that the native builder consumes.
        // Neither this read nor orbit_menu_state constructs a mock native menu.
        assert_eq!(edit_items()[0], EditItem::NativeUndo);
        assert_eq!(edit_items()[1], EditItem::NativeRedo);
        assert!(edit_items().contains(&EditItem::Command("recover_template")));
    }
    for available in [false, true, false] {
        let state = MenuState {
            recover_template: available,
            ..Default::default()
        };
        orbit_menu_state(app.handle().clone(), state)?;
        let items = commands(&current(app.handle()));
        assert!(
            !items
                .iter()
                .any(|item| item.id == "undo" || item.id == "redo")
        );
        let recovery = items
            .iter()
            .find(|item| item.id == "recover_template")
            .ok_or("recovery menu item missing")?;
        assert_eq!(recovery.enabled, available);
        assert_eq!(recovery.accelerator, "");
        assert_eq!(edit_items()[0], EditItem::NativeUndo);
    }
    Ok(())
}
