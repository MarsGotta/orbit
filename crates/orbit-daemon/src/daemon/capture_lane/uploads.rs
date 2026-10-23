//! A recording's chunks go up to the hub from the capture task, one at a
//! time, in the order the store lists them, under the retry gate the loop's
//! closures share. The loop is told what to say and count; it is never
//! waited for.
use super::CaptureLane;
use crate::daemon::store_actor::CapturePort;
use crate::daemon::upload_outcome::{self, ChunkPut, HUB_FULL_RETRY, SharedGate};
use crate::supervision::{Actor, Supervision};
use anyhow::{Context, Result};
use futures_util::FutureExt;
use orbit_core::notice::Notice;
use orbit_store::{ChunkReceipt, Job, JobState, ReadOnlyStore};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex, PoisonError},
    time::Instant,
};
use tokio::{sync::mpsc, task::JoinHandle};

/// What the loop lets the uploads do, as of its last turn or decision.
#[derive(Clone, Default)]
pub(in crate::daemon) struct UploadView {
    pub(in crate::daemon) online: bool,
    pub(in crate::daemon) privacy_stopped: bool,
    pub(in crate::daemon) generation: u64,
    pub(in crate::daemon) token: Option<String>,
    pub(in crate::daemon) hub_url: Option<String>,
    pub(in crate::daemon) development: bool,
    /// Recordings thrown away («Terminar sin guardar»): none of their
    /// chunks goes up, and one deleted under a transfer is no failure.
    pub(in crate::daemon) discarded: std::collections::BTreeSet<String>,
}

/// What one chunk's transfer leaves for the loop.
pub(in crate::daemon) enum UploadReport {
    /// The hub answered: the chunk's recording, and the failure to say.
    Settled {
        recording: Option<String>,
        failure: Option<(String, Option<Notice>)>,
    },
    /// The transfer could not even be tried (a read, the credential, the
    /// writer): what the loop's own scheduling failure used to say.
    Problem(String),
    /// The hub answered but its outcome could not be saved: the job stays
    /// as it was and is tried again.
    NotSaved(String),
}

/// The task's one chunk transfer at a time.
pub(super) struct Uploader {
    reader: Arc<Mutex<ReadOnlyStore>>,
    port: CapturePort,
    gate: SharedGate,
    root: PathBuf,
    /// The last job tried: the next one is after it, so one chunk the hub
    /// keeps failing cannot hold the ones behind it.
    cursor: Arc<Mutex<Option<String>>>,
    in_flight: Option<JoinHandle<()>>,
    done: mpsc::Sender<()>,
    supervision: Supervision,
}

/// What a spawned transfer carries.
struct Transfer {
    reader: Arc<Mutex<ReadOnlyStore>>,
    port: CapturePort,
    gate: SharedGate,
    root: PathBuf,
    cursor: Arc<Mutex<Option<String>>>,
    view: UploadView,
}

impl Uploader {
    pub(super) fn new(
        reader: ReadOnlyStore,
        port: CapturePort,
        gate: SharedGate,
        root: PathBuf,
        done: mpsc::Sender<()>,
        supervision: Supervision,
    ) -> Self {
        Self {
            reader: Arc::new(Mutex::new(reader)),
            port,
            gate,
            root,
            cursor: Arc::default(),
            in_flight: None,
            done,
            supervision,
        }
    }

    /// A transfer that ended in a panic is counted, and the next one is
    /// started as after any failed transfer: its job stays where the store
    /// has it (queued or running) and is listed again, and a chunk the hub
    /// already took is answered again by its idempotent PUT.
    fn reap(&self, finished: JoinHandle<()>) {
        if let Some(Err(error)) = finished.now_or_never()
            && error.is_panic()
        {
            self.supervision.panicked(Actor::ChunkUpload);
        }
    }

    /// Starts the next chunk's transfer if none runs and the loop allows it.
    pub(super) fn schedule(&mut self, lane: &CaptureLane) {
        if self
            .in_flight
            .as_ref()
            .is_some_and(|task| !task.is_finished())
        {
            return;
        }
        if let Some(finished) = self.in_flight.take() {
            self.reap(finished);
        }
        let view = lane.lock().upload.clone();
        if view.privacy_stopped || !view.online || !self.gate.ready(Instant::now()) {
            return;
        }
        let transfer = Transfer {
            reader: Arc::clone(&self.reader),
            port: self.port.clone(),
            gate: self.gate.clone(),
            root: self.root.clone(),
            cursor: Arc::clone(&self.cursor),
            view,
        };
        let (lane_for_task, done) = (lane.clone(), self.done.clone());
        let task = tokio::spawn(async move {
            let report = match transfer.run(&lane_for_task).await {
                Ok(report) => report,
                Err(error) => Some(UploadReport::Problem(format!("{error:#}"))),
            };
            // Only a delivered chunk calls the next one at once; anything
            // else waits for the gate's backoff or the task's next tick.
            let delivered = matches!(report, Some(UploadReport::Settled { failure: None, .. }));
            if let Some(report) = report {
                lane_for_task.lock().outbox.uploads.push(report);
                lane_for_task.wake_loop();
            }
            if delivered {
                let _ = done.try_send(());
            }
        });
        lane.lock().upload_abort = Some(task.abort_handle());
        self.in_flight = Some(task);
    }
}

