//! What each authority change retires, and the hub's drain when it goes.
use super::*;

impl Runtime {
    /// Stops all new network work and every vertical's authority before
    /// fallible persistence or native answers. Shared by the user's stop and by
    /// closing; a cut does not retire anything: Orbit keeps working.
    pub(in crate::daemon) async fn retire_authority(
        &mut self,
        change: AuthorityChange,
        interrupted_turn: Option<&str>,
    ) {
        let plan = retire(change);
        self.retire_authority_volatile(plan, change).await;
        if let Err(error) = self
            .execute_retire_plan(plan, change, RetirePhase::Link, interrupted_turn)
            .await
        {
            self.log("retire_link_failed", &[("error", &error.to_string())]);
        }
        self.retire_authority_durable(plan, change).await;
    }

    pub(in crate::daemon) async fn retire_authority_volatile(
        &mut self,
        plan: RetirePlan,
        change: AuthorityChange,
    ) {
        if let Err(error) = self
            .execute_retire_plan(plan, change, RetirePhase::Volatile, None)
            .await
        {
            self.log("retire_volatile_failed", &[("error", &error.to_string())]);
            self.say_step_failure("retire_volatile", &error);
        }
    }

    pub(in crate::daemon) async fn retire_hub(
        &mut self,
        interrupted_turn: Option<&str>,
        reason: hub_link::Retirement,
    ) {
        // Closing the sender lets hub::run write queued frames and close its
        // socket. Its own per-frame write is bounded at 5 s; the same window
        // protects retirement from a hub that never finishes local delivery.
        // Twenty loopback stops wrote the frame (load 4.38–4.67). This is not
        // a receipt from Hermes or permission to resume the stopped turn.
        let had_hub = self.link.outbound.retire(reason);
        if let Some(mut task) = self.link.task.take() {
            if had_hub {
                match tokio::time::timeout(std::time::Duration::from_secs(5), &mut task).await {
                    Ok(Ok(Ok(()))) => {}
                    // An unsent-frames error already names its count.
                    Ok(Ok(Err(error))) => self.hub_drain_failed(
                        interrupted_turn,
                        &match hub::unsent_count(&error) {
                            Some(_) => format!("el socket falló: {error}"),
                            None => {
                                format!("el socket falló: {error}; {HUB_FRAMES_UNCOUNTED}")
                            }
                        },
                    ),
                    Ok(Err(error)) => self.hub_drain_failed(
                        interrupted_turn,
                        &format!("la tarea terminó: {error}; {HUB_FRAMES_UNCOUNTED}"),
                    ),
                    Err(_) => {
                        task.abort();
                        self.hub_drain_failed(
                            interrupted_turn,
                            &format!("el vaciado agotó 5 s; {HUB_FRAMES_UNCOUNTED}"),
                        );
                    }
                }
            } else {
                task.abort();
            }
        }
    }

    pub(in crate::daemon) async fn retire_authority_durable(
        &mut self,
        plan: RetirePlan,
        change: AuthorityChange,
    ) {
        if let Err(error) = self
            .execute_retire_plan(plan, change, RetirePhase::Durable, None)
            .await
        {
            self.log("retire_durable_failed", &[("error", &error.to_string())]);
            self.say_step_failure("retire_durable", &error);
        }
    }

    pub(in crate::daemon) fn hub_drain_failed(
        &mut self,
        interrupted_turn: Option<&str>,
        cause: &str,
    ) {
        if let Some(turn_id) = interrupted_turn {
            self.interrupt_delivery_unknown(turn_id, cause);
        } else {
            self.say(
                Notice::new(NoticeCode::StopStepFailed)
                    .with_token(NoticeKey::Step, "hub_drain")
                    .with_token(NoticeKey::Cause, cause),
            );
        }
    }
}
