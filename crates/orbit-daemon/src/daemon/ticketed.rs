//! Ownership of an asynchronous worker and its generation.
use std::time::{Duration, Instant};

use tokio::task::JoinHandle;

use super::id;

/// A named limit for one wait. Its reason is required at compile time for
/// constants; this is an operational wait, never a data limit or a retry.
#[derive(Clone, Copy)]
pub(crate) struct DeadlineKind {
    duration: Duration,
    reason: &'static str,
}

impl DeadlineKind {
    pub(crate) const fn new(name: &'static str, duration: Duration, reason: &'static str) -> Self {
        assert!(!name.is_empty(), "deadline name must not be empty");
        assert!(!reason.is_empty(), "deadline reason must not be empty");
        Self { duration, reason }
    }

    pub(crate) const fn duration(self) -> Duration {
        self.duration
    }

    pub(crate) fn start(self) -> Deadline {
        self.starting_at(Instant::now())
    }

    pub(crate) fn starting_at(self, started: Instant) -> Deadline {
        Deadline {
            started,
            duration: self.duration,
            reason: self.reason,
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) struct Deadline {
    started: Instant,
    duration: Duration,
    reason: &'static str,
}

impl Deadline {
    pub(crate) fn at(self) -> Instant {
        debug_assert!(!self.reason.is_empty());
        self.started + self.duration
    }

    pub(crate) fn started(self) -> Instant {
        self.started
    }

    pub(crate) fn expired(self) -> bool {
        Instant::now() >= self.at()
    }

    pub(crate) fn waited_ms(self) -> u64 {
        u64::try_from(self.started.elapsed().as_millis()).unwrap_or(u64::MAX)
    }

    pub(crate) fn budget_ms(self) -> u64 {
        u64::try_from(self.duration.as_millis()).unwrap_or(u64::MAX)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct WorkTicket {
    id: String,
    generation: u64,
    /// The op of the turn that started the work: its requests to
    /// the hub and the notices its result causes carry it.
    op: orbit_core::notice::Op,
}

impl WorkTicket {
    pub(super) fn new(generation: u64, op: orbit_core::notice::Op) -> Self {
        Self {
            id: id(),
            generation,
            op,
        }
    }

    pub(super) fn id(&self) -> &str {
        &self.id
    }

    pub(super) fn op(&self) -> &orbit_core::notice::Op {
        &self.op
    }

    pub(super) fn generation(&self) -> u64 {
        self.generation
    }

    #[cfg(test)]
    pub(super) fn test(generation: u64) -> Self {
        Self::new(generation, orbit_ipc::new_op())
    }
}

pub(super) enum Settlement<T> {
    Current(T),
    Foreign,
    Expired {
        started_generation: u64,
        current_generation: u64,
    },
}

struct Active<T> {
    ticket: WorkTicket,
    value: T,
    task: Option<JoinHandle<()>>,
}

pub(super) struct Ticketed<T> {
    active: Option<Active<T>>,
}

impl<T> Default for Ticketed<T> {
    fn default() -> Self {
        Self { active: None }
    }
}

impl<T> Ticketed<T> {
    pub(super) fn logical() -> Self {
        Self::default()
    }
}

impl<T> Drop for Ticketed<T> {
    fn drop(&mut self) {
        self.cancel();
    }
}

impl<T> Ticketed<T> {
    pub(super) fn active(&self) -> Option<&T> {
        self.active.as_ref().map(|active| &active.value)
    }

    #[cfg(test)]
    pub(super) fn task_handle(&self) -> Option<&JoinHandle<()>> {
        self.active.as_ref().and_then(|active| active.task.as_ref())
    }

    #[cfg(test)]
    pub(super) fn task_handle_mut(&mut self) -> Option<&mut JoinHandle<()>> {
        self.active.as_mut().and_then(|active| active.task.as_mut())
    }

    #[cfg(test)]
    pub(super) fn take_task(&mut self) -> Option<JoinHandle<()>> {
        self.active.take().and_then(|active| active.task)
    }

    #[cfg(test)]
    pub(super) fn take(&mut self) -> Option<JoinHandle<()>> {
        self.take_task()
    }

    #[cfg(test)]
    pub(super) fn attach_test(
        &mut self,
        generation: u64,
        value: T,
        task: JoinHandle<()>,
    ) -> WorkTicket {
        self.cancel();
        let ticket = WorkTicket::test(generation);
        self.active = Some(Active {
            ticket: ticket.clone(),
            value,
            task: Some(task),
        });
        ticket
    }

    /// The ticket of the work in flight, for owners whose later steps must
    /// stay bound to the same attempt. Comparing tickets decides identity;
    /// generation belongs to `settle` or a non-consuming expiration check.
    pub(super) fn ticket(&self) -> Option<&WorkTicket> {
        self.active.as_ref().map(|active| &active.ticket)
    }

    pub(super) fn expired_generation(&self, ticket: &WorkTicket, generation: u64) -> Option<u64> {
        self.active
            .as_ref()
            .filter(|active| active.ticket == *ticket && ticket.generation != generation)
            .map(|_| ticket.generation)
    }

    pub(super) fn start_logical(
        &mut self,
        generation: u64,
        op: orbit_core::notice::Op,
        value: T,
        worker: impl FnOnce(WorkTicket) -> JoinHandle<()>,
    ) -> Option<WorkTicket> {
        if self.active.is_some() {
            return None;
        }
        let ticket = WorkTicket::new(generation, op);
        let task = worker(ticket.clone());
        self.active = Some(Active {
            ticket: ticket.clone(),
            value,
            task: Some(task),
        });
        Some(ticket)
    }

    /// Abort the logical work in flight, if any, and start its replacement.
    /// An owner reaches for this when a new order replaces the previous one
    /// instead of waiting for it; the replaced result then settles foreign.
    pub(super) fn replace_logical(
        &mut self,
        generation: u64,
        op: orbit_core::notice::Op,
        value: T,
        worker: impl FnOnce(WorkTicket) -> JoinHandle<()>,
    ) -> WorkTicket {
        self.cancel();
        let ticket = WorkTicket::new(generation, op);
        let task = worker(ticket.clone());
        self.active = Some(Active {
            ticket: ticket.clone(),
            value,
            task: Some(task),
        });
        ticket
    }

    pub(super) fn settle(&mut self, ticket: &WorkTicket, generation: u64) -> Settlement<T> {
        if self
            .active
            .as_ref()
            .is_none_or(|active| active.ticket != *ticket)
        {
            return Settlement::Foreign;
        }
        let Some(active) = self.active.take() else {
            return Settlement::Foreign;
        };
        if ticket.generation != generation {
            return Settlement::Expired {
                started_generation: ticket.generation,
                current_generation: generation,
            };
        }
        Settlement::Current(active.value)
    }

    /// The ticket of a worker that ended in a panic, once: it can deliver
    /// nothing, so its owner settles it as a failure or clears the slot. A
    /// worker that ended normally keeps its slot until its result settles.
    pub(super) fn panicked(&mut self) -> Option<WorkTicket> {
        use futures_util::FutureExt;
        let active = self.active.as_mut()?;
        if !active.task.as_ref()?.is_finished() {
            return None;
        }
        let finished = active.task.take()?;
        match finished.now_or_never() {
            Some(Err(error)) if error.is_panic() => Some(active.ticket.clone()),
            _ => None,
        }
    }

    pub(super) fn cancel(&mut self) {
        if let Some(active) = self.active.take()
            && let Some(task) = active.task
        {
            task.abort();
        }
    }

    /// A helper response has no local worker to abort. The ticket still owns
    /// its identity and generation so a replacement cannot settle as current.
    pub(super) fn start_external(
        &mut self,
        generation: u64,
        op: orbit_core::notice::Op,
        value: T,
    ) -> Option<WorkTicket> {
        if self.active.is_some() {
            return None;
        }
        let ticket = WorkTicket::new(generation, op);
        self.active = Some(Active {
            ticket: ticket.clone(),
            value,
            task: None,
        });
        Some(ticket)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helper_reply_ticket_has_no_worker_and_reports_both_generations() {
        let mut reply = Ticketed::logical();
        let Some(ticket) = reply.start_external(4, orbit_ipc::new_op(), "helper") else {
            panic!("empty helper slot refused its first reply");
        };
        assert!(reply.task_handle().is_none());
        assert!(matches!(
            reply.settle(&WorkTicket::test(4), 4),
            Settlement::Foreign
        ));
        assert!(matches!(
            reply.settle(&ticket, 5),
            Settlement::Expired {
                started_generation: 4,
                current_generation: 5
            }
        ));
        assert!(reply.active().is_none());
    }

    #[tokio::test]
    async fn settlement_distinguishes_current_foreign_and_expired() {
        let mut work = Ticketed::logical();
        let current = work.start_logical(4, orbit_ipc::new_op(), 17, |_| {
            tokio::spawn(std::future::pending())
        });
        let Some(current) = current else {
            panic!("first work was not admitted");
        };
        let foreign = WorkTicket::test(4);
        assert!(matches!(work.settle(&foreign, 4), Settlement::Foreign));
        assert_eq!(work.active(), Some(&17));
        assert!(matches!(work.settle(&current, 4), Settlement::Current(17)));
        assert!(work.active().is_none());

        let expired = work.start_logical(5, orbit_ipc::new_op(), 23, |_| {
            tokio::spawn(std::future::pending())
        });
        let Some(expired) = expired else {
            panic!("replacement work was not admitted");
        };
        assert!(matches!(
            work.settle(&expired, 6),
            Settlement::Expired {
                started_generation: 5,
                current_generation: 6,
            }
        ));
        assert!(work.active().is_none());
    }
}
