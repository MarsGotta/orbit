//! The account's full name, which names the user's microphone on the hub:
//! read once off the loop, and sent to the hub each time it connects.
use super::{NoteEvent, Outcome, Runtime, deliver};
use crate::hub_notes::NoteHub;
use orbit_core::notes as core;

/// After the profile could not reach the hub it goes again no sooner than
/// this: the hub is down or the network is, and once every thirty seconds
/// keeps a failing hub from being asked every second. Not a deadline: the
/// name goes again at the next connection anyway.
const RETRY_AFTER_MS: i64 = 30_000;

impl Runtime {
    pub(super) fn start_owner_lookup(&mut self) {
        let notes = &mut self.notes;
        if notes.owner_name.is_some() || notes.owner_lookup_started {
            return;
        }
        notes.owner_lookup_started = true;
        let events = notes.events.clone();
        let supervision = self.supervision.clone();
        let read = self.owner.read();
        tokio::spawn(async move {
            // A panic reads as an account without a name, as a failed read.
            let name = supervision
                .guard(crate::supervision::Actor::NoteOwner, read)
                .await
                .unwrap_or_else(|_| core::owner_name(None));
            deliver(&events, NoteEvent::Owner { name }).await;
        });
    }

    pub(super) fn start_profile(&mut self, hub: &NoteHub, now_ms: i64) -> bool {
        let generation = self.state.generation();
        let op = self.turn.op.clone();
        let notes = &mut self.notes;
        let Some(owner) = notes.owner_name.clone() else {
            return false;
        };
        if !notes.profile_owed || now_ms < notes.profile_due_ms {
            return false;
        }
        let hub = hub.clone();
        let events = notes.events.clone();
        let started = notes
            .profile
            .start_logical(generation, op, (), move |ticket| {
                tokio::spawn(async move {
                    let outcome = hub.profile(&owner).await.into();
                    deliver(&events, NoteEvent::Profile { ticket, outcome }).await;
                })
            })
            .is_some();
        if started {
            notes.profile_owed = false;
        }
        started
    }

    /// The hub's answer to the profile: what it did not take goes again later.
    pub(super) fn profile_answer(&mut self, outcome: &Outcome, now_ms: i64) {
        if !matches!(outcome, Outcome::Done { .. }) {
            self.log_outcome("device_profile_not_saved", outcome);
            let notes = &mut self.notes;
            notes.profile_owed = true;
            notes.profile_due_ms = now_ms.saturating_add(RETRY_AFTER_MS);
        }
    }
}
