use crate::daemon_thread::EmbeddedDaemon;
use orbit_app_core::{AppCommand, DaemonClient, MeetingPhase};
use std::sync::Mutex;
use tauri::{
    Manager,
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{TrayIcon, TrayIconBuilder},
};

const OPEN: &str = "open";
const RECORD: &str = "record";
const FINISH_RECORDING: &str = "finish_recording";
const STOP: &str = "stop";
const QUIT: &str = "quit";
const UPDATE: &str = "update";

/// What the menu bar's menu depends on, from the latest frames.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct TrayState {
    /// A recording that «Terminar grabación» can finish.
    recording: bool,
    update: Option<UpdateOffer>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct UpdateOffer {
    version: String,
    /// False while something stops the install (a recording, a call):
    /// the item stays, disabled, as Ajustes says why.
    installable: bool,
}

#[derive(Debug, PartialEq, Eq)]
enum Entry {
    Item {
        id: &'static str,
        label: String,
        enabled: bool,
    },
    Separator,
}

fn item(id: &'static str, label: impl Into<String>, enabled: bool) -> Entry {
    Entry::Item {
        id,
        label: label.into(),
        enabled,
    }
}

/// The menu, grouped as the window is: open Orbit; record or finish, only
/// the one that can be done now enabled; stop everything and the update on
/// offer; quit. Recording here starts at once, as ⌃⌥M does («Grabar
/// reunión» in the window opens «Nueva grabación» instead).
fn entries(state: &TrayState) -> Vec<Entry> {
    let mut entries = vec![
        item(OPEN, "Abrir Orbit", true),
        Entry::Separator,
        item(RECORD, "Grabar reunión ahora", !state.recording),
        item(FINISH_RECORDING, "Terminar grabación", state.recording),
        Entry::Separator,
        item(STOP, "Parar todo", true),
    ];
    if let Some(offer) = &state.update {
        entries.push(item(
            UPDATE,
            format!("Actualizar a la {}", offer.version),
            offer.installable,
        ));
    }
    entries.extend([Entry::Separator, item(QUIT, "Salir de Orbit", true)]);
    entries
}

/// Whether «Terminar grabación» has a recording to finish: also while it
/// starts or is paused, not while it is already being saved.
fn finishable(phase: &MeetingPhase) -> bool {
    matches!(
        phase,
        MeetingPhase::Starting | MeetingPhase::Recording | MeetingPhase::Paused
    )
}

fn menu<R: tauri::Runtime>(app: &tauri::AppHandle<R>, entries: &[Entry]) -> tauri::Result<Menu<R>> {
    let menu = Menu::new(app)?;
    for entry in entries {
        match entry {
            Entry::Item { id, label, enabled } => {
                menu.append(&MenuItem::with_id(app, *id, label, *enabled, None::<&str>)?)?
            }
            Entry::Separator => menu.append(&PredefinedMenuItem::separator(app)?)?,
        }
    }
    Ok(menu)
}

struct Tray<R: tauri::Runtime> {
    icon: TrayIcon<R>,
    state: Mutex<TrayState>,
}

/// Applies `change` to what the menu shows and redraws it when it changed.
fn refresh<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    change: impl FnOnce(&mut TrayState),
) -> tauri::Result<()> {
    let Some(tray) = app.try_state::<Tray<R>>() else {
        return Ok(());
    };
    let mut state = match tray.state.lock() {
        Ok(state) => state,
        Err(poisoned) => poisoned.into_inner(),
    };
    let before = state.clone();
    change(&mut state);
    if *state == before {
        return Ok(());
    }
    tray.icon.set_menu(Some(menu(app, &entries(&state))?))
}

pub(crate) fn refresh_update<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    update: &orbit_app_core::UpdateSnapshot,
) -> tauri::Result<()> {
    let offer = match (&update.state, &update.version) {
        (orbit_app_core::SettingsUpdateState::Available, Some(version)) => Some(UpdateOffer {
            version: version.clone(),
            installable: update.install_blocked.is_none(),
        }),
        _ => None,
    };
    refresh(app, |state| state.update = offer)
}

pub(crate) fn refresh_recording<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    phase: &MeetingPhase,
) -> tauri::Result<()> {
    refresh(app, |state| state.recording = finishable(phase))
}

/// The menu bar mark, in pixels: 18 points at the Mac's 2× density, the
/// height of the other marks in the bar.
const MARK_PIXELS: u32 = 36;
/// The app icon's five bars (icons/128x128.png), as heights in pixels.
const MARK_BARS: [f32; 5] = [12.0, 20.0, 28.0, 20.0, 12.0];
const MARK_BAR_WIDTH: f32 = 4.0;
const MARK_BAR_GAP: f32 = 3.0;
/// Samples per pixel side: edges anti-aliased, as macOS draws its own marks.
const MARK_SAMPLES: u32 = 4;

