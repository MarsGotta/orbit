//! Independent topic projections with per-topic revisions and budgets.
use super::Runtime;
use orbit_core::{
    State,
    topics::{TopicData, TopicFrame, TopicId, TopicSet},
};
use orbit_ipc::TopicChannels;
use std::collections::BTreeMap;
use tokio::sync::watch;

mod agenda;
mod assistant;
mod control;
mod notes;
mod notices;
mod permissions;
mod recording;
mod status;

pub(super) trait ProjectTopic {
    fn project(&self, source: &TopicSource<'_>) -> TopicData;
}

pub(super) struct TopicSource<'a> {
    pub(super) state: &'a State,
    pub(super) runtime: Option<&'a Runtime>,
    pub(super) now_ms: u64,
}

fn project(id: TopicId, source: &TopicSource<'_>) -> TopicData {
    match id {
        TopicId::Status => status::Projector.project(source),
        TopicId::Permissions => permissions::Projector.project(source),
        TopicId::Recording => recording::Projector.project(source),
        TopicId::Agenda => agenda::Projector.project(source),
        TopicId::Notes => notes::Projector.project(source),
        TopicId::Assistant => assistant::Projector.project(source),
        TopicId::Control => control::Projector.project(source),
        TopicId::Notices => notices::Projector.project(source),
    }
}

fn reproject_interval_ms(id: TopicId, source: &TopicSource<'_>) -> Option<u64> {
    match id {
        TopicId::Recording => {
            let active = source
                .state
                .durable
                .capture
                .recording
                .as_ref()
                .is_some_and(|recording| {
                    matches!(
                        recording.phase,
                        orbit_core::CapturePhase::Starting
                            | orbit_core::CapturePhase::Recording
                            | orbit_core::CapturePhase::Paused
                            | orbit_core::CapturePhase::Finalizing
                    )
                })
                || source.state.durable.voice.call.is_some()
                || source.runtime.is_some_and(|runtime| {
                    runtime.audio.call.is_some()
                        || runtime
                            .question
                            .current
                            .as_ref()
                            .is_some_and(|question| question.shown_until_ms().is_some())
                        || runtime.call_topic().panel.is_some()
                });
            active.then_some(id.min_interval_ms())
        }
        // The notes head uses state and runtime inputs, never the clock.
        TopicId::Notes => None,
        TopicId::Status
        | TopicId::Permissions
        | TopicId::Agenda
        | TopicId::Assistant
        | TopicId::Control
        | TopicId::Notices => None,
    }
}

struct TopicCell {
    value: TopicData,
    revision: u64,
    dirty: bool,
    last_published_ms: Option<u64>,
    last_projected_ms: Option<u64>,
    periodic_ms: Option<u64>,
    // Unchanged dirty inputs do not advance publication time; this still
    // bounds projection work by each topic's existing redraw interval.
    last_dirty_projected_ms: Option<u64>,
    sender: watch::Sender<TopicFrame>,
}

pub(super) struct TopicBoard {
    epoch: String,
    cells: BTreeMap<TopicId, TopicCell>,
}

impl TopicBoard {
    pub(super) fn new(epoch: String) -> Self {
        let cells = TopicId::ALL
            .iter()
            .copied()
            .map(|id| {
                let value = TopicData::not_observed(id);
                let (sender, receiver) = watch::channel(TopicFrame {
                    epoch: epoch.clone(),
                    revision: 0,
                    data: value.clone(),
                });
                drop(receiver);
                (
                    id,
                    TopicCell {
                        value,
                        revision: 0,
                        dirty: true,
                        last_published_ms: None,
                        last_projected_ms: None,
                        periodic_ms: None,
                        last_dirty_projected_ms: None,
                        sender,
                    },
                )
            })
            .collect();
        Self { epoch, cells }
    }

    pub(super) fn channels(&self) -> TopicChannels {
        self.cells
            .iter()
            .map(|(id, cell)| (*id, cell.sender.clone()))
            .collect()
    }

    pub(super) fn revisions(&self) -> BTreeMap<TopicId, u64> {
        self.cells
            .iter()
            .map(|(id, cell)| (*id, cell.revision))
            .collect()
    }

    pub(super) fn epoch(&self) -> &str {
        &self.epoch
    }

