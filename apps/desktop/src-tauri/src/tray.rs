use crate::daemon_thread::EmbeddedDaemon;
use orbit_app_core::{
    AppCommand, CallSnapshot, ChatSnapshot, DaemonClient, MeetingPhase, MeetingSnapshot,
};
use std::sync::{Mutex, PoisonError};
use std::time::Duration;
use tauri::{
    Manager,
    tray::{TrayIcon, TrayIconBuilder},
};

/// How often the time beside the mark moves while it records: it counts
/// seconds.
const CLOCK_TICK: Duration = Duration::from_secs(1);

/// What the menu bar shows, from the latest frames.
#[derive(Debug, Clone, PartialEq, Eq)]
struct TrayState {
    /// A recording that «Terminar grabación» can finish, timed as the
    /// sidebar's card times it.
    recording: Option<RecordedClock>,
    /// The recording can be thrown away now: it records or is paused.
    discardable: bool,
    /// «Terminar sin guardar…» was chosen and waits for its answer, in the
    /// menu itself: the first click only asks.
    asking_discard: bool,
    /// «Silenciar» (`Some(false)`) or «Reactivar micrófono» (`Some(true)`)
    /// while a recording records or a call is on. The mark does not change:
    /// the brand has only rest, recording and attention; the menu says it.
    microphone_muted: Option<bool>,
    /// An assistant waits for the user: an approval or a question.
    needs_you: bool,
    /// The system's appearance, which the menu bar follows.
    appearance: Appearance,
    update: Option<UpdateOffer>,
    language: orbit_core::language::ResolvedLanguage,
}

