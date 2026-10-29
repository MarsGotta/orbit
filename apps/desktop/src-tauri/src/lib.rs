mod application_menu;
mod commands;
mod copy;
pub mod daemon_thread;
mod events;
mod interface_flush;
mod language;
mod notify;
mod panel;
mod panel_hover;
#[cfg(feature = "pruebas")]
pub mod test_bridge;
mod tray;

use anyhow::Context;
use daemon_thread::{EmbeddedDaemon, Launch};
use orbit_app_core::{
    DEFAULT_UPDATE_BASE_URL, DaemonClient, LoginItemState, SettingsUpdateState, UpdateService,
    UpdateSnapshot,
};
use orbit_ipc::Paths;
use tauri::{Manager, RunEvent, WindowEvent};
use tauri_plugin_autostart::{MacosLauncher, ManagerExt};

pub fn run() -> anyhow::Result<()> {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        // The chat's «Copiar» writes text only; its capability allows nothing else.
        .plugin(tauri_plugin_clipboard_manager::init())
        // Only Rust posts notifications (notify.rs); the interface has no
        // permission for the plugin's commands.
        .plugin(tauri_plugin_notification::init())
        // «Adjuntar» opens macOS's panel; it answers with paths only.
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            None,
        ))
        .invoke_handler(tauri::generate_handler![
            commands::orbit_snapshot,
            commands::orbit_read_sessions_page,
            commands::orbit_read_session_messages,
            commands::orbit_agent,
            commands::orbit_vault,
            commands::orbit_send_desktop,
            commands::orbit_panel_send,
            application_menu::orbit_menu_state,
            panel::orbit_panel_resize,
            commands::orbit_report,
            commands::orbit_attachment_add,
            commands::orbit_attachment_content,
            commands::orbit_attachment_open,
            interface_flush::orbit_interface_flushed
        ])
        .manage(interface_flush::InterfaceFlush::default())
        .manage(application_menu::MenuContext::default())
        .on_menu_event(application_menu::handle_menu)
        .setup(setup)
        .on_window_event(|window, event| {
            hide_on_close(window, event);
            if let WindowEvent::ThemeChanged(theme) = event {
                tray::refresh_appearance(window.app_handle(), *theme);
            }
        })
        .build(tauri::generate_context!())
        .context("No se pudo construir la aplicación")?;
    app.run(|handle, event| {
        show_on_reopen(handle, &event);
        quit_on_exit(handle, event);
    });
    Ok(())
}

/// The Dock icon, or opening the app again from Finder, brings back the
/// window `hide_on_close` hid. macOS asks the app to reopen, and until
/// 2026-10-18 nothing answered: only the menu bar's «Abrir Orbit» showed it
/// again (seen on Orbit Pruebas). Always, not only when macOS sees no
/// visible window: the call panel floating on screen counts as one.
#[cfg(target_os = "macos")]
fn show_on_reopen<R: tauri::Runtime>(handle: &tauri::AppHandle<R>, event: &RunEvent) {
    if let RunEvent::Reopen { .. } = event {
        tray::show_window(handle);
    }
}

#[cfg(not(target_os = "macos"))]
fn show_on_reopen<R: tauri::Runtime>(_handle: &tauri::AppHandle<R>, _event: &RunEvent) {}

/// Closing the window hides it and stops nothing: a recording in progress
/// goes on, and the app stays in the menu bar (AGENTS.md).
fn hide_on_close<R: tauri::Runtime>(window: &tauri::Window<R>, event: &WindowEvent) {
    if let WindowEvent::CloseRequested { api, .. } = event {
        api.prevent_close();
        let _ = window.hide();
    }
}

/// Every way out closes the daemon here before the process ends. The tray,
/// «Salir» and an update reach `ExitRequested`, and `exit` fires it again.
/// macOS's own Quit (⌘Q, the app menu, the Dock) never does: it goes through
/// `applicationWillTerminate`, which tao turns into `Exit`, and the process
/// ends when this returns. The daemon is asked once and waited for, so
/// whichever comes first does the closure and the rest find it done.
fn quit_on_exit<R: tauri::Runtime>(handle: &tauri::AppHandle<R>, event: RunEvent) {
    if matches!(event, RunEvent::ExitRequested { .. } | RunEvent::Exit)
        && let Some(daemon) = handle.try_state::<EmbeddedDaemon>()
    {
        #[cfg(feature = "pruebas")]
        if let Some(bridge) = handle.try_state::<test_bridge::Bridge>() {
            bridge.stop();
        }
        let daemon = daemon.inner().clone();
        tauri::async_runtime::block_on(daemon.shutdown());
    }
}