    /// Every handled state input may change a projection, including runtime
    /// facts outside State. Compare values at each subscribed topic's cadence
    /// rather than maintaining a second map of those dependencies.
    pub(super) fn state_changed(&mut self) {
        for cell in self.cells.values_mut() {
            cell.dirty = true;
        }
    }

    pub(super) fn next_due_ms(&self) -> Option<u64> {
        self.cells
            .iter()
            .filter(|(_, cell)| cell.sender.receiver_count() > 0)
            .filter_map(|(id, cell)| {
                let earliest_publish = cell
                    .last_published_ms
                    .map(|last| last.saturating_add(id.min_interval_ms()))
                    .unwrap_or(0);
                let dirty = cell.dirty.then_some(
                    cell.last_dirty_projected_ms
                        .map(|last| last.saturating_add(id.min_interval_ms()))
                        .unwrap_or(0)
                        .max(earliest_publish),
                );
                let periodic = cell.periodic_ms.and_then(|interval| {
                    cell.last_projected_ms
                        .map(|last| last.saturating_add(interval).max(earliest_publish))
                });
                dirty.into_iter().chain(periodic).min()
            })
            .min()
    }

    pub(super) fn publish(&mut self, source: &TopicSource<'_>) -> TopicSet {
        self.publish_mode(source, false)
    }

    pub(super) fn publish_command(&mut self, source: &TopicSource<'_>) -> TopicSet {
        self.publish_mode(source, true)
    }

    fn publish_mode(&mut self, source: &TopicSource<'_>, command: bool) -> TopicSet {
        let mut projected = TopicSet::new();
        for (id, cell) in &mut self.cells {
            if cell.sender.receiver_count() == 0 {
                continue;
            }
            let periodic = cell.periodic_ms.is_some_and(|interval| {
                cell.last_projected_ms
                    .is_some_and(|last| source.now_ms.saturating_sub(last) >= interval)
            });
            // A deadline already scheduled gets its final projection even if
            // that projection no longer needs a clock (for example an expired offer).
            cell.periodic_ms = reproject_interval_ms(*id, source);
            if !cell.dirty && !periodic {
                continue;
            }
            let interval = id.min_interval_ms();
            let published_due = cell
                .last_published_ms
                .map(|last| last.saturating_add(interval))
                .unwrap_or(0);
            let dirty_due = cell
                .last_dirty_projected_ms
                .map(|last| last.saturating_add(interval))
                .unwrap_or(0);
            // Notes starts a list read, not a command receipt. Publishing its
            // transient recording head here would keep that list stale until
            // the next one-second redraw, even after the recording finishes.
            let immediate = command && *id != TopicId::Notes;
            if !immediate
                && (source.now_ms < published_due
                    || (cell.dirty && source.now_ms < dirty_due && !periodic))
            {
                continue;
            }
            let next = orbit_ipc::bound_topic_frame(TopicFrame {
                epoch: self.epoch.clone(),
                revision: cell.revision.saturating_add(1),
                data: project(*id, source),
            })
            .data;
            cell.last_projected_ms = Some(source.now_ms);
            if cell.dirty {
                cell.last_dirty_projected_ms = Some(source.now_ms);
            }
            if cell.last_published_ms.is_some() && next == cell.value {
                cell.dirty = false;
                continue;
            }
            cell.value = next;
            cell.revision = cell.revision.saturating_add(1);
            cell.dirty = false;
            cell.last_published_ms = Some(source.now_ms);
            cell.sender.send_replace(TopicFrame {
                epoch: self.epoch.clone(),
                revision: cell.revision,
                data: cell.value.clone(),
            });
            projected.insert(*id);
        }
        projected
    }

    #[cfg(test)]
    fn recompute_all(&self, source: &TopicSource<'_>) -> BTreeMap<TopicId, TopicData> {
        TopicId::ALL
            .iter()
            .copied()
            .filter(|id| {
                self.cells
                    .get(id)
                    .is_some_and(|cell| !cell.dirty && cell.revision > 0)
            })
            .map(|id| (id, project(id, source)))
            .collect()
    }
}

#[cfg(test)]
mod board_tests;

#[cfg(test)]
mod projection_tests;

#[cfg(test)]
mod measurement_tests;

#[cfg(test)]
mod publication_tests;