pub fn install(app: &tauri::AppHandle) -> tauri::Result<()> {
    let state = TrayState::default();
    let icon = TrayIconBuilder::new()
        .icon(tauri::image::Image::new_owned(
            mark_rgba(),
            MARK_PIXELS,
            MARK_PIXELS,
        ))
        .icon_as_template(true)
        .tooltip("Orbit")
        .menu(&menu(app, &entries(&state))?)
        .on_menu_event(handle_menu)
        .build(app)?;
    app.manage(Tray {
        icon,
        state: Mutex::new(state),
    });
    Ok(())
}

/// The menu bar mark: the app icon's bars, black on clear. Only the alpha
/// carries the shape, so macOS tints it as a template, like the rest of the
/// bar, in light and dark. Without an image the status item had no width:
/// from the first version on, its menu was in the bar and could not be seen.
fn mark_rgba() -> Vec<u8> {
    let side = MARK_PIXELS as f32;
    let count = MARK_BARS.len() as f32;
    let span = count * MARK_BAR_WIDTH + (count - 1.0) * MARK_BAR_GAP;
    let left = (side - span) / 2.0;
    let middle = side / 2.0;
    let radius = MARK_BAR_WIDTH / 2.0;
    let inside = |x: f32, y: f32| {
        MARK_BARS.iter().enumerate().any(|(bar, height)| {
            let centre = left + bar as f32 * (MARK_BAR_WIDTH + MARK_BAR_GAP) + radius;
            // A capsule: a vertical segment widened by the radius.
            let along = ((y - middle).abs() - (height / 2.0 - radius)).max(0.0);
            let across = x - centre;
            across * across + along * along <= radius * radius
        })
    };
    let mut rgba = vec![0_u8; (MARK_PIXELS * MARK_PIXELS * 4) as usize];
    let step = 1.0 / MARK_SAMPLES as f32;
    for y in 0..MARK_PIXELS {
        for x in 0..MARK_PIXELS {
            let mut covered = 0;
            for sy in 0..MARK_SAMPLES {
                for sx in 0..MARK_SAMPLES {
                    let px = x as f32 + (sx as f32 + 0.5) * step;
                    let py = y as f32 + (sy as f32 + 0.5) * step;
                    covered += u32::from(inside(px, py));
                }
            }
            let alpha = covered * 255 / (MARK_SAMPLES * MARK_SAMPLES);
            rgba[((y * MARK_PIXELS + x) * 4 + 3) as usize] = u8::try_from(alpha).unwrap_or(u8::MAX);
        }
    }
    rgba
}

fn handle_menu(app: &tauri::AppHandle, event: tauri::menu::MenuEvent) {
    match event.id().as_ref() {
        OPEN => show_window(app),
        RECORD => send_meeting_command(app, recording_command()),
        FINISH_RECORDING => send_meeting_command(
            app,
            AppCommand::Daemon(Box::new(orbit_core::protocol::Command::MeetingFinish)),
        ),
        STOP => stop_privacy(app),
        QUIT => quit(app),
        UPDATE => {
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                let client = app.state::<DaemonClient>();
                let daemon = app.state::<EmbeddedDaemon>();
                let updater = app.state::<orbit_app_core::UpdateService>();
                if let Err(error) =
                    crate::commands::install_update(&app, &client, &daemon, &updater).await
                {
                    eprintln!("Orbit: no se pudo actualizar: {error}");
                }
            });
        }
        _ => {}
    }
}

fn recording_command() -> AppCommand {
    AppCommand::Recording(
        orbit_app_core::features::recording::RecordingCommand::MeetingStart {
            title: String::new(),
        },
    )
}

fn send_meeting_command(app: &tauri::AppHandle, command: AppCommand) {
    let client = app.state::<DaemonClient>().inner().clone();
    tauri::async_runtime::spawn(async move {
        report_tray_result(client.send(command).await);
    });
}

fn report_tray_result(result: Result<orbit_app_core::CommandReceipt, orbit_app_core::AppError>) {
    match result {
        Ok(receipt) => {
            if let Some(orbit_app_core::ReceiptDelivery::FrameNotObserved {
                topic,
                expected_revision,
                observed_revision,
                waited_ms,
                ..
            }) = receipt.delivery
            {
                eprintln!(
                    "Orbit tray: topic {topic:?} revision {observed_revision:?} did not reach {expected_revision} after {waited_ms} ms"
                );
            }
        }
        Err(error) => eprintln!("Orbit tray command failed: {}", error.code),
    }
}

pub(crate) fn show_window<R: tauri::Runtime>(app: &tauri::AppHandle<R>) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.set_focus();
    }
}

