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

fn reproject_interval_ms(id: TopicId) -> Option<u64> {
    match id {
        TopicId::Recording | TopicId::Notes => Some(id.min_interval_ms()),
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
                let periodic = reproject_interval_ms(*id).and_then(|interval| {
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
            let periodic = reproject_interval_ms(*id).is_some_and(|interval| {
                cell.last_projected_ms
                    .is_some_and(|last| source.now_ms.saturating_sub(last) >= interval)
            });
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
mod tests {
    use super::*;

    #[test]
    fn oversized_topic_is_absent_before_any_transport_reads_it() -> anyhow::Result<()> {
        use orbit_core::{
            AccountSummary,
            protocol::TopicEvent,
            refusal::{Refusal, Resource},
            topics::TOPIC_ENVELOPE_BYTES,
        };
        let mut state = State::default();
        state.views.account = Some(AccountSummary {
            device_name: Some("\\".repeat(9 * 1024).into()),
            ..AccountSummary::default()
        });
        let mut board = TopicBoard::new("bounded-topic".into());
        let channels = board.channels();
        let receiver = channels[&TopicId::Status].subscribe();
        let source = TopicSource {
            state: &state,
            runtime: None,
            now_ms: 0,
        };
        let unbounded = TopicFrame {
            epoch: board.epoch().into(),
            revision: 1,
            data: project(TopicId::Status, &source),
        };
        let bytes = serde_json::to_vec(&TopicEvent::Changed { frame: unbounded })?.len() + 1;
        board.publish(&source);
        let published = receiver.borrow().clone();
        assert_eq!(published.revision, 1);
        assert!(
            published.data
                == TopicData::absent(
                    TopicId::Status,
                    Refusal::TooLarge {
                        resource: Resource::IpcResponse,
                        bytes: bytes as u64,
                        max_bytes: (TopicId::Status.max_bytes() + TOPIC_ENVELOPE_BYTES) as u64,
                    }
                )
        );
        Ok(())
    }

    #[test]
    fn thousand_audio_frames_keep_unchanged_revisions_at_the_audio_cadence() {
        let state = State::default();
        let source = |now_ms| TopicSource {
            state: &state,
            runtime: None,
            now_ms,
        };
        let mut board = TopicBoard::new("synthetic-epoch".into());
        let channels = board.channels();
        let receivers: Vec<_> = channels.values().map(watch::Sender::subscribe).collect();
        assert_eq!(board.publish(&source(0)).len(), TopicId::ALL.len());
        let mut projections = 0;
        for frame in 0..1_000 {
            board.state_changed();
            let now_ms = frame;
            let before = board.cells[&TopicId::Recording].last_projected_ms;
            assert!(board.publish(&source(now_ms)).is_empty());
            if board.cells[&TopicId::Recording].last_projected_ms != before {
                projections += 1;
            }
            for (id, expected) in board.recompute_all(&source(now_ms)) {
                assert_eq!(board.cells[&id].value, expected);
                let published = channels[&id].borrow();
                assert_eq!(published.data, expected);
                assert_eq!(published.revision, board.cells[&id].revision);
            }
        }
        assert_eq!(projections, 3);
        for sender in channels.values() {
            assert_eq!(sender.borrow().revision, 1);
        }
        assert_eq!(receivers.len(), TopicId::ALL.len());
        assert_eq!(board.revisions()[&TopicId::Recording], 1);
    }

    #[test]
    fn topics_have_independent_cells_and_epoch() {
        let state = State::default();
        let source = |now_ms| TopicSource {
            state: &state,
            runtime: None,
            now_ms,
        };
        let mut board = TopicBoard::new("one".into());
        let channels = board.channels();
        let recording = channels[&TopicId::Recording].subscribe();
        let status = channels[&TopicId::Status].subscribe();
        board.publish(&source(0));
        board.state_changed();
        board.publish(&source(250));
        assert_eq!(status.borrow().revision, 1);
        assert_eq!(recording.borrow().revision, 1);
        assert_eq!(
            board.cells[&TopicId::Recording].last_projected_ms,
            Some(250)
        );
        let next = TopicBoard::new("two".into());
        assert_ne!(
            recording.borrow().epoch,
            next.channels()[&TopicId::Recording]
                .subscribe()
                .borrow()
                .epoch
        );
    }

    #[test]
    fn periodic_recording_projection_does_not_publish_an_unchanged_value() {
        let state = State::default();
        let source = |now_ms| TopicSource {
            state: &state,
            runtime: None,
            now_ms,
        };
        let mut board = TopicBoard::new("one".into());
        let receiver = board.channels()[&TopicId::Recording].subscribe();
        assert!(board.publish(&source(0)).contains(&TopicId::Recording));
        assert!(!board.publish(&source(249)).contains(&TopicId::Recording));
        assert!(!board.publish(&source(250)).contains(&TopicId::Recording));
        assert_eq!(receiver.borrow().revision, 1);
        assert_eq!(board.cells[&TopicId::Recording].last_published_ms, Some(0));
        assert_eq!(
            board.cells[&TopicId::Recording].last_projected_ms,
            Some(250)
        );
    }

    #[test]
    fn unchanged_periodic_projection_does_not_delay_a_dirty_audio_projection() {
        let state = State::default();
        let source = |now_ms| TopicSource {
            state: &state,
            runtime: None,
            now_ms,
        };
        let mut board = TopicBoard::new("one".into());
        let _receiver = board.channels()[&TopicId::Recording].subscribe();
        board.publish(&source(0));
        assert!(!board.publish(&source(250)).contains(&TopicId::Recording));
        board.state_changed();
        assert!(!board.publish(&source(251)).contains(&TopicId::Recording));
        assert_eq!(
            board.cells[&TopicId::Recording].last_projected_ms,
            Some(251)
        );
        assert!(!board.cells[&TopicId::Recording].dirty);
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod projection_tests {
    use super::*;
    use orbit_core::{
        State,
        topics::{Knowledge, TopicData},
        voice::{CallPhase, CallState},
    };

    fn source(state: &State) -> TopicSource<'_> {
        TopicSource {
            state,
            runtime: None,
            now_ms: 0,
        }
    }

    #[test]
    fn notices_publish_every_new_notice() {
        use orbit_core::notice::NoticeCode;
        let mut state = State::default();
        let initial = project(TopicId::Notices, &source(&state));
        assert!(matches!(
            initial,
            TopicData::Notices(Knowledge::Present { .. })
        ));
        let mut board = TopicBoard::new("notices".into());
        let receiver = board.channels()[&TopicId::Notices].subscribe();
        board.publish(&source(&state));
        let initial_revision = receiver.borrow().revision;
        state.set_notice(NoticeCode::PrivacyStopped);
        board.state_changed();
        board.publish(&source(&state));
        assert!(receiver.borrow().revision > initial_revision);
        let after_stop = receiver.borrow().revision;
        state.set_notice(NoticeCode::StopStepFailed);
        board.state_changed();
        board.publish(&source(&state));
        assert!(receiver.borrow().revision > after_stop);
        let frame = serde_json::to_value(&receiver.borrow().data).unwrap();
        assert_eq!(
            frame["knowledge"]["value"]["notice"]["code"],
            "stop_step_failed"
        );
    }

    /// A notice is a code, an op and closed data, so its frame has a
    /// ceiling of its own. Every key filled with the longest token and the
    /// longest op still fits the registered frame.
    #[test]
    fn notices_frame_fits_with_every_key_at_its_longest() {
        use orbit_core::notice::{Notice, NoticeCode, NoticeKey, Op};
        let mut notice = Notice::new(NoticeCode::NoteChangeUnsent);
        for key in NoticeKey::ALL {
            notice = notice.with_token(*key, &"x".repeat(orbit_core::notice::Token::MAX_LEN));
        }
        notice.op = Op::parse(&"a".repeat(Op::MAX_LEN));
        let mut state = State::default();
        state.set_notice(notice);
        let data = project(TopicId::Notices, &source(&state));
        assert!(matches!(
            data,
            TopicData::Notices(Knowledge::Present { .. })
        ));
        let bytes = serde_json::to_vec(&data).unwrap().len();
        eprintln!(
            "notice frame: keys={} frame_bytes={bytes}",
            NoticeKey::ALL.len()
        );
        assert!(bytes <= TopicId::Notices.max_bytes());
    }

    #[test]
    fn assistant_call_is_known_and_missing_wire_defaults_to_unknown() {
        let mut state = State::default();
        let call = CallState {
            id: "synthetic".into(),
            phase: CallPhase::Starting,
            started_at_ms: 12,
            expires_at_ms: 60_012,
            turns: 0,
            origin: orbit_core::CallOrigin::App,
        };
        state.durable.call = Some(call.clone());
        let active = serde_json::to_value(project(TopicId::Assistant, &source(&state))).unwrap();
        assert_eq!(
            active["knowledge"]["value"]["call"]["value"]["phase"],
            "starting"
        );
        state.durable.call = None;
        let stopped = serde_json::to_value(project(TopicId::Assistant, &source(&state))).unwrap();
        assert_eq!(
            stopped["knowledge"]["value"]["call"]["value"],
            serde_json::Value::Null
        );
        let mut old = active["knowledge"]["value"].clone();
        old.as_object_mut().unwrap().remove("call");
        let decoded: orbit_core::topics::assistant::Value = serde_json::from_value(old).unwrap();
        let decoded = serde_json::to_value(decoded).unwrap();
        assert_eq!(decoded["call"]["state"], "unknown");

        let mut board = TopicBoard::new("call".into());
        let receiver = board.channels()[&TopicId::Assistant].subscribe();
        board.publish(&source(&state));
        let initial_revision = receiver.borrow().revision;
        state.durable.call = Some(call);
        board.state_changed();
        board.publish_command(&source(&state));
        assert!(receiver.borrow().revision > initial_revision);
        let active_revision = receiver.borrow().revision;
        state.durable.call = None;
        board.state_changed();
        board.publish_command(&source(&state));
        assert!(receiver.borrow().revision > active_revision);
        let stopped = serde_json::to_value(&receiver.borrow().data).unwrap();
        assert_eq!(
            stopped["knowledge"]["value"]["call"]["value"],
            serde_json::Value::Null
        );
    }

    #[test]
    fn command_publication_bypasses_audio_cadence() {
        let mut state = State::default();
        let mut board = TopicBoard::new("synthetic".into());
        let receiver = board.channels()[&TopicId::Recording].subscribe();
        board.publish(&source(&state));
        let before = receiver.borrow().revision;
        state.durable.pending_jobs = 1;
        board.state_changed();
        board.publish_command(&source(&state));
        assert!(
            receiver.borrow().revision > before,
            "command still waits for recording cadence"
        );
    }

    #[test]
    fn meeting_and_privacy_frames_match_command_receipt_revisions() -> anyhow::Result<()> {
        use orbit_core::{CapturePhase, Recording, protocol::MeetingAudio};
        let directory = tempfile::tempdir()?;
        let (mut runtime, _helper) = crate::daemon::audio::tests::fixture(directory.path())?;
        let mut board = TopicBoard::new("receipt-epoch".into());
        let channels = board.channels();
        let recording = channels[&TopicId::Recording].subscribe();
        let status = channels[&TopicId::Status].subscribe();
        let control = channels[&TopicId::Control].subscribe();
        board.publish(&TopicSource {
            state: &runtime.state,
            runtime: Some(&runtime),
            now_ms: 0,
        });
        runtime.state.durable.recording = Some(Recording {
            id: "synthetic-recording".into(),
            title: "Prueba".into(),
            started_at_ms: 10,
            ended_at_ms: None,
            expires_at_ms: 60_010,
            application_pid: None,
            audio: MeetingAudio::System,
            phase: CapturePhase::Starting,
            sealed_chunks: 0,
            gaps: 0,
            tracks: Vec::new(),
            finalizing_since_ms: None,
        });
        board.state_changed();
        board.state_changed();
        board.publish_command(&TopicSource {
            state: &runtime.state,
            runtime: Some(&runtime),
            now_ms: 0,
        });
        let receipt = board.revisions();
        assert_eq!(recording.borrow().revision, receipt[&TopicId::Recording]);
        let recording_json = serde_json::to_value(&recording.borrow().data)?;
        assert_eq!(
            recording_json["knowledge"]["value"]["recording"]["phase"],
            "starting"
        );

        {
            let fixture_value = serde_json::json!(true);
            crate::daemon::authority_fixture::set(
                &mut runtime.state,
                "privacy_stopped",
                fixture_value,
            );
        }
        board.state_changed();
        board.state_changed();
        board.publish_command(&TopicSource {
            state: &runtime.state,
            runtime: Some(&runtime),
            now_ms: 0,
        });
        let receipt = board.revisions();
        assert_eq!(status.borrow().revision, receipt[&TopicId::Status]);
        assert_eq!(control.borrow().revision, receipt[&TopicId::Control]);
        let status_json = serde_json::to_value(&status.borrow().data)?;
        assert_eq!(status_json["knowledge"]["value"]["privacy_stopped"], true);
        let control_json = serde_json::to_value(&control.borrow().data)?;
        assert_eq!(
            control_json["knowledge"]["value"]["engine"]["refusal"]["code"],
            "privacy_stopped"
        );
        Ok(())
    }

    #[test]
    fn quiet_notices_assistant_and_notes_publish_no_new_revisions() -> anyhow::Result<()> {
        let directory = tempfile::tempdir()?;
        let (runtime, _helper) = crate::daemon::audio::tests::fixture(directory.path())?;
        let mut board = TopicBoard::new("quiet".into());
        let channels = board.channels();
        let _receivers: Vec<_> = [TopicId::Notices, TopicId::Assistant, TopicId::Notes]
            .into_iter()
            .map(|id| channels[&id].subscribe())
            .collect();
        board.publish(&TopicSource {
            state: &runtime.state,
            runtime: Some(&runtime),
            now_ms: 0,
        });
        let before = board.revisions();
        for second in 1..=10 {
            board.publish(&TopicSource {
                state: &runtime.state,
                runtime: Some(&runtime),
                now_ms: second * 1_000,
            });
        }
        assert_eq!(board.revisions(), before);
        Ok(())
    }
}

#[cfg(test)]
mod measurement_tests;

#[cfg(test)]
mod publication_tests;
