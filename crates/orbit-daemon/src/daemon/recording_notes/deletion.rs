//! «Borrar reunión» and «Terminar sin guardar» in the notes outbox.
//!
//! The order to the hub (`NoteOrder::Delete`) is written to the outbox
//! before anything else happens, so a restart finishes what was asked. It
//! goes first in its meeting's lane, and the lane's other orders and its
//! private notes wait behind it: once the hub deleted the meeting they are
//! moot and leave this Mac with it. A meeting still closing (sealing, or its
//! closure not yet on the hub) is deleted once it is closed.
//!
//! What this Mac keeps of the meeting goes once the hub deleted it
//! (`DeletionScope::Everything`). A thrown-away recording's audio goes
//! before that, as soon as its capture drained (`DeletionScope::Audio`), and
//! none of it is uploaded meanwhile. A hub that cannot delete (0.11) answers
//! that it does not know the route: a meeting kept on it keeps everything
//! here too and the person is told to update the hub; a thrown-away one,
//! whose audio is already gone, waits until the hub can take the order.
//! Logs carry codes and counts, never a title.
use super::{
    NoteEvent, Outcome, Runtime, check_recording, deliver,
    outbox::{Lane, Sent},
};
use crate::hub_notes::{Answer, NoteHub};
use anyhow::{Result, bail};
use orbit_core::{
    CapturePhase,
    note_outbox::{Lane as OrderLane, NoteOrder, QueuedOrder},
    notice::{Notice, NoticeCode},
    refusal::{Expected, Parameter, Refusal},
};
use orbit_store::{Deleted, DeletionScope, JobState, WaitingOrder};
use std::collections::BTreeMap;

/// Chunks deleted per store operation. A one-second chunk per track makes a
/// four-hour meeting about 29 000 of them; 256 a step keeps the writer free
/// for a new recording's seals between steps (each a few file unlinks and
/// one directory sync). It bounds a step, not what is deleted.
const PURGE_STEP_CHUNKS: usize = 256;

/// What this Mac knows of deletions now.
#[derive(Default)]
pub(in crate::daemon) struct Deletions {
    /// Whether the hub's last `ready` said it deletes recordings; `None`
    /// until a hub answered this run.
    pub(in crate::daemon) hub_deletes: Option<bool>,
    /// The meetings whose remains wait to leave this Mac, and how much of them.
    owed: BTreeMap<String, DeletionScope>,
    /// The one deletion on the store now: at most one, so a long one never
    /// stacks writes in front of a new recording's seals.
    pub(super) purging: Option<String>,
    /// What already left this Mac this run, so it is not done again.
    purged: BTreeMap<String, DeletionScope>,
    /// The next try of a deletion the store refused, by meeting.
    retry_at_ms: BTreeMap<String, i64>,
}

/// A route this hub does not have: FastAPI's 405 on a path it knows for
/// other methods, or a 404 without a code of the hub's own.
fn route_missing(status: u16, code: Option<&str>) -> bool {
    status == 405 || (status == 404 && code.is_none())
}

pub(super) async fn send_delete(hub: &NoteHub, recording: &str) -> Outcome {
    let answer: Answer<serde_json::Value> = hub
        .send(
            reqwest::Method::DELETE,
            hub.endpoint(&["v1", "recordings", recording]),
            None,
        )
        .await;
    match answer {
        Answer::Value(value) if value["deleted"] == true && value["recording_id"] == recording => {
            Outcome::Done {
                requeued: None,
                conflicts: Vec::new(),
            }
        }
        Answer::Value(_) => Outcome::Unreachable {
            cause: crate::hub_notes::Unreached::Shape,
        },
        other @ (Answer::Refused { .. } | Answer::Unreachable(_)) => other.into(),
    }
}