fn stop_privacy(app: &tauri::AppHandle) {
    let client = app.state::<DaemonClient>().inner().clone();
    tauri::async_runtime::spawn(async move {
        report_tray_result(
            client
                .send(AppCommand::Daemon(Box::new(
                    orbit_core::protocol::Command::PrivacyStop,
                )))
                .await,
        );
    });
}

fn quit(app: &tauri::AppHandle) {
    let daemon = app.state::<EmbeddedDaemon>().inner().clone();
    crate::commands::quit(app.clone(), daemon);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tray_recording_delegates_its_automatic_title_to_app_core() {
        let AppCommand::Recording(
            orbit_app_core::features::recording::RecordingCommand::MeetingStart { title },
        ) = recording_command()
        else {
            panic!("the tray did not start a recording");
        };
        assert!(title.is_empty(), "only app-core reads the local clock");
    }

    fn labels(state: &TrayState) -> Vec<(String, bool)> {
        entries(state)
            .into_iter()
            .map(|entry| match entry {
                Entry::Item { label, enabled, .. } => (label, enabled),
                Entry::Separator => ("—".to_owned(), false),
            })
            .collect()
    }

    fn owned(rows: &[(&str, bool)]) -> Vec<(String, bool)> {
        rows.iter()
            .map(|(label, on)| ((*label).to_owned(), *on))
            .collect()
    }

    /// The menu bar's menu, grouped as the window is: open it; record or
    /// finish, only the one that can be done now; stop everything; quit.
    #[test]
    fn the_menu_offers_only_what_can_be_done_now() {
        assert_eq!(
            labels(&TrayState::default()),
            owned(&[
                ("Abrir Orbit", true),
                ("—", false),
                ("Grabar reunión ahora", true),
                ("Terminar grabación", false),
                ("—", false),
                ("Parar todo", true),
                ("—", false),
                ("Salir de Orbit", true),
            ])
        );
        let recording = TrayState {
            recording: true,
            update: None,
        };
        let rows = labels(&recording);
        assert_eq!(rows[2], ("Grabar reunión ahora".to_owned(), false));
        assert_eq!(rows[3], ("Terminar grabación".to_owned(), true));
    }

    #[test]
    fn a_new_version_sits_beside_parar_todo_and_says_when_it_cannot_install() {
        let offered = TrayState {
            recording: true,
            update: Some(UpdateOffer {
                version: "0.10.2".to_owned(),
                installable: false,
            }),
        };
        let rows = labels(&offered);
        assert_eq!(rows[5], ("Parar todo".to_owned(), true));
        assert_eq!(rows[6], ("Actualizar a la 0.10.2".to_owned(), false));
        assert_eq!(rows.last(), Some(&("Salir de Orbit".to_owned(), true)));
    }

    #[test]
    fn a_recording_can_be_finished_while_it_starts_or_is_paused_but_not_while_it_saves() {
        use orbit_app_core::MeetingPhase;
        assert!(finishable(&MeetingPhase::Starting));
        assert!(finishable(&MeetingPhase::Recording));
        assert!(finishable(&MeetingPhase::Paused));
        assert!(!finishable(&MeetingPhase::Finishing));
        assert!(!finishable(&MeetingPhase::Idle));
    }

    fn alpha(rgba: &[u8], x: u32, y: u32) -> u8 {
        rgba[((y * MARK_PIXELS + x) * 4 + 3) as usize]
    }

    /// A status item with no image has no width: from the first version on,
    /// the menu was in the bar and nobody could see it (seen 2026-10-18). The mark
    /// is drawn, black on clear, and only its alpha carries the shape, as a
    /// template image needs.
    #[test]
    fn the_menu_bar_mark_has_ink_margins_and_the_icons_symmetry() {
        let rgba = mark_rgba();
        assert_eq!(rgba.len(), (MARK_PIXELS * MARK_PIXELS * 4) as usize);
        let ink: u32 = rgba.chunks(4).map(|pixel| u32::from(pixel[3])).sum();
        assert!(ink > 0, "the mark has no ink");
        assert!(rgba.chunks(4).all(|pixel| pixel[..3] == [0, 0, 0]));
        let last = MARK_PIXELS - 1;
        for edge in 0..MARK_PIXELS {
            for (x, y) in [(edge, 0), (edge, last), (0, edge), (last, edge)] {
                assert_eq!(alpha(&rgba, x, y), 0, "ink on the border at {x},{y}");
            }
        }
        for y in 0..MARK_PIXELS {
            for x in 0..MARK_PIXELS {
                assert_eq!(alpha(&rgba, x, y), alpha(&rgba, last - x, y), "at {x},{y}");
            }
        }
        // The middle bar is the tallest: its column holds the most ink.
        let middle = MARK_PIXELS / 2;
        let column = |x: u32| {
            (0..MARK_PIXELS)
                .filter(|&y| alpha(&rgba, x, y) > 127)
                .count()
        };
        assert!(column(middle) > column(middle - 7));
    }
}
