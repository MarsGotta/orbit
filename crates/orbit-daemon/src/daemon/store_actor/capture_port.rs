//! The writer as the capture lane uses it: seals, and the updates of the
//! upload jobs those seals queued.
//!
//! What it admits goes to the same ordered writer as every other operation;
//! this port only lets a second thread admit it without going through the
//! loop. Its sender is cleared when the actor closes, so no port can keep
//! the writer thread alive after its owner is gone.
use super::*;

/// What the writer logged for a seal: its durable receipt or its failure.
pub(in crate::daemon) enum StoreCompletion {
    Sealed {
        id: String,
        recording: String,
        sequence: u64,
        #[allow(
            dead_code,
            reason = "the queue wait is consumed by the measurement tests"
        )]
        queue_wait_us: Option<u64>,
    },
    SealFailed {
        message: String,
        /// Why, as a code (`capture_lane::seal_cause`).
        cause: &'static str,
        /// Seals the writer still owes, this one included.
        owed: usize,
    },
}

#[derive(Clone)]
pub(in crate::daemon) struct CapturePort {
    sender: Arc<Mutex<Option<mpsc::SyncSender<StoreOp>>>>,
    completions: Arc<Mutex<VecDeque<StoreCompletion>>>,
    /// Seals admitted and not yet receipted, by chunk id: a segment is
    /// admitted once however many times it is offered.
    dispatched: Arc<Mutex<BTreeSet<String>>>,
}

fn stopped() -> anyhow::Error {
    anyhow::anyhow!("The storage actor stopped")
}

impl CapturePort {
    pub(super) fn new(
        sender: mpsc::SyncSender<StoreOp>,
        completions: Arc<Mutex<VecDeque<StoreCompletion>>>,
    ) -> Self {
        Self {
            sender: Arc::new(Mutex::new(Some(sender))),
            completions,
            dispatched: Arc::default(),
        }
    }

    fn send(&self, operation: StoreOp) -> Result<()> {
        let sender = self.sender.lock().map_err(|_| stopped())?;
        sender
            .as_ref()
            .ok_or_else(stopped)?
            .try_send(operation)
            .map_err(admission_error)
    }

    /// Admits one typed operation and returns its future durable receipt,
    /// never waiting for the writer.
    pub(in crate::daemon) fn begin_run<T, F>(&self, operation: F) -> Result<StoreReceipt<T>>
    where
        T: Send + 'static,
        F: FnOnce(&mut Store) -> Result<T> + Send + 'static,
    {
        let (receipt, response) = tokio::sync::oneshot::channel();
        let operation = Box::new(move |store: &mut Store| {
            let before = store.mutation_count();
            let result = operation(store);
            let changed = store.mutation_count() != before;
            receipt.send(uncertain(result, changed)).is_err()
        });
        self.send(StoreOp::Run(operation))?;
        Ok(StoreReceipt { response })
    }

    /// Admits a seal unless it was admitted already; never waits.
    pub(in crate::daemon) fn seal(&self, segment: Arc<crate::capture::Segment>) -> Result<()> {
        let id = segment.metadata.id.clone();
        let mut dispatched = self.dispatched.lock().map_err(|_| stopped())?;
        if dispatched.contains(&id) {
            return Ok(());
        }
        self.send(StoreOp::Seal {
            segment,
            admitted_at: Instant::now(),
        })?;
        dispatched.insert(id);
        Ok(())
    }

    pub(in crate::daemon) fn acknowledge(&self, id: &str) {
        if let Ok(mut dispatched) = self.dispatched.lock() {
            dispatched.remove(id);
        }
    }

    pub(in crate::daemon) fn retry(&self) -> Result<()> {
        self.send(StoreOp::RetrySeals)
    }

    /// The receipts and failures the writer has logged since the last take.
    pub(in crate::daemon) fn take_completions(&self) -> Vec<StoreCompletion> {
        let Ok(mut completions) = self.completions.lock() else {
            return Vec::new();
        };
        completions.drain(..).collect()
    }

    pub(super) fn close(&self) {
        match self.sender.lock() {
            Ok(mut sender) => {
                sender.take();
            }
            Err(poisoned) => {
                poisoned.into_inner().take();
            }
        }
    }
}

impl StoreActor {
    #[cfg(test)]
    pub(in crate::daemon) fn seal(&mut self, segment: Arc<crate::capture::Segment>) -> Result<()> {
        self.seals.seal(segment)
    }

    pub(in crate::daemon) fn retry_seals(&self) -> Result<()> {
        self.seals.retry()
    }

    #[cfg(test)]
    pub(in crate::daemon) fn take_completions(&self) -> Vec<StoreCompletion> {
        self.seals.take_completions()
    }

    /// The writer as the capture lane uses it off the loop.
    pub(in crate::daemon) fn capture_port(&self) -> CapturePort {
        self.seals.clone()
    }
}
