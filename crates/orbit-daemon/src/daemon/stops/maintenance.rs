//! The loop's once-a-second upkeep, in independent parts: a failing
//! vertical is said and never keeps the capture's upkeep from running. The
//! chunks' own upkeep (seal receipts and retries, uploads) runs on the
//! capture task's clock, off the loop entirely.
use super::*;

impl Runtime {
    /// The whole upkeep in its order: verticals, capture, state. What a test
    /// runs as one second of the loop; the loop itself runs the capture's
    /// part on its own clock (`capture_tick`), never behind the others.
    #[cfg(test)]
    pub(in crate::daemon) async fn run_maintenance(&mut self) {
        Box::pin(self.run_vertical_maintenance()).await;
        Box::pin(self.run_capture_maintenance()).await;
        Box::pin(self.run_state_maintenance()).await;
    }

    /// An owed privacy retirement, then every vertical but the capture. Each
    /// part is a named step of the turn (`daemon_turn_slow`).
    pub(in crate::daemon) async fn run_vertical_maintenance(&mut self) {
        let watch = self.turn_watch.clone();
        if self.privacy_retirement_owed && self.state.privacy_stopped() && !self.closing {
            // A writer blocked past the user's stop budget cannot erase the work
            // that followed it. Retry only the durable tail: a new hub or
            // other live authority is never retired by a later tick.
            if watch
                .time(
                    "privacy_retirement",
                    tokio::time::timeout(
                        RETIREMENT_RETRY_SLICE,
                        self.retire_authority_durable(
                            retire(AuthorityChange::UserStop),
                            AuthorityChange::UserStop,
                        ),
                    ),
                )
                .await
                .is_ok()
            {
                self.privacy_retirement_owed = false;
                if let Err(error) = self.store_actor.defer_snapshot(&self.state) {
                    self.log(
                        "stop_snapshot_defer_failed",
                        &[("error", &error.to_string())],
                    );
                }
            }
        }
        // The verticals name their own steps, one per vertical.
        if let Err(error) = Box::pin(self.maintain_verticals()).await {
            watch
                .time(
                    "maintain_verticals.contain",
                    self.contain(
                        StopCause::VerticalMaintenanceFailed,
                        &format!(
                            "Fallo en el mantenimiento de Orbit, no de la captura: {error}. No se corta nada."
                        ),
                    ),
                )
                .await;
        }
    }

    /// The capture's part: only its own failure cuts what is live.
    pub(in crate::daemon) async fn run_capture_maintenance(&mut self) {
        let watch = self.turn_watch.clone();
        if let Err(error) = watch
            .time("maintain_capture", Box::pin(self.maintain_capture()))
            .await
        {
            watch
                .time(
                    "maintain_capture.contain",
                    self.contain(
                        StopCause::CaptureMaintenanceFailed,
                        &format!("Fallo en el mantenimiento de la captura: {error}."),
                    ),
                )
                .await;
        }
    }

    /// The pending jobs, the closures and the waiting voice notes: their
    /// failures are the state's or the network's, said and never a cut.
    pub(in crate::daemon) async fn run_state_maintenance(&mut self) {
        let watch = self.turn_watch.clone();
        if let Err(error) = watch
            .time("maintain_state", Box::pin(self.maintain_state()))
            .await
        {
            watch
                .time(
                    "maintain_state.contain",
                    self.contain(
                        StopCause::VerticalMaintenanceFailed,
                        &format!(
                            "Fallo al guardar el estado o la cola de subida de Orbit: {error}. No se corta nada."
                        ),
                    ),
                )
                .await;
        }
    }
}