impl Runtime {
    /// «Borrar reunión», confirmed. A meeting being recorded is thrown
    /// away with «Terminar sin guardar» instead.
    pub(in crate::daemon) async fn delete_meeting(&mut self, recording_id: String) -> Result<()> {
        check_recording(&recording_id)?;
        if self
            .state
            .durable
            .capture
            .recording
            .as_ref()
            .is_some_and(|recording| {
                recording.id == recording_id
                    && matches!(
                        recording.phase,
                        CapturePhase::Starting | CapturePhase::Recording | CapturePhase::Paused
                    )
            })
        {
            return Err(Refusal::InvalidParameter {
                parameter: Parameter::Request,
                expected: Expected::CorrectCaptureState,
            }
            .into());
        }
        self.notes.refuse_without_hub_delete()?;
        self.queue_order(QueuedOrder::Meeting {
            recording_id,
            order: NoteOrder::Delete { discarded: false },
        })
        .await
    }

    /// «Terminar sin guardar», confirmed. The order to the hub is kept
    /// first, so a restart finishes the job; then no chunk of the recording
    /// goes up, the one on its way included, and its capture stops without
    /// sealing what drains (`sealing::seal`). Its audio leaves this Mac once
    /// the capture is over (`start_due_purge`).
    pub(in crate::daemon) async fn discard_meeting(&mut self, recording_id: String) -> Result<()> {
        check_recording(&recording_id)?;
        let event = orbit_core::Event::DiscardMeeting {
            recording_id: recording_id.clone(),
        };
        // What the core refuses is refused before anything is written.
        orbit_core::decide(&self.state, event.clone(), crate::daemon::now())?;
        self.notes.refuse_without_hub_delete()?;
        let lane = self
            .keep_order(QueuedOrder::Meeting {
                recording_id: recording_id.clone(),
                order: NoteOrder::Delete { discarded: true },
            })
            .await?;
        {
            let mut core = self.capture.lane.lock();
            core.upload.discarded.insert(recording_id.clone());
            if let Some(transfer) = core.upload_abort.take() {
                transfer.abort();
            }
        }
        if let Err(error) = self.apply(event).await {
            // It goes on recording: nothing of it may be deleted.
            let kept = self.notes.outbox.meeting_delete(&recording_id);
            if let Some(kept) = kept {
                self.forget_order(lane, kept).await?;
            }
            self.capture
                .lane
                .lock()
                .upload
                .discarded
                .remove(&recording_id);
            return Err(error);
        }
        self.maintain_recording_notes();
        Ok(())
    }

    /// Whether the meeting is still closing: being recorded or sealed
    /// here, or its closure not yet taken by the hub.
    pub(super) fn closing(&self, recording: &str) -> bool {
        if self
            .state
            .durable
            .capture
            .recording
            .as_ref()
            .is_some_and(|active| active.id == recording && active.ended_at_ms.is_none())
        {
            return true;
        }
        match self.reader.get_job(&format!("finalize-{recording}")) {
            Ok(Some(job)) => matches!(
                job.state,
                JobState::Queued | JobState::Running | JobState::RetryWait | JobState::Unknown
            ),
            Ok(None) => false,
            Err(error) => {
                self.log(
                    "meeting_delete_closure_unread",
                    &[("error", &error.to_string())],
                );
                true
            }
        }
    }