impl Default for TrayState {
    fn default() -> Self {
        Self {
            recording: None,
            discardable: false,
            asking_discard: false,
            microphone_muted: None,
            needs_you: false,
            appearance: Appearance::default(),
            update: None,
            language: orbit_core::language::ResolvedLanguage::English,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct UpdateOffer {
    version: String,
    /// False while something stops the install (a recording, a call):
    /// the item stays, disabled, as Settings says why.
    installable: bool,
}

/// The time recorded, as the sidebar's card counts it
/// (`src/app/recorded-time.ts`): it stops in a pause and goes on from there.
/// The daemon publishes only when the recording began, so the pauses are the
/// ones this process saw; one before Orbit opened counts as recorded time.
#[derive(Debug, Clone, PartialEq, Eq)]
struct RecordedClock {
    recording_id: Option<String>,
    started_at_ms: Option<i64>,
    /// When the pause in progress began; `None` while it records.
    paused_since_ms: Option<i64>,
    /// The pauses this process saw end.
    paused_ms: i64,
}

impl RecordedClock {
    /// What it recorded by `now_ms`; `None` until the daemon says when it began.
    fn recorded_ms(&self, now_ms: i64) -> Option<i64> {
        let started = self.started_at_ms?;
        let until = self.paused_since_ms.unwrap_or(now_ms);
        Some(
            until
                .saturating_sub(started)
                .saturating_sub(self.paused_ms)
                .max(0),
        )
    }
}

impl TrayState {
    fn follow_meeting(
        &mut self,
        meeting: &MeetingSnapshot,
        call: Option<&CallSnapshot>,
        now_ms: i64,
    ) {
        self.microphone_muted = match call {
            Some(call) => Some(call.microphone_muted),
            None => (meeting.phase == MeetingPhase::Recording).then_some(meeting.microphone_muted),
        };
        let same = self
            .recording
            .as_ref()
            .is_some_and(|clock| clock.recording_id == meeting.recording_id);
        self.discardable = matches!(
            meeting.phase,
            MeetingPhase::Recording | MeetingPhase::Paused
        );
        self.asking_discard &= same && self.discardable;
        if !finishable(&meeting.phase) {
            self.recording = None;
            return;
        }
        let mut clock = match self.recording.take() {
            Some(clock) if clock.recording_id == meeting.recording_id => clock,
            _ => RecordedClock {
                recording_id: meeting.recording_id.clone(),
                started_at_ms: None,
                paused_since_ms: None,
                paused_ms: 0,
            },
        };
        clock.started_at_ms = meeting.started_at_ms;
        let paused = meeting.phase == MeetingPhase::Paused;
        match (paused, clock.paused_since_ms) {
            (true, None) => clock.paused_since_ms = Some(now_ms),
            (false, Some(since)) => {
                clock.paused_ms = clock
                    .paused_ms
                    .saturating_add(now_ms.saturating_sub(since).max(0));
                clock.paused_since_ms = None;
            }
            _ => {}
        }
        self.recording = Some(clock);
    }

    fn follow_chat(&mut self, chat: &ChatSnapshot) {
        self.needs_you = chat.pending_contract.is_some();
    }

    fn ask_discard(&mut self) {
        self.asking_discard = self.discardable;
    }

    fn keep_recording(&mut self) {
        self.asking_discard = false;
    }
}

/// «Borrar y terminar»: the order for the recording the menu asked about.
fn discard_command(state: &TrayState) -> Option<AppCommand> {
    let recording_id = state.recording.as_ref()?.recording_id.clone()?;
    state.asking_discard.then(|| {
        AppCommand::Daemon(Box::new(orbit_core::protocol::Command::MeetingDiscard {
            recording_id,
        }))
    })
}

mod look;
#[cfg(test)]
use look::MarkImage;
use look::{Appearance, Look, look};
mod menu;
use menu::*;
#[cfg(test)]
use tauri::image::Image;

struct Shown {
    state: TrayState,
    /// A task moves the time beside the mark while it records.
    ticking: bool,
}

/// The status item and what it shows. The lock is held while the item is
/// redrawn, so two changes never reach it out of order. Every tray call
/// waits for the main thread, so the lock is only ever taken off it: a
/// theme change arrives there and is passed on (`refresh_appearance`).
struct Tray<R: tauri::Runtime> {
    icon: TrayIcon<R>,
    shown: Mutex<Shown>,
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| {
            i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX)
        })
}

/// Puts `after` on the item where it differs from `before`.
fn show<R: tauri::Runtime>(icon: &TrayIcon<R>, before: &Look, after: &Look) -> tauri::Result<()> {
    if before.image != after.image {
        icon.set_icon_with_as_template(Some(after.image.pixels()), after.image.is_template())?;
    }
    if before.title != after.title {
        // An empty title clears it: tray-icon 0.24 leaves the old one on `None`.
        icon.set_title(Some(after.title.as_str()))?;
    }
    Ok(())
}

/// Computes and applies one redraw. The state commits only after the native
/// renderer succeeds, allowing a later frame to retry a failed menu update.
fn render_change<E>(
    state: &mut TrayState,
    change: impl FnOnce(&mut TrayState, i64),
    now: i64,
    render: impl FnOnce(Option<&[Entry]>, &Look, &Look) -> Result<(), E>,
) -> Result<(), E> {
    let mut after = state.clone();
    change(&mut after, now);
    let before_entries = entries(state);
    let after_entries = entries(&after);
    let menu = (before_entries != after_entries).then_some(after_entries.as_slice());
    render(menu, &look(state, now), &look(&after, now))?;
    *state = after;
    Ok(())
}

/// Applies `change` to what the menu bar shows and redraws what changed.
fn refresh<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    change: impl FnOnce(&mut TrayState, i64),
) -> tauri::Result<()> {
    let Some(tray) = app.try_state::<Tray<R>>() else {
        return Ok(());
    };
    let now = now_ms();
    let mut shown = tray.shown.lock().unwrap_or_else(PoisonError::into_inner);
    render_change(
        &mut shown.state,
        change,
        now,
        |menu_entries, before, after| {
            if let Some(menu_entries) = menu_entries {
                tray.icon.set_menu(Some(menu(app, menu_entries)?))?;
            }
            show(&tray.icon, before, after)
        },
    )?;
    if shown.state.recording.is_some() && !shown.ticking {
        shown.ticking = true;
        tick(app.clone());
    }
    Ok(())
}

