//! The helper's handle: the channel to `OrbitNative`, the orders still
//! waiting for its reply and what the helper last said about itself.
use super::MAX_PENDING_NATIVE;
use super::helper_waits::HelperAnswer;
use super::native_host::{HostIdentity, HostUpdate, NativeHost};
use orbit_core::wire::native::{NativeCommandHeader, NativeResponseFields};
use serde_json::Value;
use std::collections::BTreeMap;
use std::time::Duration;
use tokio::sync::mpsc::error::{SendError, TrySendError};
use tokio::sync::{mpsc, oneshot};

pub(super) struct HelperHandle {
    /// Commands to the helper, in order; the bridge writes them to its stdin.
    pub(super) sender: mpsc::Sender<Value>,
    /// Helper command ids still waiting for a reply, with the name of the
    /// command each one carried. Bounded by [`MAX_PENDING_NATIVE`]; entries
    /// are removed when the matching response arrives, so a late refusal can
    /// be classified without parsing names out of the id.
    pub(super) pending: BTreeMap<String, String>,
    /// Helper orders a task off the loop waits for (`helper_waits`), by id.
    /// An entry leaves when its reply arrives, when its task gave up and
    /// the next order is sent, or when the helper restarts.
    pub(super) waits: BTreeMap<String, oneshot::Sender<HelperAnswer>>,
    /// Observed helper identity (`native_host`).
    pub(super) host: NativeHost,
}

impl HelperHandle {
    pub(super) fn new(sender: mpsc::Sender<Value>) -> Self {
        Self {
            sender,
            pending: BTreeMap::new(),
            waits: BTreeMap::new(),
            host: NativeHost::default(),
        }
    }

    /// Records that `command` was sent to the helper under `id`, so a late
    /// refusal can be classified by command name without parsing the id. Drops
    /// the oldest entry at capacity so the table stays bounded.
    pub(super) fn record_pending(&mut self, command: &Value) {
        let Ok(header) = NativeCommandHeader::from_value(command) else {
            return;
        };
        let (id, name) = (header.id, header.command);
        if self.pending.len() >= MAX_PENDING_NATIVE
            && !self.pending.contains_key(id)
            && let Some(oldest) = self.pending.keys().next().cloned()
        {
            self.pending.remove(&oldest);
        }
        self.pending.insert(id.to_owned(), name.to_owned());
    }

    /// Hands a helper reply to the task waiting for it; `false` when no task
    /// waits for this id.
    pub(super) fn answer_wait(&mut self, response: &NativeResponseFields) -> bool {
        let Some(waiting) = self.waits.remove(&response.id) else {
            return false;
        };
        let answer = if response.ok {
            Ok(response.result.clone().unwrap_or(Value::Null))
        } else {
            Err(response.error.as_ref().map_or_else(
                || "native_operation_failed".to_owned(),
                |error| error.code.clone(),
            ))
        };
        let _ = waiting.send(answer);
        true
    }

    /// Reduce once before dispatching revocation to independent consumers.
    pub(super) fn observe_host(&mut self, event: &Value) -> HostUpdate {
        self.host.observe(event)
    }

    /// Who the helper last said it is; `None` until it says so again.
    pub(super) fn identity(&self) -> Option<&HostIdentity> {
        self.host.identity()
    }

    /// Whether the helper running now has a calendar reader.
    pub(super) fn calendar_backend_available(&self) -> bool {
        self.host.calendar_backend_available()
    }

    /// The current helper's shortcut registration failures; absent before readback.
    pub(super) fn shortcut_failures(
        &self,
    ) -> Option<Vec<orbit_core::topics::status::ShortcutFailure>> {
        self.host.shortcut_failures.clone()
    }

    /// Whether the order sent under `id` still waits for its reply.
    pub(super) fn awaits_reply(&self, id: &str) -> bool {
        self.pending.contains_key(id)
    }

    /// Gives up on the reply to the order sent under `id`.
    pub(super) fn forget_reply(&mut self, id: &str) {
        self.pending.remove(id);
    }

    /// Puts `command` on the helper's channel only if it has room now.
    pub(super) fn try_send(&self, command: Value) -> Result<(), TrySendError<Value>> {
        self.sender.try_send(command)
    }

    /// How many orders the helper's channel holds, for the line that says
    /// it was full.
    pub(super) fn channel_capacity(&self) -> usize {
        self.sender.max_capacity()
    }

    /// Puts `command` on the helper's channel, waiting at most `budget` for
    /// room: `Err` when the budget ran out, `Ok(Err)` when the channel is
    /// closed.
    pub(super) async fn send_within(
        &self,
        command: Value,
        budget: Duration,
    ) -> Result<Result<(), SendError<Value>>, tokio::time::error::Elapsed> {
        tokio::time::timeout(budget, self.sender.send(command)).await
    }

    /// Holds a place on the helper's channel for an order still being
    /// prepared; `None` while the channel is full or closed.
    pub(super) fn reserve(&self) -> Option<mpsc::OwnedPermit<Value>> {
        self.sender.clone().try_reserve_owned().ok()
    }
}
