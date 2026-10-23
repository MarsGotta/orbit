//! Receipts for what the writer was handed, and the startup stop's own writer.
use super::*;

#[derive(Clone)]
pub(in crate::daemon) struct StartupStopWriter {
    pub(super) sender: Weak<Mutex<Option<mpsc::SyncSender<StoreOp>>>>,
}

impl StartupStopWriter {
    pub(in crate::daemon) async fn persist(&self, state: State) -> Result<()> {
        let (receipt, response) = tokio::sync::oneshot::channel();
        let sender = self
            .sender
            .upgrade()
            .ok_or(crate::daemon::startup::StartupError::StopWriterStopped)?;
        {
            let sender = sender
                .lock()
                .map_err(|_| crate::daemon::startup::StartupError::StopWriterLockPoisoned)?;
            let sender = sender
                .as_ref()
                .ok_or(crate::daemon::startup::StartupError::StopWriterStopped)?;
            sender
                .try_send(StoreOp::StartupStop {
                    state: Box::new(state),
                    receipt,
                })
                .map_err(admission_error)?;
        }
        receive(response).await?.map_err(anyhow::Error::msg)
    }
}

/// Receipt for an operation admitted synchronously to the ordered writer.
///
/// Admission and waiting are separate so the daemon can preserve admission
/// order in its current turn, then let a worker wait for disk without holding IPC.
pub(in crate::daemon) struct StoreReceipt<T> {
    pub(super) response: tokio::sync::oneshot::Receiver<Result<T>>,
}

/// A failed operation says whether SQLite saw it change anything: when it
/// did, the outcome of those changes is uncertain and must not be retried
/// blindly.
pub(super) fn uncertain<T>(result: Result<T>, changed: bool) -> Result<T> {
    result.map_err(|error| {
        let context = format!(
            "Operación local fallida: cambios SQLite observados {changed}; el resultado de esos cambios es incierto; causa: {error:#}"
        );
        error.context(context)
    })
}

impl<T> StoreReceipt<T> {
    pub(in crate::daemon) async fn wait(self) -> Result<T> {
        receive(self.response).await?
    }
}

pub(super) async fn receive<T>(mut response: tokio::sync::oneshot::Receiver<T>) -> Result<T> {
    // Tokio's paused clock cannot observe work performed by this std thread.
    // Give the writer a short cooperative handoff before awaiting normally so
    // virtual-time deadline tests do not skip over an already-running fsync.
    let handoff_until = Instant::now() + ACTOR_HANDOFF_GRACE;
    loop {
        match response.try_recv() {
            Ok(value) => return Ok(value),
            Err(tokio::sync::oneshot::error::TryRecvError::Closed) => {
                return Err(anyhow::anyhow!(
                    "El actor de almacenamiento perdió el recibo"
                ));
            }
            Err(tokio::sync::oneshot::error::TryRecvError::Empty)
                if Instant::now() < handoff_until =>
            {
                tokio::task::yield_now().await;
            }
            Err(tokio::sync::oneshot::error::TryRecvError::Empty) => {
                return response
                    .await
                    .map_err(|_| anyhow::anyhow!("El actor de almacenamiento perdió el recibo"));
            }
        }
    }
}

pub(super) fn queue_full() -> anyhow::Error {
    anyhow::anyhow!(
        "La cola de almacenamiento de {STORE_OP_CAPACITY} operaciones está llena; la escritura no se admitió"
    )
}

pub(super) fn admission_error<T>(error: mpsc::TrySendError<T>) -> anyhow::Error {
    match error {
        mpsc::TrySendError::Full(_) => queue_full(),
        mpsc::TrySendError::Disconnected(_) => {
            anyhow::anyhow!("El actor de almacenamiento se desconectó; la escritura no se admitió")
        }
    }
}