    /// The hub answered a delete. Taken: what this Mac keeps of the meeting
    /// goes too. Refused for a missing route: see the module.
    pub(super) async fn delete_answered(
        &mut self,
        lane: OrderLane,
        sent: WaitingOrder,
        outcome: Outcome,
        now_ms: i64,
    ) -> Result<()> {
        let Some((recording, discarded)) = sent
            .order
            .deletes()
            .map(|(recording, discarded)| (recording.to_owned(), discarded))
        else {
            return Ok(());
        };
        let key = Lane::Orders(lane.clone());
        match outcome {
            Outcome::Done { .. } => {
                self.log(
                    "meeting_deleted_on_hub",
                    &[("discarded", &discarded.to_string())],
                );
                self.notes.outbox.succeeded(&key);
                self.notes.owe_purge(&recording, DeletionScope::Everything);
                self.start_due_purge(now_ms);
                Ok(())
            }
            Outcome::Refused { status, code, .. } if route_missing(status, code.as_deref()) => {
                self.log(
                    "meeting_delete_needs_hub_update",
                    &[("status", &status.to_string())],
                );
                self.notes.deletions.hub_deletes = Some(false);
                let outbox = &mut self.notes.outbox;
                if discarded {
                    // Its audio is gone already: the order waits for a hub
                    // that can take it, said once.
                    outbox.failed(&key, now_ms);
                    if outbox.first_said(&key, status, code.as_deref()) {
                        self.say(Notice::new(NoticeCode::MeetingDeleteNeedsHubUpdate));
                    }
                    return Ok(());
                }
                self.forget_order(lane, sent).await?;
                self.say(Notice::new(NoticeCode::MeetingDeleteNeedsHubUpdate));
                Ok(())
            }
            refused @ Outcome::Refused { .. } => {
                // Refused for good with the hub's code, said as any order.
                // A meeting thrown away has no audio left here: the rest
                // of it goes too.
                let order = sent.order.clone();
                self.forget_order(lane, sent).await?;
                if discarded {
                    self.notes.owe_purge(&recording, DeletionScope::Everything);
                }
                if let QueuedOrder::Meeting { order, .. } = order {
                    self.order_answer(&order, refused);
                }
                Ok(())
            }
            Outcome::Unreachable { .. } => {
                self.notes.outbox.failed(&key, now_ms);
                Ok(())
            }
        }
    }

    /// The order leaves the outbox, here and in the store; the meeting is
    /// listed again.
    async fn forget_order(&mut self, lane: OrderLane, sent: WaitingOrder) -> Result<()> {
        let (lane_key, target, sequence) = (lane.key(), sent.target.clone(), sent.sequence);
        self.store_actor
            .run(move |store| Ok(store.remove_note_order(&lane_key, &target, sequence)?))
            .await?;
        let outbox = &mut self.notes.outbox;
        outbox.succeeded(&Lane::Orders(lane.clone()));
        outbox.forget(&lane, &sent);
        match &lane {
            OrderLane::Meeting(recording) => self.notes.head.touch_recording(recording),
            OrderLane::Library => self.notes.head.touch(),
        }
        Ok(())
    }

    /// Starts the next deletion this Mac owes, one at a time. A recording
    /// thrown away owes its audio once its capture is over; nothing is
    /// deleted under a capture still draining.
    pub(super) fn start_due_purge(&mut self, now_ms: i64) {
        for recording in self.notes.outbox.discarded() {
            let done = self.notes.deletions.purged.contains_key(&recording);
            if !done {
                self.notes.owe_purge(&recording, DeletionScope::Audio);
            }
        }
        // Never under a capture: the recording being drained, or any other
        // still in the state.
        let draining = self
            .state
            .durable
            .capture
            .recording
            .as_ref()
            .map(|recording| recording.id.clone());
        let deletions = &self.notes.deletions;
        if deletions.purging.is_some() {
            return;
        }
        let Some((recording, scope)) = deletions
            .owed
            .iter()
            .find(|(recording, _)| {
                Some(recording.as_str()) != draining.as_deref()
                    && deletions
                        .retry_at_ms
                        .get(*recording)
                        .is_none_or(|at| now_ms >= *at)
            })
            .map(|(recording, scope)| (recording.clone(), *scope))
        else {
            return;
        };
        self.notes.deletions.owed.remove(&recording);
        self.notes.deletions.purging = Some(recording.clone());
        let port = self.store_actor.capture_port();
        let events = self.notes.events.clone();
        let supervision = self.supervision.clone();
        let deleting = recording.clone();
        tokio::spawn(async move {
            let purge = purge_steps(port, deleting, scope);
            // A panic is a deletion that failed: it is owed again and
            // retried, as any failure of the store here is.
            let outcome = supervision
                .guard(crate::supervision::Actor::NoteDeletion, purge)
                .await
                .unwrap_or_else(|_| Err("El borrado en este Mac falló; se reintentará".into()));
            let event = NoteEvent::Purged {
                recording,
                scope,
                outcome,
            };
            deliver(&events, event).await;
        });
    }