/// Moves the time beside the mark each second until the recording ends.
fn tick<R: tauri::Runtime>(app: tauri::AppHandle<R>) {
    tauri::async_runtime::spawn(async move {
        let mut second = tokio::time::interval(CLOCK_TICK);
        second.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            second.tick().await;
            let Some(tray) = app.try_state::<Tray<R>>() else {
                return;
            };
            let mut shown = tray.shown.lock().unwrap_or_else(PoisonError::into_inner);
            if shown.state.recording.is_none() {
                shown.ticking = false;
                return;
            }
            let title = look(&shown.state, now_ms()).title;
            if let Err(error) = tray.icon.set_title(Some(title.as_str())) {
                eprintln!("Orbit: no se pudo actualizar el tiempo de la barra de menús: {error}");
            }
        }
    });
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
    refresh(app, |state, _| state.update = offer)
}

pub(crate) fn refresh_language<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    language: orbit_core::language::ResolvedLanguage,
) -> tauri::Result<()> {
    refresh(app, |state, _| state.language = language)
}

pub(crate) fn refresh_recording<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    meeting: &MeetingSnapshot,
    call: Option<&CallSnapshot>,
) -> tauri::Result<()> {
    refresh(app, |state, now| state.follow_meeting(meeting, call, now))
}

pub(crate) fn refresh_assistant<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    chat: &ChatSnapshot,
) -> tauri::Result<()> {
    refresh(app, |state, _| state.follow_chat(chat))
}

/// The system's appearance changed. It arrives on the main thread, where
/// the tray's lock is never taken: the redraw goes to the async runtime.
pub(crate) fn refresh_appearance<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    theme: tauri::Theme,
) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(error) = refresh(&app, |state, _| state.appearance = theme.into()) {
            eprintln!("Orbit: no se pudo cambiar la barra de menús de apariencia: {error}");
        }
    });
}

/// Without an image the status item had no width: from the first version
/// on, its menu was in the bar and could not be seen.
pub fn install(app: &tauri::AppHandle) -> tauri::Result<()> {
    let appearance = app
        .get_webview_window("main")
        .and_then(|window| window.theme().ok())
        .map_or_else(Appearance::default, Appearance::from);
    let state = TrayState {
        appearance,
        language: crate::language::current(app),
        ..TrayState::default()
    };
    let rest = look(&state, 0).image;
    let icon = TrayIconBuilder::new()
        .icon(rest.pixels())
        .icon_as_template(rest.is_template())
        .tooltip("Orbit")
        .menu(&menu(app, &entries(&state))?)
        .on_menu_event(handle_menu)
        .build(app)?;
    app.manage(Tray {
        icon,
        shown: Mutex::new(Shown {
            state,
            ticking: false,
        }),
    });
    Ok(())
}

fn handle_menu(app: &tauri::AppHandle, event: tauri::menu::MenuEvent) {
    match event.id().as_ref() {
        OPEN => show_window(app),
        RECORD => send_meeting_command(app, recording_command()),
        FINISH_RECORDING => send_meeting_command(
            app,
            AppCommand::Daemon(Box::new(orbit_core::protocol::Command::MeetingFinish)),
        ),
        DISCARD_RECORDING => redraw(app, TrayState::ask_discard),
        KEEP_RECORDING => redraw(app, TrayState::keep_recording),
        CONFIRM_DISCARD => {
            let Some(tray) = app.try_state::<Tray<tauri::Wry>>() else {
                return;
            };
            let state = tray
                .shown
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .state
                .clone();
            redraw(app, TrayState::keep_recording);
            if let Some(command) = discard_command(&state) {
                send_meeting_command(app, command);
            }
        }
        MUTE | UNMUTE => send_meeting_command(
            app,
            AppCommand::Daemon(Box::new(orbit_core::protocol::Command::MicrophoneMute {
                muted: event.id().as_ref() == MUTE,
            })),
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

/// A change of the menu alone, from a click on it.
fn redraw(app: &tauri::AppHandle, change: fn(&mut TrayState)) {
    if let Err(error) = refresh(app, |state, _| change(state)) {
        eprintln!("Orbit: no se pudo cambiar el menú de la barra de menús: {error}");
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
mod tests;
