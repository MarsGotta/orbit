//! What the hub's answer to one lane's request does to the outbox: a taken
//! order leaves it, a refused one leaves it and is said, one whose answer did
//! not come back waits and goes again, unless it is not sent twice.
use super::{
    Outcome, Runtime,
    outbox::{Lane, Sent},
};
use crate::daemon::ticketed::WorkTicket;
use anyhow::{Context, Result};
use orbit_core::note_outbox::{Lane as OrderLane, QueuedOrder};
use orbit_store::{PendingPrivateNotes, WaitingOrder};

impl Runtime {
    /// One lane's answer. An answer started before a privacy stop or a
    /// change of pairing settles as expired and is dropped; what it carried
    /// is still owed.
    pub(super) async fn lane_answer(
        &mut self,
        lane: Lane,
        ticket: WorkTicket,
        outcome: Outcome,
        now_ms: i64,
    ) -> Result<()> {
        let current = self.state.generation();
        let settlement = self.features.notes.outbox.settle(&lane, &ticket, current);
        let Some(sent) = self.settled_value("note_lane", settlement) else {
            return Ok(());
        };
        match (lane, sent) {
            (Lane::Private(recording), Sent::Private(pending)) => {
                self.private_answer(recording, pending, outcome, now_ms)
                    .await
            }
            (Lane::Orders(lane), Sent::Order(sent)) => {
                self.order_delivered(lane, sent, outcome, now_ms).await
            }
            (Lane::Private(_), Sent::Order(_)) | (Lane::Orders(_), Sent::Private(_)) => {
                self.log("note_lane_mismatch", &[]);
                Ok(())
            }
        }
    }

    async fn private_answer(
        &mut self,
        recording: String,
        sent: PendingPrivateNotes,
        outcome: Outcome,
        now_ms: i64,
    ) -> Result<()> {
        let lane = Lane::Private(recording.clone());
        match outcome {
            Outcome::Done { .. } => {
                // The text the hub confirmed is the one sent; a newer one
                // typed meanwhile stays in the outbox and goes next.
                let id = recording.clone();
                let confirmed = self
                    .store_actor
                    .run(move |store| Ok(store.confirm_private_notes(&id, &sent.markdown)?))
                    .await
                    .context("No se pudo apuntar que el hub tiene las notas privadas")?;
                self.features
                    .notes
                    .outbox
                    .private_delivered(&recording, confirmed);
            }
            Outcome::Refused { status, code, .. } => {
                // They stay on this Mac: only the person can give them up.
                self.log(
                    "private_notes_not_saved",
                    &[
                        ("status", &status.to_string()),
                        ("code", code.as_deref().unwrap_or("none")),
                    ],
                );
                let outbox = &mut self.features.notes.outbox;
                outbox.failed(&lane, now_ms);
                if outbox.first_said(&lane, status, code.as_deref()) {
                    self.private_notes_refused(status, code.as_deref());
                }
            }
            Outcome::Unreachable { .. } => {
                self.log_outcome("private_notes_not_saved", &outcome);
                self.features.notes.outbox.failed(&lane, now_ms);
            }
        }
        Ok(())
    }

    async fn order_delivered(
        &mut self,
        lane: OrderLane,
        sent: WaitingOrder,
        outcome: Outcome,
        now_ms: i64,
    ) -> Result<()> {
        let key = Lane::Orders(lane.clone());
        if matches!(outcome, Outcome::Unreachable { .. }) && sent.order.repeatable() {
            // The hub did not answer: the order waits and goes again.
            self.log_outcome("note_order_retrying", &outcome);
            self.features.notes.outbox.failed(&key, now_ms);
            return Ok(());
        }
        if sent.order.deletes().is_some() {
            return self.delete_answered(lane, sent, outcome, now_ms).await;
        }
        let (lane_key, target, sequence) = (lane.key(), sent.target.clone(), sent.sequence);
        self.store_actor
            .run(move |store| Ok(store.remove_note_order(&lane_key, &target, sequence)?))
            .await
            .context("No se pudo apuntar en este Mac que el hub respondió al cambio")?;
        let outbox = &mut self.features.notes.outbox;
        outbox.succeeded(&key);
        // A newer order that replaced this one meanwhile keeps its place.
        outbox.forget(&lane, &sent);
        match sent.order {
            QueuedOrder::Meeting {
                recording_id,
                order,
            } => {
                self.features.notes.head.touch_recording(&recording_id);
                self.order_answer(&order, outcome);
            }
            QueuedOrder::Library { request } => self.library_answer(&request, outcome),
        }
        Ok(())
    }
}
