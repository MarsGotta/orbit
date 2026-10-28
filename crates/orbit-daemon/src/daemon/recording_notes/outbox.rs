//! The notes outbox at the daemon: what the person wrote waits in the store
//! until the hub took it, in lanes that never hold each other back.
//!
//! - Lanes: each meeting's orders, each meeting's private notes, and the
//!   library's orders. One request in flight per lane, at most
//!   [`MAX_LANES_IN_FLIGHT`] lanes at once.
//! - The orders live in the store's `note_outbox` table, the private notes in
//!   their own records (`orbit_store::recording_notes`); this keeps a copy of
//!   what waits so that the daemon loop never reads SQLite to schedule.
//! - A lane whose request failed waits (`note_outbox::retry_after_ms`); the
//!   hub's return sends what is owed at once.
use super::{
    NoteEvent, Runtime, deliver,
    send::{send_library_order, send_note_order, send_private_notes},
};
use crate::daemon::ticketed::{Settlement, Ticketed, WorkTicket};
use crate::hub_notes::NoteHub;
use anyhow::{Context, Result};
use orbit_core::note_outbox::{
    Lane as OrderLane, MAX_WAITING_ORDERS, NoteOrder, QueuedOrder, retry_after_ms,
};
use orbit_core::notes::LibraryRequest;
use orbit_store::WaitingOrder;
use std::collections::{BTreeMap, BTreeSet};

mod supervised;

/// Lanes with a request in flight at once. Six is what this Mac kept open
/// before the outbox (four note orders, one private notes save, one library
/// order), so the hub sees no more requests than it did; one per lane keeps
/// a meeting's orders in the order they were given.
pub(super) const MAX_LANES_IN_FLIGHT: usize = 6;

/// One lane of the outbox.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(in crate::daemon) enum Lane {
    Orders(OrderLane),
    Private(String),
}

/// What a lane has on its way to the hub.
#[derive(Debug, Clone)]
pub(super) enum Sent {
    /// The private notes as they were sent: a newer text typed meanwhile
    /// stays to be sent.
    Private(orbit_store::PendingPrivateNotes),
    Order(WaitingOrder),
}

struct LaneState {
    /// Failed requests in a row; the lane waits longer after each.
    failures: u32,
    due_ms: i64,
    /// The refusal of private notes already said, so that it is said once.
    said: Option<(u16, Option<String>)>,
    in_flight: Ticketed<Sent>,
}

impl Default for LaneState {
    fn default() -> Self {
        Self {
            failures: 0,
            due_ms: i64::MIN,
            said: None,
            in_flight: Ticketed::logical(),
        }
    }
}

#[derive(Default)]
pub(in crate::daemon) struct Outbox {
    /// Recordings whose private notes wait in the store for the hub.
    private: BTreeSet<String>,
    /// The orders waiting in the store, by lane, oldest first.
    orders: BTreeMap<OrderLane, Vec<WaitingOrder>>,
    lanes: BTreeMap<Lane, LaneState>,
}

impl Outbox {
    fn waiting(&self) -> usize {
        self.orders.values().map(Vec::len).sum()
    }

    pub(super) fn in_flight(&self) -> usize {
        self.lanes
            .values()
            .filter(|lane| lane.in_flight.active().is_some())
            .count()
    }

    /// The library's orders still on this Mac, oldest first.
    pub(super) fn library_orders(&self) -> impl Iterator<Item = &LibraryRequest> {
        self.orders
            .get(&OrderLane::Library)
            .into_iter()
            .flatten()
            .filter_map(|waiting| match &waiting.order {
                QueuedOrder::Library { request } => Some(request),
                QueuedOrder::Meeting { .. } => None,
            })
    }

    /// One meeting's orders still on this Mac, oldest first.
    pub(super) fn meeting_orders(&self, recording: &str) -> Vec<NoteOrder> {
        self.orders
            .get(&OrderLane::Meeting(recording.to_owned()))
            .into_iter()
            .flatten()
            .filter_map(|waiting| match &waiting.order {
                QueuedOrder::Meeting { order, .. } => Some(order.clone()),
                QueuedOrder::Library { .. } => None,
            })
            .collect()
    }

    /// The titles given to meetings that the hub has not taken yet; an
    /// empty one gives the meeting back its own, which only the hub knows.
    pub(super) fn pending_titles(&self) -> BTreeMap<String, String> {
        self.orders
            .values()
            .flatten()
            .filter_map(|waiting| match &waiting.order {
                QueuedOrder::Meeting {
                    recording_id,
                    order: NoteOrder::Title { title },
                } if !title.is_empty() => Some((recording_id.clone(), title.clone())),
                QueuedOrder::Meeting { .. } | QueuedOrder::Library { .. } => None,
            })
            .collect()
    }

