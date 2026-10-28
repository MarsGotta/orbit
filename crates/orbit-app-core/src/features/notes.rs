use super::FeatureInput;
use crate::AppSnapshot;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

pub type View = Option<crate::NotesSnapshot>;

/// The list as read (`client::notes`), with each row's day on the user's clock
/// now: a note from 23:00 moves to «Ayer» at midnight without a new read.
pub fn project(input: &FeatureInput<'_>) -> View {
    let mut view = input.localized_notes();
    if let Some(notes) = view.as_mut() {
        for item in &mut notes.list {
            item.group = crate::projection::notes::note_day_group(
                item.started_at_ms,
                input.clock.now_ms,
                &input.clock.zone,
            );
        }
    }
    view
}

pub(super) fn localize_details(notes: &mut crate::NotesSnapshot, copy: crate::PresentationCopy) {
    if let Some(index) = notes.index.as_mut() {
        copy.optional_detail(&mut index.detail);
    }
    if let Some(read) = notes.open_read.as_mut() {
        copy.optional_detail(&mut read.detail);
    }
    if let Some(open) = notes.open.as_mut() {
        localize_note(open, copy);
    }
    if let Some(library) = notes.library.as_mut() {
        copy.optional_detail(&mut library.read_detail);
        copy.optional_detail(&mut library.templates.detail);
        copy.optional_detail(&mut library.links.detail);
        if let Some(dictionaries) = library.dictionaries.as_mut() {
            copy.optional_detail(&mut dictionaries.detail);
        }
    }
    if let Some(read) = notes.series_previous.as_mut()
        && let Err(error) = &mut read.answer
    {
        *error = copy.detail(error);
    }
    if let Some(read) = notes.reminder.as_mut()
        && let Err(error) = &mut read.answer
    {
        *error = copy.detail(error);
    }
}

pub(super) fn localize_note(note: &mut crate::OpenNoteSnapshot, copy: crate::PresentationCopy) {
    copy.optional_detail(&mut note.transcript.error);
}

pub fn apply(view: &View, snapshot: &mut AppSnapshot) {
    snapshot.notes = view.clone();
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "command", rename_all = "snake_case")]
#[ts(export, export_to = "../../apps/desktop/src/bridge/generated/")]
pub enum NotesCommand {
    NotesRefresh,
    /// «Cargar más»: one more page of the hub's list.
    NotesMore,
    /// Today asks the hub for the next meeting's «Recordatorio personal».
    ReminderRead {
        series_key: String,
    },
    /// An edit of it, written at `edited_at_ms`; taken once the hub kept it
    /// (or a later one), then read again.
    ReminderSave {
        series_key: String,
        text: String,
        #[ts(type = "number")]
        edited_at_ms: i64,
    },
}

#[cfg(test)]
mod tests {
    use crate::{AppSnapshot, DaemonConnection, NotesIndexState};

    /// Before the first read of the list the app does not know whether there
    /// are notes: the list is on its way, never «no notes».
    #[test]
    fn the_list_never_read_is_on_its_way_not_empty() {
        let snapshot = AppSnapshot::empty(0, DaemonConnection::Connecting);
        let index = snapshot.notes.and_then(|notes| notes.index);
        assert_eq!(
            index.map(|index| index.state),
            Some(NotesIndexState::Loading)
        );
    }
}