fn setup(app: &mut tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    let paths = Paths::resolve(None)?;
    let executable = std::env::current_exe()
        .and_then(std::fs::canonicalize)
        .context("No se pudo localizar el ejecutable de la aplicación")?;
    let launch = Launch::for_executable(&executable)?;
    // The client spawns its tasks on Tauri's runtime, so it connects in it.
    let (daemon, client) = tauri::async_runtime::block_on(async {
        let daemon = EmbeddedDaemon::start(paths.clone(), launch).await?;
        daemon.wait_until_ready().await?;
        let transport = daemon.client().await?;
        Ok::<_, anyhow::Error>((daemon, DaemonClient::connect(paths.clone(), transport)))
    })?;
    let transport = tauri::async_runtime::block_on(daemon.client())?;
    app.manage(commands::Attachments {
        client: transport.clone(),
    });
    app.manage(commands::AppReportLog {
        log: orbit_ipc::report::ReportLog::app(&paths.root),
        client: transport,
    });
    relaunch_when_the_daemon_falls(app.handle().clone(), daemon.clone());
    app.manage(daemon);
    client.set_login_item(login_item(app));
    show_last_installation(&client);
    app.manage(language::NativeLanguage::new(
        client
            .composed_snapshot()
            .settings
            .language
            .unwrap_or_default(),
        language::preferred_languages,
    ));
    client.set_presentation_copy(copy::presentation::renderer(language::current(
        app.handle(),
    )));
    app.manage(client.clone());
    let updater = update_service();
    app.manage(updater.clone());
    #[cfg(feature = "pruebas")]
    app.manage(test_bridge::Menus::default());
    tray::install(app.handle())?;
    language::follow(app.handle(), client.composed_snapshot().settings.language)?;
    #[cfg(feature = "pruebas")]
    test_bridge::install(app.handle(), &paths.root)?;
    events::forward_frames(app.handle().clone(), client.clone());
    panel_hover::watch(app.handle().clone(), client.clone());
    tauri::async_runtime::spawn(follow_updates(paths, executable, client, updater));
    Ok(())
}

/// A daemon that falls without anyone asking (a panic, or its run failed)
/// relaunches the whole app: a fresh process starts a fresh daemon and
/// helper from the saved state, and a recording that was live goes on
/// (`cut_recovery`). A daemon that falls soon after its start is not
/// relaunched (`daemon_thread::relaunches`): the app ends and the log says
/// why. A SIGTERM ends here too, after the daemon's orderly closure, and
/// the app ends with it: that closure was asked for.
fn relaunch_when_the_daemon_falls(exits: tauri::AppHandle, watched: EmbeddedDaemon) {
    let started = std::time::Instant::now();
    tauri::async_runtime::spawn(async move {
        let ended = watched.ended().await;
        if watched.quitting() {
            return;
        }
        let uptime = started.elapsed();
        if daemon_thread::relaunches(&ended, uptime) {
            eprintln!(
                "daemon: el servicio {} sin que se pidiera salir tras {} s; Orbit se reinicia",
                ended.describe(),
                uptime.as_secs()
            );
            exits.request_restart();
        } else {
            eprintln!(
                "daemon: el servicio {} sin que se pidiera salir {} s después de arrancar; Orbit sale",
                ended.describe(),
                uptime.as_secs()
            );
            exits.exit(ended.exit_code());
        }
    });
}

/// Whether Orbit opens at login, as macOS's login items say.
fn login_item(app: &tauri::App) -> LoginItemState {
    app.autolaunch()
        .is_enabled()
        .map_or(LoginItemState::Unknown, |enabled| {
            if enabled {
                LoginItemState::On
            } else {
                LoginItemState::Off
            }
        })
}

/// Settings shows the last installation's result, or why it cannot be read.
fn show_last_installation(client: &DaemonClient) {
    let Ok(home) = std::env::var("HOME") else {
        return;
    };
    match UpdateService::last_installation_at(std::path::Path::new(&home)) {
        Ok(Some(last_installation)) => {
            client.set_update(UpdateSnapshot {
                last_installation: Some(last_installation),
                ..UpdateSnapshot::default()
            });
        }
        Ok(None) => {}
        Err(refusal) => {
            client.set_update(UpdateSnapshot {
                state: SettingsUpdateState::Failed,
                refusal: Some(refusal),
                ..UpdateSnapshot::default()
            });
        }
    }
}