    /// The meetings waiting to be deleted (`NoteOrder::Delete`).
    pub(in crate::daemon) fn deleting(&self) -> BTreeSet<String> {
        self.deletes().map(|(recording, _)| recording).collect()
    }

    /// The recordings thrown away while recording, whose audio is gone or
    /// going: none of it goes up.
    pub(in crate::daemon) fn discarded(&self) -> BTreeSet<String> {
        self.deletes()
            .filter(|(_, discarded)| *discarded)
            .map(|(recording, _)| recording)
            .collect()
    }

    fn deletes(&self) -> impl Iterator<Item = (String, bool)> + '_ {
        self.orders
            .values()
            .flatten()
            .filter_map(|waiting| waiting.order.deletes())
            .map(|(recording, discarded)| (recording.to_owned(), discarded))
    }

    /// The meeting's delete still on this Mac.
    pub(super) fn meeting_delete(&self, recording: &str) -> Option<WaitingOrder> {
        self.orders
            .get(&OrderLane::Meeting(recording.to_owned()))?
            .iter()
            .find(|waiting| waiting.order.deletes().is_some())
            .cloned()
    }

    /// The meeting left this Mac: nothing of it is owed any more.
    pub(super) fn forget_meeting(&mut self, recording: &str) {
        let lane = OrderLane::Meeting(recording.to_owned());
        self.orders.remove(&lane);
        self.lanes.remove(&Lane::Orders(lane));
        self.private.remove(recording);
        self.lanes.remove(&Lane::Private(recording.to_owned()));
    }

    /// Every lane with something to send, in a fixed order.
    fn owed(&self) -> Vec<Lane> {
        let mut owed = self
            .orders
            .iter()
            .filter(|(_, waiting)| !waiting.is_empty())
            .map(|(lane, _)| Lane::Orders(lane.clone()))
            .chain(self.private.iter().cloned().map(Lane::Private))
            .collect::<Vec<_>>();
        owed.sort();
        owed
    }

    pub(super) fn forget(&mut self, lane: &OrderLane, sent: &WaitingOrder) {
        if let Some(waiting) = self.orders.get_mut(lane) {
            waiting.retain(|kept| !(kept.target == sent.target && kept.sequence == sent.sequence));
        }
    }

    pub(super) fn failed(&mut self, lane: &Lane, now_ms: i64) {
        let state = self.lanes.entry(lane.clone()).or_default();
        state.failures = state.failures.saturating_add(1);
        state.due_ms = now_ms.saturating_add(retry_after_ms(state.failures));
    }

    /// The lane's request that `ticket` names, if it is the one in flight.
    pub(super) fn settle(
        &mut self,
        lane: &Lane,
        ticket: &WorkTicket,
        generation: u64,
    ) -> Settlement<Sent> {
        match self.lanes.get_mut(lane) {
            Some(state) => state.in_flight.settle(ticket, generation),
            None => Settlement::Foreign,
        }
    }

    /// The hub has a meeting's private notes; `confirmed` when the text it
    /// took is the one still kept, which then leaves the Mac.
    pub(super) fn private_delivered(&mut self, recording: &str, confirmed: bool) {
        let lane = Lane::Private(recording.to_owned());
        self.succeeded(&lane);
        if confirmed {
            self.private.remove(recording);
            self.lanes.remove(&lane);
        }
    }

    /// Whether this refusal of the lane is a new one, to be said once.
    pub(super) fn first_said(&mut self, lane: &Lane, status: u16, code: Option<&str>) -> bool {
        let state = self.lanes.entry(lane.clone()).or_default();
        let refusal = Some((status, code.map(str::to_owned)));
        if state.said == refusal {
            return false;
        }
        state.said = refusal;
        true
    }

    pub(super) fn succeeded(&mut self, lane: &Lane) {
        let state = self.lanes.entry(lane.clone()).or_default();
        state.failures = 0;
        state.due_ms = i64::MIN;
        state.said = None;
    }
}

impl Runtime {
    /// What an older Mac or an interrupted run left owed.
    pub(super) fn recover_outbox(&mut self) -> Result<()> {
        let private = self
            .reader
            .pending_private_notes()
            .context("Could not read pending private notes")?;
        let waiting = self
            .reader
            .note_orders()
            .context("Could not read pending note changes")?;
        let outbox = &mut self.notes.outbox;
        outbox.private = private.into_iter().collect();
        outbox.orders.clear();
        for order in waiting {
            outbox
                .orders
                .entry(order.order.lane())
                .or_default()
                .push(order);
        }
        Ok(())
    }

