//! The outbox's lanes under supervision: a lane whose request ended in a
//! panic can deliver nothing, so its slot is cleared and the lane sends
//! again what the store still has waiting, as after a request that failed.
use super::Outbox;

impl Outbox {
    /// Clears every lane whose request panicked; returns how many.
    pub(in crate::daemon) fn reap_panicked(&mut self) -> usize {
        let mut fell = 0;
        for lane in self.lanes.values_mut() {
            if lane.in_flight.panicked().is_some() {
                lane.in_flight.cancel();
                fell += 1;
            }
        }
        fell
    }
}

#[cfg(test)]
mod tests {
    use super::super::{Lane, LaneState, Sent};
    use super::Outbox;

    #[tokio::test]
    #[allow(clippy::panic, reason = "the request panics on purpose")]
    async fn a_lane_whose_request_panics_is_cleared_to_send_again() -> anyhow::Result<()> {
        let mut outbox = Outbox::default();
        let mut lane = LaneState::default();
        let sent = Sent::Private(orbit_store::PendingPrivateNotes {
            recording_id: "6f2b7d5a-9c41-4b7e-8d3f-1a2b3c4d5e6f".into(),
            markdown: "Notas sintéticas".into(),
            base_revision: 0,
        });
        lane.in_flight.attach_test(
            0,
            sent,
            tokio::spawn(async { panic!("injected lane panic") }),
        );
        let private = Lane::Private("6f2b7d5a-9c41-4b7e-8d3f-1a2b3c4d5e6f".into());
        outbox.lanes.insert(private.clone(), lane);
        while outbox.lanes[&private]
            .in_flight
            .task_handle()
            .is_some_and(|task| !task.is_finished())
        {
            tokio::task::yield_now().await;
        }
        assert_eq!(outbox.reap_panicked(), 1);
        assert!(outbox.lanes[&private].in_flight.active().is_none());
        assert_eq!(outbox.reap_panicked(), 0, "a fall is counted once");
        Ok(())
    }
}