/// Retires the copies an update left behind, then checks for a new version
/// every `AUTOMATIC_CHECK_INTERVAL` unless one is being checked or installed.
async fn follow_updates(
    paths: Paths,
    executable: std::path::PathBuf,
    client: DaemonClient,
    updater: UpdateService,
) {
    let log = orbit_ipc::report::LogFile::new(
        paths.root.join("logs/updates.log"),
        orbit_ipc::report::MAX_BYTES,
    );
    // Status was accepted in `setup`, after open_bootstrap and load_snapshot.
    if let Ok(home) = std::env::var("HOME")
        && let Some(bundle) =
            orbit_core::orbit_bundle::bundle_of(&executable, orbit_core::orbit_bundle::Member::App)
        && let Err(error) =
            UpdateService::retire_previous(std::path::Path::new(&home), &bundle, &log).await
    {
        eprintln!("Orbit: se conservan las copias anteriores: {error}");
    }
    loop {
        if !matches!(
            client.composed_snapshot().settings.update.state,
            SettingsUpdateState::Installing | SettingsUpdateState::Checking
        ) && let Some(result) = updater.check_automatically(&log).await
        {
            client.set_update_result(result);
        }
        tokio::time::sleep(orbit_app_core::AUTOMATIC_CHECK_INTERVAL).await;
    }
}

fn update_service() -> UpdateService {
    UpdateService::new(DEFAULT_UPDATE_BASE_URL)
}

#[cfg(test)]
mod tests {
    use super::daemon_thread::{EmbeddedDaemon, Ended, Launch};

    /// The orders a port file sends: each `invoke("orbit_…")`.
    fn sent_orders(port: &str) -> Vec<&str> {
        port.match_indices("invoke")
            .filter_map(|(at, _)| {
                let call =
                    port[at + "invoke".len()..].trim_start_matches(|c| c != '(' && c != '\n');
                let name = call.strip_prefix("(\"")?;
                name.split('"')
                    .next()
                    .filter(|name| name.starts_with("orbit_"))
            })
            .collect()
    }