    /// A privacy stop, a revocation, the close or a new device: what is in
    /// flight is aborted and stays owed, to go after the rearm, except an
    /// order that is not sent twice (`QueuedOrder::repeatable`): whether the
    /// hub acted on it is unknown, so it leaves the outbox, in the log.
    pub(super) fn retire_outbox(&mut self) {
        let mut unsure = Vec::new();
        for (lane, state) in &mut self.notes.outbox.lanes {
            if let (Lane::Orders(order_lane), Some(Sent::Order(sent))) =
                (lane, state.in_flight.active())
                && !sent.order.repeatable()
            {
                unsure.push((order_lane.clone(), sent.clone()));
            }
            state.in_flight.cancel();
            state.due_ms = i64::MIN;
        }
        for (lane, sent) in unsure {
            self.notes.outbox.forget(&lane, &sent);
            let (key, target, sequence) = (lane.key(), sent.target.clone(), sent.sequence);
            if let Err(error) = self
                .store_actor
                .begin_run(move |store| Ok(store.remove_note_order(&key, &target, sequence)?))
            {
                self.log("note_order_unforgotten", &[("error", &error.to_string())]);
            }
            let name = match &sent.order {
                QueuedOrder::Meeting { order, .. } => order.name(),
                QueuedOrder::Library { .. } => "library",
            };
            self.log("note_order_cancelled_unconfirmed", &[("order", name)]);
            if let QueuedOrder::Library { request } = &sent.order {
                let library = &mut self.notes.library;
                super::library::refuse_order(library, request, 0, "not_sent", BTreeMap::new());
                self.notes.head.touch_library();
            }
        }
    }

    /// Keeps `order` until the hub takes it, and sends it now if its lane is
    /// free. Refused when the outbox is full, with both numbers.
    pub(super) async fn queue_order(&mut self, order: QueuedOrder) -> Result<()> {
        let lane = self.keep_order(order).await?;
        if let Some(hub) = self.note_hub() {
            let now_ms = self.notes.notes_now();
            self.start_lane(&hub, Lane::Orders(lane), now_ms);
        }
        Ok(())
    }

    /// Keeps `order` in the store and in the outbox, without sending it.
    pub(in crate::daemon) async fn keep_order(&mut self, order: QueuedOrder) -> Result<OrderLane> {
        let lane = order.lane();
        let target = order
            .target()
            .unwrap_or_else(|| format!("dictionary:{}", crate::daemon::id()));
        let outbox = &self.notes.outbox;
        let waiting = outbox.waiting();
        let replaces = outbox
            .orders
            .get(&lane)
            .is_some_and(|orders| orders.iter().any(|kept| kept.target == target));
        if !replaces && waiting >= MAX_WAITING_ORDERS {
            return Err(orbit_core::refusal::Refusal::WorkInFlight {
                in_flight: waiting as u64,
                max: MAX_WAITING_ORDERS as u64,
            }
            .into());
        }
        let now_ms = self.notes.notes_now();
        let queued = self
            .store_actor
            .run(move |store| Ok(store.queue_note_order(&target, order, now_ms)?))
            .await
            .context("Could not save the change on this Mac")?;
        let notes = &mut self.notes;
        let orders = notes.outbox.orders.entry(lane.clone()).or_default();
        orders.retain(|kept| kept.target != queued.target);
        orders.push(queued);
        match &lane {
            OrderLane::Library => notes.head.touch_library(),
            OrderLane::Meeting(recording) => notes.head.touch_recording(recording),
        }
        Ok(lane)
    }

    pub(in crate::daemon) async fn note_private_save(
        &mut self,
        recording_id: String,
        markdown: String,
        base_revision: u64,
    ) -> Result<()> {
        super::check_recording(&recording_id)?;
        orbit_core::notes::check_private_notes(&markdown)?;
        let now_ms = self.notes.notes_now();
        let pending = orbit_store::PendingPrivateNotes {
            recording_id: recording_id.clone(),
            markdown,
            base_revision,
        };
        self.store_actor
            .run(move |store| Ok(store.save_private_notes(&pending, now_ms)?))
            .await
            .context("Could not save private notes")?;
        // They go with the next turn of the outbox, at once.
        let outbox = &mut self.notes.outbox;
        outbox.private.insert(recording_id.clone());
        let lane = Lane::Private(recording_id);
        outbox.lanes.entry(lane).or_default().due_ms = i64::MIN;
        Ok(())
    }

