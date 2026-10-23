//! What the kernel stores after a decision: the fingerprint of the durable
//! snapshot, so an unchanged decision is not stored again, and the closure a
//! replaced recording leaves behind.
use super::super::*;
use orbit_core::authority::AuthorityChange;

#[derive(Clone, PartialEq, Eq)]
pub(in crate::daemon) struct DurableSnapshot {
    recording: Option<orbit_core::Recording>,
    privacy_stopped: bool,
    privacy_stop_by: Option<orbit_core::privacy_stop::StopRequester>,
    generation: u64,
    connection: Connection,
    session_id: Option<String>,
    agent_id: Option<String>,
    pending_approval: Option<Approval>,
    call: Option<orbit_core::voice::CallState>,
}

/// What the kernel last stored, so an unchanged decision is not stored again.
pub(in crate::daemon) struct Persisted {
    pub(in crate::daemon) durable: DurableSnapshot,
    /// The next decision is stored even if its snapshot did not change.
    pub(in crate::daemon) force_once: bool,
}

impl Persisted {
    pub(in crate::daemon) fn new(state: &State) -> Self {
        Self {
            durable: DurableSnapshot::from(state),
            force_once: false,
        }
    }
}

impl From<&State> for DurableSnapshot {
    fn from(state: &State) -> Self {
        Self {
            recording: state.durable.capture.recording.clone(),
            privacy_stopped: state.privacy_stopped(),
            privacy_stop_by: state.privacy_stop_by(),
            generation: state.generation(),
            connection: state.connection().clone(),
            session_id: state.session_id().clone(),
            agent_id: state.agent_id().map(str::to_owned),
            pending_approval: state.durable.assistant.pending_approval.clone(),
            call: state.durable.voice.call.clone(),
        }
    }
}

/// A recording that ended in place: closed, and idle or interrupted.
pub(super) fn closed(state: &State) -> bool {
    state.durable.capture.recording.as_ref().is_some_and(|r| {
        r.ended_at_ms.is_some() && matches!(r.phase, CapturePhase::Idle | CapturePhase::Interrupted)
    })
}

impl Runtime {
    /// Stores the adopted decision: with the replaced recording's closure in
    /// one transaction, or alone when its snapshot changed or is forced. A
    /// failed store restores `previous` for a decision with no authority
    /// change, or one that only changed the conversation or interrupted it.
    pub(super) async fn persist_decision(
        &mut self,
        closure: Option<Job>,
        force_snapshot: bool,
        change: Option<AuthorityChange>,
        previous: State,
    ) -> Result<()> {
        let next_durable = DurableSnapshot::from(&self.state);
        let persisted = if let Some(job) = closure {
            self.store_actor
                .snapshot_with_job(&self.state, job)
                .await
                .map(|()| true)
        } else if force_snapshot || next_durable != self.persisted.durable {
            self.store_actor.snapshot(&self.state).await.map(|()| false)
        } else {
            Ok(false)
        };
        match persisted {
            Ok(with_closure) => {
                if with_closure {
                    // This receipt covers the snapshot and replacement
                    // closure in one store transaction.
                    self.notes.head.touch();
                }
                self.persisted.durable = next_durable;
                Ok(())
            }
            Err(error) => {
                if change.is_none()
                    || matches!(
                        change,
                        Some(AuthorityChange::Conversation | AuthorityChange::Interrupt)
                    )
                {
                    self.state = previous;
                }
                Err(error)
            }
        }
    }
}