    /// Each window may send every order its interface sends, through the
    /// real capabilities. In 0.12.6 `orbit_vault` was registered but not
    /// allowed: Settings › Notes failed in the app while every interface
    /// test, with a fake port, passed.
    #[test]
    fn each_window_may_send_every_order_its_interface_sends()
    -> Result<(), Box<dyn std::error::Error>> {
        // No handler: the capability is checked before an order is looked up.
        // `test`: the app's own context already embeds the macOS Info.plist.
        let app = tauri::test::mock_builder().build(tauri::generate_context!(test = true))?;
        for (label, ports) in [
            (
                "main",
                vec![
                    include_str!("../../src/bridge/tauri-port.ts"),
                    include_str!("../../src/bridge/attachments.ts"),
                ],
            ),
            (
                super::panel::PANEL_LABEL,
                vec![include_str!("../../src/bridge/panel-port.ts")],
            ),
        ] {
            let window =
                tauri::WebviewWindowBuilder::new(&app, label, tauri::WebviewUrl::default())
                    .build()?;
            let orders: Vec<&str> = ports.into_iter().flat_map(sent_orders).collect();
            assert!(!orders.is_empty(), "{label} sends no order");
            for order in orders {
                let answer = tauri::test::get_ipc_response(
                    &window,
                    tauri::webview::InvokeRequest {
                        cmd: order.into(),
                        callback: tauri::ipc::CallbackFn(0),
                        error: tauri::ipc::CallbackFn(1),
                        url: "tauri://localhost".parse()?,
                        body: tauri::ipc::InvokeBody::default(),
                        headers: Default::default(),
                        invoke_key: tauri::test::INVOKE_KEY.to_string(),
                    },
                );
                // Allowed, the order is then not found here; only the
                // capability's refusal says "not allowed".
                if let Err(error) = answer {
                    assert!(
                        !error.to_string().contains("not allowed"),
                        "{label} may not send {order}: {error}"
                    );
                }
            }
        }
        Ok(())
    }
    use orbit_ipc::Paths;
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };

    /// On Tauri's mock runtime: the app's own exit handler with the
    /// real daemon on its thread. While the app runs the daemon answers; when
    /// the last window goes, the runtime fires `ExitRequested` and the handler
    /// closes the daemon before the loop ends. Window close itself is not
    /// exercised: the mock runtime never calls window listeners (its
    /// `on_window_event` only hands out an id), so `hide_on_close` is proved
    /// on the Mac, not here; the daemon ends only through `shutdown` or a
    /// signal, and `hide_on_close` calls neither.
    #[test]
    fn exit_requested_closes_the_daemon_before_the_loop_ends()
    -> Result<(), Box<dyn std::error::Error>> {
        // Bounds a hung fixture: the app's readiness wait plus its quit wait.
        const GUARD: std::time::Duration = std::time::Duration::from_secs(15 + 17);
        let directory = tempfile::tempdir()?;
        let paths = Paths::resolve(Some(directory.path().to_path_buf()))?;
        let daemon = tauri::async_runtime::block_on(async {
            let daemon = EmbeddedDaemon::start(paths.clone(), Launch::development())
                .await
                .map_err(|error| error.to_string())?;
            daemon
                .wait_until_ready()
                .await
                .map_err(|error| error.to_string())?;
            // Past the startup recovery, which refuses `Shutdown`.
            super::daemon_thread::tests::recovered(&daemon)
                .await
                .map_err(|error| error.to_string())?;
            Ok::<_, String>(daemon)
        })?;
        let app = tauri::test::mock_builder()
            .on_window_event(super::hide_on_close)
            .manage(daemon.clone())
            .build(tauri::test::mock_context(tauri::test::noop_assets()))?;
        let window =
            tauri::WebviewWindowBuilder::new(&app, "main", tauri::WebviewUrl::default()).build()?;
        let running = Arc::new(AtomicBool::new(false));
        let ready = Arc::clone(&running);
        let probe_daemon = daemon.clone();
        let probe = std::thread::spawn(move || -> Result<(), String> {
            let deadline = std::time::Instant::now() + GUARD;
            while !ready.load(Ordering::SeqCst) {
                if std::time::Instant::now() >= deadline {
                    return Err("the mock event loop never started".into());
                }
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            let answered = tauri::async_runtime::block_on(async {
                probe_daemon
                    .client()
                    .await?
                    .request_command(orbit_core::protocol::Command::Status)
                    .await
            })
            .map_err(|error| format!("{error:#}"));
            // The last window goes: the mock runtime then asks to exit.
            window.destroy().map_err(|error| error.to_string())?;
            answered.map(|_| ())
        });
        app.run_return(move |handle, event| {
            if matches!(event, tauri::RunEvent::Ready) {
                running.store(true, Ordering::SeqCst);
            }
            super::quit_on_exit(handle, event);
        });
        // The handler waited for the closure: its socket is gone when the
        // loop is, before anything here waits.
        assert!(!paths.socket.exists());
        probe.join().map_err(|_| "the probe panicked")??;
        assert!(daemon.quitting());
        assert_eq!(
            tauri::async_runtime::block_on(daemon.ended()),
            Ended::Closed
        );
        Ok(())
    }

    /// On the Mac, macOS's own Quit (⌘Q, the app menu, the Dock)
    /// goes through `applicationWillTerminate`, which tao turns into `Exit`,
    /// never `ExitRequested` (tao 0.35, `app_delegate.rs`), and the process
    /// ends as soon as the handler returns. Measured on 2026-10-18: ⌘Q left
    /// no closure, the socket behind and CuaDriver orphaned. The handler
    /// closes the daemon on `Exit` too.
    #[test]
    fn the_systems_quit_closes_the_daemon_before_the_process_ends()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let paths = Paths::resolve(Some(directory.path().to_path_buf()))?;
        let daemon = tauri::async_runtime::block_on(async {
            let daemon = EmbeddedDaemon::start(paths.clone(), Launch::development())
                .await
                .map_err(|error| error.to_string())?;
            daemon
                .wait_until_ready()
                .await
                .map_err(|error| error.to_string())?;
            super::daemon_thread::tests::recovered(&daemon)
                .await
                .map_err(|error| error.to_string())?;
            Ok::<_, String>(daemon)
        })?;
        let app = tauri::test::mock_builder()
            .manage(daemon.clone())
            .build(tauri::test::mock_context(tauri::test::noop_assets()))?;
        super::quit_on_exit(app.handle(), tauri::RunEvent::Exit);
        assert!(!paths.socket.exists(), "the closure removes its socket");
        assert!(daemon.quitting());
        assert_eq!(
            tauri::async_runtime::block_on(daemon.ended()),
            Ended::Closed
        );
        Ok(())
    }
}