    /// Starts every lane that is due. Returns how many requests went out.
    pub(super) fn start_due_lanes(&mut self, hub: &NoteHub, now_ms: i64) -> usize {
        self.notes
            .outbox
            .owed()
            .into_iter()
            .filter(|lane| self.start_lane(hub, lane.clone(), now_ms))
            .count()
    }

    /// Sends the lane's oldest order, or its private notes, when the lane is
    /// free and due and fewer than [`MAX_LANES_IN_FLIGHT`] are on their way.
    fn start_lane(&mut self, hub: &NoteHub, lane: Lane, now_ms: i64) -> bool {
        let outbox = &self.notes.outbox;
        let busy = outbox
            .lanes
            .get(&lane)
            .is_some_and(|state| state.in_flight.active().is_some() || now_ms < state.due_ms);
        if busy || outbox.in_flight() >= MAX_LANES_IN_FLIGHT {
            return false;
        }
        let deleting = match &lane {
            Lane::Private(recording) | Lane::Orders(OrderLane::Meeting(recording)) => {
                outbox.meeting_delete(recording).is_some()
            }
            Lane::Orders(OrderLane::Library) => false,
        };
        let sent = match &lane {
            // A meeting being deleted sends nothing else: its delete goes
            // first, and takes the rest with it.
            Lane::Private(_) if deleting => None,
            Lane::Orders(order_lane) if deleting => outbox
                .orders
                .get(order_lane)
                .and_then(|orders| orders.iter().find(|kept| kept.order.deletes().is_some()))
                .cloned()
                .map(Sent::Order)
                .filter(|sent| self.delete_due(sent)),
            Lane::Private(recording) => match self.reader.private_notes(recording) {
                Ok(Some(pending)) => Some(Sent::Private(pending)),
                Ok(None) => {
                    let outbox = &mut self.notes.outbox;
                    outbox.private.remove(recording);
                    outbox.lanes.remove(&lane);
                    None
                }
                Err(error) => {
                    self.log("private_notes_unreadable", &[("error", &error.to_string())]);
                    self.notes.outbox.failed(&lane, now_ms);
                    None
                }
            },
            Lane::Orders(order_lane) => outbox
                .orders
                .get(order_lane)
                .and_then(|orders| orders.first())
                .cloned()
                .map(Sent::Order),
        };
        let Some(sent) = sent else {
            let purging = self.notes.deletions.purging.as_deref();
            if let Lane::Orders(OrderLane::Meeting(recording)) = &lane
                && deleting
                && purging != Some(recording.as_str())
            {
                // A meeting still closing: its delete is looked at again in
                // a while, not on every turn of the loop.
                let state = self.notes.outbox.lanes.entry(lane).or_default();
                state.due_ms = now_ms.saturating_add(orbit_core::note_outbox::FIRST_RETRY_MS);
            }
            return false;
        };
        self.send_lane(hub, lane, sent);
        true
    }

    fn send_lane(&mut self, hub: &NoteHub, lane: Lane, sent: Sent) {
        let generation = self.state.generation();
        let op = self.turn.op.clone();
        let notes = &mut self.notes;
        let events = notes.events.clone();
        let hub = hub.clone();
        let request = (lane.clone(), sent.clone());
        let state = notes.outbox.lanes.entry(lane).or_default();
        state
            .in_flight
            .start_logical(generation, op, sent, move |ticket| {
                tokio::spawn(async move {
                    let (lane, sent) = request;
                    let outcome = match &sent {
                        Sent::Private(pending) => send_private_notes(&hub, pending).await,
                        Sent::Order(waiting) => match &waiting.order {
                            QueuedOrder::Meeting {
                                recording_id,
                                order,
                            } => send_note_order(&hub, recording_id, order).await,
                            QueuedOrder::Library { request } => {
                                send_library_order(&hub, request).await
                            }
                        },
                    };
                    deliver(
                        &events,
                        NoteEvent::Lane {
                            lane,
                            ticket,
                            outcome,
                        },
                    )
                    .await;
                })
            });
    }
}

impl crate::daemon::recording_notes::NotesHandle {
    /// The hub connected: every lane may go now.
    pub(super) fn outbox_hub_ready(&mut self) {
        for state in self.outbox.lanes.values_mut() {
            state.failures = 0;
            state.due_ms = i64::MIN;
        }
    }
}