    /// A deletion on this Mac ended.
    pub(super) fn purge_answer(
        &mut self,
        recording: String,
        scope: DeletionScope,
        outcome: std::result::Result<Deleted, String>,
        now_ms: i64,
    ) {
        self.notes.deletions.purging = None;
        match outcome {
            Ok(deleted) => {
                self.log(
                    "meeting_deleted_on_mac",
                    &[
                        (
                            "everything",
                            &(scope == DeletionScope::Everything).to_string(),
                        ),
                        ("chunks", &deleted.chunks.to_string()),
                        ("jobs", &deleted.jobs.to_string()),
                        ("records", &deleted.records.to_string()),
                        ("orders", &deleted.orders.to_string()),
                    ],
                );
                let deletions = &mut self.notes.deletions;
                let purged = deletions.purged.entry(recording.clone()).or_insert(scope);
                if scope == DeletionScope::Everything {
                    *purged = scope;
                    self.notes.outbox.forget_meeting(&recording);
                }
                self.notes.head.touch_recording(&recording);
            }
            Err(error) => {
                // Said in the log with its cause; tried again, never dropped.
                self.log("meeting_delete_on_mac_failed", &[("error", &error)]);
                let deletions = &mut self.notes.deletions;
                deletions.owed.entry(recording.clone()).or_insert(scope);
                deletions.retry_at_ms.insert(
                    recording,
                    now_ms.saturating_add(orbit_core::note_outbox::FIRST_RETRY_MS),
                );
            }
        }
        self.start_due_purge(now_ms);
    }

    /// Whether the lane's delete may go now: not while the meeting closes,
    /// nor once the hub took it and its remains are leaving this Mac.
    pub(super) fn delete_due(&self, sent: &Sent) -> bool {
        let Sent::Order(waiting) = sent else {
            return true;
        };
        let Some((recording, discarded)) = waiting.order.deletes() else {
            return true;
        };
        let deletions = &self.notes.deletions;
        if deletions.purging.as_deref() == Some(recording)
            || deletions.owed.get(recording) == Some(&DeletionScope::Everything)
        {
            return false;
        }
        discarded || !self.closing(recording)
    }
}

/// The deletion's steps, each a bounded run of the writer, until the store
/// says it is done.
async fn purge_steps(
    port: crate::daemon::store_actor::CapturePort,
    recording: String,
    scope: DeletionScope,
) -> std::result::Result<Deleted, String> {
    let mut total = Deleted::default();
    loop {
        let id = recording.clone();
        let step = match port
            .begin_run(move |store| Ok(store.delete_recording(&id, scope, PURGE_STEP_CHUNKS)?))
        {
            Ok(receipt) => receipt.wait().await,
            Err(error) => Err(error),
        };
        match step {
            Ok(step) => {
                total.chunks += step.chunks;
                total.jobs += step.jobs;
                total.records += step.records;
                total.orders += step.orders;
                if step.done {
                    return Ok(total);
                }
            }
            Err(error) => return Err(format!("{error:#}")),
        }
    }
}

impl crate::daemon::recording_notes::NotesHandle {
    /// A hub known not to delete takes no delete: nothing changes here.
    pub(in crate::daemon) fn refuse_without_hub_delete(&self) -> Result<()> {
        if self.deletions.hub_deletes == Some(false) {
            bail!("Actualiza el hub para borrar reuniones");
        }
        Ok(())
    }

    fn owe_purge(&mut self, recording: &str, scope: DeletionScope) {
        let deletions = &mut self.deletions;
        let owed = deletions.owed.entry(recording.to_owned()).or_insert(scope);
        if scope == DeletionScope::Everything {
            *owed = scope;
        }
        deletions.retry_at_ms.remove(recording);
    }
}

#[cfg(test)]
mod supervision_tests;
