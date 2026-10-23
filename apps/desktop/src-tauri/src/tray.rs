use crate::daemon_thread::EmbeddedDaemon;
use orbit_app_core::{AppCommand, ChatSnapshot, DaemonClient, MeetingPhase, MeetingSnapshot};
use std::sync::{Mutex, PoisonError};
use std::time::Duration;
use tauri::{
    Manager,
    image::Image,
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{TrayIcon, TrayIconBuilder},
};

const OPEN: &str = "open";
const RECORD: &str = "record";
const FINISH_RECORDING: &str = "finish_recording";
const STOP: &str = "stop";
const QUIT: &str = "quit";
const UPDATE: &str = "update";

/// How often the time beside the mark moves while it records: it counts
/// seconds.
const CLOCK_TICK: Duration = Duration::from_secs(1);

/// What the menu bar shows, from the latest frames.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct TrayState {
    /// A recording that «Terminar grabación» can finish, timed as the
    /// sidebar's card times it.
    recording: Option<RecordedClock>,
    /// An assistant waits for the user: an approval or a question.
    needs_you: bool,
    /// The system's appearance, which the menu bar follows.
    appearance: Appearance,
    update: Option<UpdateOffer>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct UpdateOffer {
    version: String,
    /// False while something stops the install (a recording, a call):
    /// the item stays, disabled, as Ajustes says why.
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
    fn follow_meeting(&mut self, meeting: &MeetingSnapshot, now_ms: i64) {
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
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum Appearance {
    #[default]
    Dark,
    Light,
}

impl From<tauri::Theme> for Appearance {
    fn from(theme: tauri::Theme) -> Self {
        match theme {
            tauri::Theme::Light => Self::Light,
            _ => Self::Dark,
        }
    }
}

/// The brand's menu bar images (`icons/menu-bar`, from the brand pack's
/// `3-barra-de-menus`), embedded in the binary. Each is the pack's `@2x`
/// file, 36 px: the status item draws its image 18 points high
/// (tray-icon 0.24, `set_icon_for_ns_status_item_button`), sharp at the
/// Mac's 2× density. Tauri takes one bitmap per icon, so the pack's 18 px
/// files have nowhere to go.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MarkImage {
    /// The small symbol in black on clear: macOS tints it as the bar's other
    /// marks, in light and dark.
    Template,
    /// The moon in red. With colour it cannot be a template: one file for a
    /// dark bar, one for a light bar.
    RecordingOnDark,
    RecordingOnLight,
    /// The moon in amber.
    NeedsYouOnDark,
    NeedsYouOnLight,
}

impl MarkImage {
    fn is_template(self) -> bool {
        self == Self::Template
    }

    fn pixels(self) -> Image<'static> {
        match self {
            Self::Template => tauri::include_image!("icons/menu-bar/orbitTemplate@2x.png"),
            Self::RecordingOnDark => {
                tauri::include_image!("icons/menu-bar/orbit-grabando-barra-oscuro@2x.png")
            }
            Self::RecordingOnLight => {
                tauri::include_image!("icons/menu-bar/orbit-grabando-barra-claro@2x.png")
            }
            Self::NeedsYouOnDark => {
                tauri::include_image!("icons/menu-bar/orbit-aviso-barra-oscuro@2x.png")
            }
            Self::NeedsYouOnLight => {
                tauri::include_image!("icons/menu-bar/orbit-aviso-barra-claro@2x.png")
            }
        }
    }
}

/// The status item: its image, and the text beside it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Look {
    image: MarkImage,
    /// Only the time recorded, while it records; empty otherwise.
    title: String,
}

/// «00:42», «12:04», «1:02:03».
fn elapsed_text(ms: i64) -> String {
    let seconds = ms.max(0) / 1_000;
    let (hours, minutes, seconds) = (seconds / 3_600, seconds / 60 % 60, seconds % 60);
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes:02}:{seconds:02}")
    }
}

