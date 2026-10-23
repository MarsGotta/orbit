//! How a decision's state is adopted around the retirements that run before
//! it: what those retirements already changed is kept, not overwritten by a
//! decision computed before they ran.
use super::super::retire::RetirePhase;
use super::super::*;
use orbit_core::authority::{AuthorityChange, RetirePlan, retire_stopped_decision};

impl Runtime {
    /// A new session or turn cancels the sources before its decision is
    /// adopted, as the old command path did. A failed receipt cannot commit
    /// that decision or retire its other volatile workers.
    pub(super) async fn retire_before_decision(
        &mut self,
        state: &mut State,
        change: AuthorityChange,
        plan: RetirePlan,
    ) -> Result<()> {
        if !matches!(
            change,
            AuthorityChange::Conversation | AuthorityChange::Interrupt
        ) {
            return Ok(());
        }
        self.execute_retire_plan(plan, change, RetirePhase::BeforeDecision, None)
            .await?;
        self.keep_before_decision_receipts(state, change);
        Ok(())
    }

    /// The decision was calculated before the fallible source receipt.
    /// Keep that receipt and the source notice when adopting its other
    /// fields; otherwise a successful cancellation becomes Listing.
    fn keep_before_decision_receipts(&self, state: &mut State, change: AuthorityChange) {
        state
            .views
            .source_inventory
            .clone_from(&self.state.views.source_inventory);
        state
            .views
            .source_request
            .clone_from(&self.state.views.source_request);
        match change {
            AuthorityChange::Conversation => {
                state.durable.notice.clone_from(&self.state.durable.notice);
                state
                    .durable
                    .notice_log
                    .clone_from(&self.state.durable.notice_log);
            }
            AuthorityChange::Interrupt => {
                // The core has already added the interruption notice to
                // its decision. Put it after the source notices produced
                // by BeforeDecision, in the same order as the old command
                // path.
                // Those local cancellations also change published state:
                // the decision was computed before they ran, so retain
                // their settled call phase when adopting it.
                state
                    .durable
                    .voice
                    .call
                    .clone_from(&self.state.durable.voice.call);
                state
                    .durable
                    .notice_log
                    .clone_from(&self.state.durable.notice_log);
                if let Some(notice) = state.durable.notice.clone() {
                    state.set_notice(notice);
                }
            }
            AuthorityChange::UserStop
            | AuthorityChange::Revoked
            | AuthorityChange::Rearm
            | AuthorityChange::RearmWithoutStop
            | AuthorityChange::Closing
            | AuthorityChange::HubLink
            | AuthorityChange::HubTransportReady
            | AuthorityChange::HubLinkOffline
            | AuthorityChange::HubDisconnected
            | AuthorityChange::Device => {}
        }
    }

    /// The retirements that run once the decision is adopted and before it
    /// is stored: what appeared while stopped, then the change's own
    /// volatile workers and its claimed input.
    pub(super) async fn retire_before_snapshot(
        &mut self,
        change: Option<AuthorityChange>,
        plan: Option<RetirePlan>,
    ) -> Result<()> {
        if self.state.privacy_stopped()
            && !matches!(
                change,
                Some(
                    AuthorityChange::UserStop | AuthorityChange::Revoked | AuthorityChange::Closing
                )
            )
        {
            // A later Tick or unrelated decision must still close authority
            // that appeared while stopped. The old kernel repeated these
            // nine slots, but never repeated hub or durable retirement here.
            let stopped = retire_stopped_decision();
            for phase in [RetirePhase::Volatile, RetirePhase::PreSnapshot] {
                self.execute_retire_plan(stopped, AuthorityChange::UserStop, phase, None)
                    .await?;
            }
        }
        if let Some((change, plan)) = change.zip(plan) {
            self.execute_retire_plan(plan, change, RetirePhase::Volatile, None)
                .await?;
            // A claimed input is withdrawn before its snapshot is stored.
            self.execute_retire_plan(plan, change, RetirePhase::PreSnapshot, None)
                .await?;
        }
        Ok(())
    }

    /// The retirements after the effects: the base sent capture_stop and
    /// Interrupt before a hub drain or a fallible source retirement, so a
    /// failed durable slot must not skip either effect after a revocation.
    pub(super) async fn retire_after_effects(
        &mut self,
        change: AuthorityChange,
        plan: RetirePlan,
    ) -> Result<()> {
        let before_retirement = self.state.clone();
        self.execute_retire_plan(plan, change, RetirePhase::Link, None)
            .await?;
        self.execute_retire_plan(plan, change, RetirePhase::Durable, None)
            .await?;
        if self.state != before_retirement {
            self.store_actor.snapshot(&self.state).await?;
            self.persisted.durable = DurableSnapshot::from(&self.state);
        }
        Ok(())
    }
}
