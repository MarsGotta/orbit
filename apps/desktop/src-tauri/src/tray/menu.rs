//! The menu bar's menu: its items, in the order the window groups them.
use super::TrayState;
use orbit_app_core::MeetingPhase;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};

pub(super) const OPEN: &str = "open";
pub(super) const RECORD: &str = "record";
pub(super) const FINISH_RECORDING: &str = "finish_recording";
pub(super) const DISCARD_RECORDING: &str = "discard_recording";
pub(super) const DISCARD_QUESTION: &str = "discard_question";
pub(super) const CONFIRM_DISCARD: &str = "confirm_discard";
pub(super) const KEEP_RECORDING: &str = "keep_recording";
pub(super) const MUTE: &str = "microphone_mute";
pub(super) const UNMUTE: &str = "microphone_unmute";
pub(super) const STOP: &str = "stop";
pub(super) const QUIT: &str = "quit";
pub(super) const UPDATE: &str = "update";

#[derive(Debug, PartialEq, Eq)]
pub(super) enum Entry {
    Item {
        id: &'static str,
        label: String,
        enabled: bool,
    },
    Separator,
}

pub(super) fn item(id: &'static str, label: impl Into<String>, enabled: bool) -> Entry {
    Entry::Item {
        id,
        label: label.into(),
        enabled,
    }
}

/// The menu, grouped as the window is: open Orbit; record or finish, only
/// the one that can be done now enabled, and «Terminar sin guardar…», which
/// asks in the menu itself before it throws the recording away, then
/// «Silenciar» or «Reactivar micrófono» while a microphone records; stop
/// everything and the update on offer; quit. Recording here starts at once,
/// as ⌃⌥M does («Grabar reunión» in the window opens «Nueva grabación»).
pub(super) fn entries(state: &TrayState) -> Vec<Entry> {
    let copy = crate::copy::for_language(state.language);
    let recording = state.recording.is_some();
    let mut entries = vec![
        item(OPEN, copy.open, true),
        Entry::Separator,
        item(RECORD, copy.record, !recording),
        item(FINISH_RECORDING, copy.finish, recording),
    ];
    if state.asking_discard {
        entries.extend([
            item(DISCARD_QUESTION, copy.discard_question, false),
            item(CONFIRM_DISCARD, copy.confirm_discard, true),
            item(KEEP_RECORDING, copy.keep_recording, true),
        ]);
    } else {
        let discardable = recording && state.discardable;
        entries.push(item(DISCARD_RECORDING, copy.discard, discardable));
    }
    match state.microphone_muted {
        Some(false) => entries.push(item(MUTE, copy.mute, true)),
        Some(true) => entries.push(item(UNMUTE, copy.unmute, true)),
        None => {}
    }
    entries.extend([Entry::Separator, item(STOP, copy.stop, true)]);
    if let Some(offer) = &state.update {
        entries.push(item(
            UPDATE,
            crate::copy::fill(copy.update, &[("version", &offer.version)]),
            offer.installable,
        ));
    }
    entries.extend([Entry::Separator, item(QUIT, copy.quit, true)]);
    entries
}

/// Whether «Terminar grabación» has a recording to finish: also while it
/// starts or is paused, not while it is already being saved.
pub(super) fn finishable(phase: &MeetingPhase) -> bool {
    matches!(
        phase,
        MeetingPhase::Starting | MeetingPhase::Recording | MeetingPhase::Paused
    )
}

pub(super) fn menu<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    entries: &[Entry],
) -> tauri::Result<Menu<R>> {
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