/// The brand manual's three marks. A recording goes first: while it
/// records, something listens, and the time beside the mark says so. An
/// assistant that waits keeps its amber moon until the user answers, and
/// shows it again when a recording ends first.
fn look(state: &TrayState, now_ms: i64) -> Look {
    let dark = state.appearance == Appearance::Dark;
    if let Some(clock) = &state.recording {
        return Look {
            image: if dark {
                MarkImage::RecordingOnDark
            } else {
                MarkImage::RecordingOnLight
            },
            title: clock
                .recorded_ms(now_ms)
                .map(elapsed_text)
                .unwrap_or_default(),
        };
    }
    let image = match (state.needs_you, dark) {
        (false, _) => MarkImage::Template,
        (true, true) => MarkImage::NeedsYouOnDark,
        (true, false) => MarkImage::NeedsYouOnLight,
    };
    Look {
        image,
        title: String::new(),
    }
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
    let recording = state.recording.is_some();
    let mut entries = vec![
        item(OPEN, "Abrir Orbit", true),
        Entry::Separator,
        item(RECORD, "Grabar reunión ahora", !recording),
        item(FINISH_RECORDING, "Terminar grabación", recording),
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
    let before = shown.state.clone();
    change(&mut shown.state, now);
    let after = &shown.state;
    let menu_entries = entries(after);
    if entries(&before) != menu_entries {
        tray.icon.set_menu(Some(menu(app, &menu_entries)?))?;
    }
    show(&tray.icon, &look(&before, now), &look(after, now))?;
    if after.recording.is_some() && !shown.ticking {
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

pub(crate) fn refresh_recording<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    meeting: &MeetingSnapshot,
) -> tauri::Result<()> {
    refresh(app, |state, now| state.follow_meeting(meeting, now))
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
        let mut recording = TrayState::default();
        recording.follow_meeting(&meeting(MeetingPhase::Recording, "rec-1", START), START);
        let rows = labels(&recording);
        assert_eq!(rows[2], ("Grabar reunión ahora".to_owned(), false));
        assert_eq!(rows[3], ("Terminar grabación".to_owned(), true));
    }

    #[test]
    fn a_new_version_sits_beside_parar_todo_and_says_when_it_cannot_install() {
        let mut offered = TrayState {
            update: Some(UpdateOffer {
                version: "0.10.2".to_owned(),
                installable: false,
            }),
            ..TrayState::default()
        };
        offered.follow_meeting(&meeting(MeetingPhase::Recording, "rec-1", START), START);
        let rows = labels(&offered);
        assert_eq!(rows[5], ("Parar todo".to_owned(), true));
        assert_eq!(rows[6], ("Actualizar a la 0.10.2".to_owned(), false));
        assert_eq!(rows.last(), Some(&("Salir de Orbit".to_owned(), true)));
    }

    #[test]
    fn a_recording_can_be_finished_while_it_starts_or_is_paused_but_not_while_it_saves() {
        assert!(finishable(&MeetingPhase::Starting));
        assert!(finishable(&MeetingPhase::Recording));
        assert!(finishable(&MeetingPhase::Paused));
        assert!(!finishable(&MeetingPhase::Finishing));
        assert!(!finishable(&MeetingPhase::Idle));
    }

    fn meeting(phase: MeetingPhase, id: &str, started_at_ms: i64) -> MeetingSnapshot {
        let mut meeting =
            orbit_app_core::AppSnapshot::empty(1, orbit_app_core::DaemonConnection::Ready).meeting;
        meeting.phase = phase;
        meeting.recording_id = Some(id.to_owned());
        meeting.started_at_ms = Some(started_at_ms);
        meeting
    }

    fn chat(pending: Option<orbit_app_core::ChatPending>) -> ChatSnapshot {
        let mut chat =
            orbit_app_core::AppSnapshot::empty(1, orbit_app_core::DaemonConnection::Ready).chat;
        chat.pending_contract = pending;
        chat
    }

    fn approval() -> orbit_app_core::ChatPending {
        orbit_app_core::ChatPending::Approval {
            id: "approval-1".to_owned(),
            text: "Borrar un borrador".to_owned(),
            choices: vec!["once".to_owned(), "deny".to_owned()],
            deletes: None,
        }
    }

    fn rest() -> Look {
        Look {
            image: MarkImage::Template,
            title: String::new(),
        }
    }

    const START: i64 = 1_800_000_000_000;

    /// At rest the bar shows the brand's small symbol as a template, which
    /// macOS tints like its own marks, with nothing beside it.
    #[test]
    fn at_rest_the_mark_is_the_template_symbol_with_nothing_beside_it() {
        for appearance in [Appearance::Dark, Appearance::Light] {
            let state = TrayState {
                appearance,
                ..TrayState::default()
            };
            assert_eq!(look(&state, START), rest());
        }
        assert!(MarkImage::Template.is_template());
    }

    /// A recording turns the moon red, in the file for the bar's appearance,
    /// with the time recorded beside it: «mm:ss», then «h:mm:ss».
    #[test]
    fn a_recording_turns_the_moon_red_and_shows_its_time() {
        let mut state = TrayState::default();
        state.follow_meeting(&meeting(MeetingPhase::Recording, "rec-1", START), START);
        assert_eq!(
            look(&state, START + 42_000),
            Look {
                image: MarkImage::RecordingOnDark,
                title: "00:42".to_owned(),
            }
        );
        assert_eq!(look(&state, START + 724_000).title, "12:04");
        assert_eq!(look(&state, START + 3_723_000).title, "1:02:03");
        assert!(!MarkImage::RecordingOnDark.is_template());
        state.appearance = Appearance::Light;
        assert_eq!(
            look(&state, START + 42_000).image,
            MarkImage::RecordingOnLight
        );
        assert!(!MarkImage::RecordingOnLight.is_template());
    }

    /// The time stops in a pause and goes on from there, as the sidebar's
    /// card counts it.
    #[test]
    fn a_pause_stops_the_time_beside_the_mark() {
        let mut state = TrayState::default();
        state.follow_meeting(&meeting(MeetingPhase::Recording, "rec-1", START), START);
        state.follow_meeting(
            &meeting(MeetingPhase::Paused, "rec-1", START),
            START + 60_000,
        );
        assert_eq!(look(&state, START + 600_000).title, "01:00");
        state.follow_meeting(
            &meeting(MeetingPhase::Recording, "rec-1", START),
            START + 120_000,
        );
        assert_eq!(look(&state, START + 130_000).title, "01:10");
        // Another recording starts its own count.
        state.follow_meeting(
            &meeting(MeetingPhase::Recording, "rec-2", START + 200_000),
            START + 200_000,
        );
        assert_eq!(look(&state, START + 205_000).title, "00:05");
    }

    /// An assistant that waits for an approval or an answer turns the moon
    /// amber until the user answers.
    #[test]
    fn an_assistant_waiting_for_you_turns_the_moon_amber_until_you_answer() {
        let mut state = TrayState::default();
        state.follow_chat(&chat(Some(approval())));
        assert_eq!(
            look(&state, START),
            Look {
                image: MarkImage::NeedsYouOnDark,
                title: String::new(),
            }
        );
        state.appearance = Appearance::Light;
        assert_eq!(look(&state, START).image, MarkImage::NeedsYouOnLight);
        state.follow_chat(&chat(Some(orbit_app_core::ChatPending::Clarify {
            id: "clarify-1".to_owned(),
            text: "¿Qué calendario?".to_owned(),
            choices: Vec::new(),
            questions: Vec::new(),
        })));
        assert_eq!(look(&state, START).image, MarkImage::NeedsYouOnLight);
        assert!(!MarkImage::NeedsYouOnLight.is_template());
        state.follow_chat(&chat(None));
        assert_eq!(look(&state, START), rest());
    }

    /// When the recording ends the bar goes back to the template, or to the
    /// amber moon of an assistant still waiting.
    #[test]
    fn the_mark_goes_back_when_the_recording_ends() {
        let mut state = TrayState::default();
        state.follow_meeting(&meeting(MeetingPhase::Recording, "rec-1", START), START);
        state.follow_meeting(
            &meeting(MeetingPhase::Finishing, "rec-1", START),
            START + 5_000,
        );
        assert_eq!(look(&state, START + 6_000), rest());
        state.follow_meeting(&meeting(MeetingPhase::Recording, "rec-2", START), START);
        state.follow_chat(&chat(Some(approval())));
        assert_eq!(look(&state, START).image, MarkImage::RecordingOnDark);
        state.follow_meeting(&meeting(MeetingPhase::Idle, "rec-2", START), START);
        assert_eq!(
            look(&state, START),
            Look {
                image: MarkImage::NeedsYouOnDark,
                title: String::new(),
            }
        );
    }

    fn opaque(image: &Image<'_>) -> Vec<[u8; 3]> {
        image
            .rgba()
            .chunks(4)
            .filter(|pixel| pixel[3] > 200)
            .map(|pixel| [pixel[0], pixel[1], pixel[2]])
            .collect()
    }

    /// The embedded files are the pack's, 18 points at 2×: the template
    /// carries its shape only in its alpha; the recording one has a red
    /// moon and the warning one an amber moon, with the symbol in white for
    /// a dark bar and in near black for a light one.
    #[test]
    fn each_mark_is_the_brand_image_it_names() {
        let red = |[r, g, b]: [u8; 3]| r > 200 && g < 100 && b < 100;
        let amber = |[r, g, b]: [u8; 3]| r > 200 && (120..200).contains(&g) && b < 40;
        let white = |[r, g, b]: [u8; 3]| r > 240 && g > 240 && b > 240;
        let ink = |[r, g, b]: [u8; 3]| r < 40 && g < 40 && b < 40;
        for (mark, moon, symbol) in [
            (
                MarkImage::RecordingOnDark,
                &red as &dyn Fn([u8; 3]) -> bool,
                &white as &dyn Fn([u8; 3]) -> bool,
            ),
            (MarkImage::RecordingOnLight, &red, &ink),
            (MarkImage::NeedsYouOnDark, &amber, &white),
            (MarkImage::NeedsYouOnLight, &amber, &ink),
        ] {
            let image = mark.pixels();
            assert_eq!((image.width(), image.height()), (36, 36), "{mark:?}");
            let pixels = opaque(&image);
            assert!(pixels.iter().any(|pixel| moon(*pixel)), "{mark:?}: no moon");
            assert!(
                pixels.iter().any(|pixel| symbol(*pixel)),
                "{mark:?}: no symbol"
            );
        }
        let template = MarkImage::Template.pixels();
        assert_eq!((template.width(), template.height()), (36, 36));
        assert!(
            template.rgba().chunks(4).any(|pixel| pixel[3] > 0),
            "no ink"
        );
        assert!(
            template
                .rgba()
                .chunks(4)
                .all(|pixel| pixel[3] == 0 || pixel[..3] == [0, 0, 0])
        );
    }
}
