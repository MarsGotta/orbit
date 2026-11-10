//! `orbit_plugin` as a window calls it: the order's own arguments, the
//! daemon on its thread, and the outcome as the window reads it. Only the
//! main window may send it; the floating panel answers calls only.
use crate::daemon_thread::{EmbeddedDaemon, Launch};
use serde_json::{Value, json};

/// The window's `invoke("orbit_plugin", body)`; a rejection, as its text.
fn invoke(
    window: &tauri::WebviewWindow<tauri::test::MockRuntime>,
    body: Value,
) -> Result<tauri::ipc::InvokeResponseBody, String> {
    let url = "tauri://localhost"
        .parse::<tauri::Url>()
        .map_err(|error| error.to_string())?;
    tauri::test::get_ipc_response(
        window,
        tauri::webview::InvokeRequest {
            cmd: "orbit_plugin".into(),
            callback: tauri::ipc::CallbackFn(0),
            error: tauri::ipc::CallbackFn(1),
            url,
            body: tauri::ipc::InvokeBody::Json(body),
            headers: Default::default(),
            invoke_key: tauri::test::INVOKE_KEY.to_string(),
        },
    )
    .map_err(|error| error.to_string())
}

/// The window sends `{plugin, call}` and reads the outcome as the daemon
/// gave it: here the device's own refusal, a resolved answer with its cause.
#[test]
fn a_plugin_call_answers_the_window_with_the_daemons_outcome()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let paths = orbit_ipc::Paths::resolve(Some(directory.path().to_path_buf()))?;
    let (daemon, client) = tauri::async_runtime::block_on(async {
        let daemon = EmbeddedDaemon::start(paths.clone(), Launch::development())
            .await
            .map_err(|error| error.to_string())?;
        daemon
            .wait_until_ready()
            .await
            .map_err(|error| error.to_string())?;
        crate::daemon_thread::tests::recovered(&daemon)
            .await
            .map_err(|error| error.to_string())?;
        let local = daemon.client().await.map_err(|error| error.to_string())?;
        // The app's client starts its subscriptions on the runtime.
        let client = orbit_app_core::DaemonClient::connect(paths, local);
        Ok::<_, String>((daemon, client))
    })?;
    let app = tauri::test::mock_builder()
        .invoke_handler(tauri::generate_handler![super::orbit_plugin])
        .manage(client)
        .build(tauri::test::mock_context(tauri::test::noop_assets()))?;
    let window =
        tauri::WebviewWindowBuilder::new(&app, "main", tauri::WebviewUrl::default()).build()?;
    let answer = invoke(
        &window,
        json!({"plugin": "vault", "call": {"name": "config", "input": {}}}),
    )
    .map_err(|error| format!("orbit_plugin was rejected: {error}"))?;
    assert_eq!(
        answer.deserialize::<Value>()?,
        json!({"outcome": "failed", "refusal": {"code": "not_paired", "retry": {"kind": "after_fix"}}})
    );
    // A call that is not `{name, input}` is the frame's to refuse.
    assert!(
        invoke(
            &window,
            json!({"plugin": "vault", "call": {"name": "config", "input": []}}),
        )
        .is_err()
    );
    tauri::async_runtime::block_on(daemon.shutdown());
    Ok(())
}

/// Through the app's real capabilities: the main window may send the order,
/// the floating panel may not.
#[test]
fn only_the_main_window_may_call_a_plugin() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    // No handler: the capability is checked before an order is looked up.
    let app = tauri::test::mock_builder().build(tauri::generate_context!(test = true))?;
    for (label, allowed) in [("main", true), (crate::panel::PANEL_LABEL, false)] {
        let window = tauri::WebviewWindowBuilder::new(&app, label, tauri::WebviewUrl::default())
            .data_directory(directory.path().join(label))
            .build()?;
        let refused = invoke(&window, json!({})).is_err_and(|error| error.contains("not allowed"));
        assert_eq!(!refused, allowed, "{label}");
    }
    Ok(())
}