/// The next chunk job, its receipt and whether its recording forbids its
/// upload, read off the capture thread.
async fn next_job(
    reader: &Arc<Mutex<ReadOnlyStore>>,
    cursor: &Arc<Mutex<Option<String>>>,
) -> Result<Option<(Job, ChunkReceipt, bool)>> {
    let (reader, cursor) = (Arc::clone(reader), Arc::clone(cursor));
    tokio::task::spawn_blocking(move || {
        let reader = reader.lock().unwrap_or_else(PoisonError::into_inner);
        let mut cursor = cursor.lock().unwrap_or_else(PoisonError::into_inner);
        let mut jobs = reader.pending_chunk_uploads(cursor.as_deref(), 1)?;
        if jobs.is_empty() {
            *cursor = None;
            jobs = reader.pending_chunk_uploads(None, 1)?;
        }
        let Some(job) = jobs.pop() else {
            return Ok(None);
        };
        *cursor = Some(job.id.clone());
        let chunk_id = job.payload["metadata"]["id"]
            .as_str()
            .context("Trabajo sin chunk_id")?;
        let receipt = reader
            .chunk_receipt(chunk_id)?
            .context("Fragmento pendiente sin recibo local")?;
        let forbidden = reader
            .get_job(&format!("recording-{}", receipt.metadata.recording_id))?
            .is_some_and(|manifest| {
                manifest.kind == "recording_manifest" && manifest.payload["upload_allowed"] == false
            });
        Ok(Some((job, receipt, forbidden)))
    })
    .await?
}

fn blocking_read<T: Send + 'static>(
    reader: &Arc<Mutex<ReadOnlyStore>>,
    read: impl FnOnce(&ReadOnlyStore) -> Result<T> + Send + 'static,
) -> impl std::future::Future<Output = Result<T>> {
    let reader = Arc::clone(reader);
    async move {
        tokio::task::spawn_blocking(move || {
            read(&reader.lock().unwrap_or_else(PoisonError::into_inner))
        })
        .await?
    }
}

impl Transfer {
    async fn run(self, lane: &CaptureLane) -> Result<Option<UploadReport>> {
        #[cfg(test)]
        lane.panic_upload_if_injected();
        let discarded = |recording: &str| lane.lock().upload.discarded.contains(recording);
        let Some((job, receipt, forbidden)) = next_job(&self.reader, &self.cursor).await? else {
            return Ok(None);
        };
        let recording = receipt.metadata.recording_id.clone();
        if discarded(&recording) {
            return Ok(None);
        }
        match self.transfer(lane, job, receipt, forbidden).await {
            // Its chunks were deleted under this transfer: nothing to say.
            Err(_) if discarded(&recording) => Ok(None),
            other => other,
        }
    }

    async fn transfer(
        self,
        lane: &CaptureLane,
        job: Job,
        receipt: ChunkReceipt,
        forbidden: bool,
    ) -> Result<Option<UploadReport>> {
        let job_id = job.id.clone();
        if forbidden {
            self.port
                .begin_run(move |store| Ok(store.update_job(&job_id, JobState::Cancelled)?))?
                .wait()
                .await?;
            return Ok(None);
        }
        let chunk = receipt.metadata.id.clone();
        let bytes =
            blocking_read(&self.reader, move |reader| Ok(reader.read_chunk(&chunk)?)).await?;
        let token = self.view.token.clone().context("Sin credencial")?;
        let hub = self.view.hub_url.as_deref().context("Sin hub")?;
        let base = crate::config::validate_hub_url(hub, self.view.development)?;
        let running = job_id.clone();
        self.port
            .begin_run(move |store| Ok(store.update_job(&running, JobState::Running)?))?
            .wait()
            .await?;
        let put = ChunkPut {
            base,
            token,
            receipt,
            bytes,
        };
        let result = upload_outcome::put_chunk(put, &orbit_ipc::new_op()).await;
        // A privacy stop, a closure or another hub meanwhile: the job stays
        // running and is reconciled by an idempotent PUT later.
        {
            let core = lane.lock();
            if core.upload.generation != self.view.generation || core.upload.privacy_stopped {
                return Ok(None);
            }
        }
        self.settle(job_id, result).await.map(Some)
    }

    async fn settle(self, job_id: String, result: Result<()>) -> Result<UploadReport> {
        let now = Instant::now();
        self.gate.finish(result.is_ok(), now);
        let read = job_id.clone();
        let stored = blocking_read(&self.reader, move |reader| Ok(reader.get_job(&read)?))
            .await?
            .context("Recibo sin trabajo local")?;
        let settlement = upload_outcome::settle(&stored, &result);
        if settlement.out_of_space {
            self.gate.hold(HUB_FULL_RETRY, now);
        }
        let (next_state, prune, root) =
            (settlement.next_state, settlement.prune.clone(), self.root);
        let updated = job_id.clone();
        let saved = self
            .port
            .begin_run(move |store| {
                upload_outcome::record_settlement(
                    store,
                    &updated,
                    next_state,
                    prune.as_deref(),
                    &root,
                )
            })?
            .wait()
            .await
            .with_context(|| format!("No se pudo guardar el resultado de subida {job_id}"));
        if let Err(error) = saved {
            return Ok(UploadReport::NotSaved(format!("{error:#}")));
        }
        Ok(UploadReport::Settled {
            recording: settlement.chunk_recording,
            failure: result
                .err()
                .map(|error| (format!("{error:#}"), settlement.notice)),
        })
    }
}
