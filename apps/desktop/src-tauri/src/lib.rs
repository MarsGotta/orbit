mod commands;
pub mod daemon_thread;
mod events;
mod interface_flush;
mod notify;
mod panel;
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
            commands::orbit_send_desktop,
            commands::orbit_panel_send,
            commands::orbit_report,
            commands::orbit_attachment_add,
            commands::orbit_attachment_content,
            commands::orbit_attachment_open,
            interface_flush::orbit_interface_flushed
        ])
        .manage(interface_flush::InterfaceFlush::default())
        .setup(setup)
        .on_window_event(hide_on_close)
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
    // A daemon that ends without anyone asking (a panic, or its run ended)
    // relaunches the whole app: a fresh process starts a fresh daemon and
    // helper from the saved state, and a recording that was live goes on
    // (`cut_recovery`). A daemon that falls soon after its start is not
    // relaunched (`daemon_thread::relaunches_after`): the app ends and the
    // log says why. A SIGTERM ends here too, after the daemon's closure.
    let watched = daemon.clone();
    let exits = app.handle().clone();
    let started = std::time::Instant::now();
    tauri::async_runtime::spawn(async move {
        let ended = watched.ended().await;
        if watched.quitting() {
            return;
        }
        let uptime = started.elapsed();
        if daemon_thread::relaunches_after(uptime) {
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
    app.manage(daemon);
    let login_item = app
        .autolaunch()
        .is_enabled()
        .map_or(LoginItemState::Unknown, |enabled| {
            if enabled {
                LoginItemState::On
            } else {
                LoginItemState::Off
            }
        });
    client.set_login_item(login_item);
    if let Ok(home) = std::env::var("HOME") {
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
    app.manage(client.clone());
    let updater = update_service();
    app.manage(updater.clone());
    tray::install(app.handle())?;
    events::forward_frames(app.handle().clone(), client.clone());
    tauri::async_runtime::spawn(async move {
        let log = orbit_ipc::report::LogFile::new(
            paths.root.join("logs/updates.log"),
            orbit_ipc::report::MAX_BYTES,
        );
        // Status was accepted above, after open_bootstrap and load_snapshot.
        if let Ok(home) = std::env::var("HOME")
            && let Some(bundle) = orbit_core::orbit_bundle::bundle_of(
                &executable,
                orbit_core::orbit_bundle::Member::App,
            )
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
    });
    Ok(())
}

fn update_service() -> UpdateService {
    UpdateService::new(DEFAULT_UPDATE_BASE_URL)
}

#[cfg(test)]
mod tests {
    use super::daemon_thread::{EmbeddedDaemon, Ended, Launch};
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
