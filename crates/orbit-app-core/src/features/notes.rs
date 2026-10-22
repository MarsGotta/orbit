use super::FeatureInput;
use crate::AppSnapshot;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

pub type View = Option<crate::NotesSnapshot>;

/// The list as read (`client::notes`), with each row's day on the user's clock
/// now: a note from 23:00 moves to «Ayer» at midnight without a new read.
pub fn project(input: &FeatureInput<'_>) -> View {
    let mut view = input.local.notes.clone();
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
