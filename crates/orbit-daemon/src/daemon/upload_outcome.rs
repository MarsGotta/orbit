//! What a transfer's outcome means: for its job, for the retry gate and for
//! the person. One rule for every transfer, whether the loop or the capture
//! task ran it.
use anyhow::{Result, bail};
use orbit_core::notice::{Notice, NoticeCode, NoticeKey};
use orbit_store::{ChunkReceipt, Job, JobState, Store};

/// The one retry gate of every transfer, shared by the loop's closures and
/// the capture task's chunks: a hub that refuses or has no room holds both.
#[derive(Clone, Default)]
pub(super) struct SharedGate(std::sync::Arc<std::sync::Mutex<crate::retry::RetryGate>>);

impl SharedGate {
    fn lock(&self) -> std::sync::MutexGuard<'_, crate::retry::RetryGate> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    pub(super) fn ready(&self, clock: std::time::Instant) -> bool {
        self.lock().ready(clock)
    }

    pub(super) fn hold(&self, wait: std::time::Duration, clock: std::time::Instant) {
        self.lock().hold(wait, clock);
    }

    pub(super) fn finish(&self, success: bool, clock: std::time::Instant) {
        self.lock().finish(success, clock);
    }
}

/// A finished transfer, as its job and the person see it.
pub(super) struct Settlement {
    pub(super) succeeded: bool,
    pub(super) next_state: JobState,
    /// What to tell the person about a failed transfer.
    pub(super) notice: Option<Notice>,
    /// The hub has no room: every transfer waits for `HUB_FULL_RETRY`.
    pub(super) out_of_space: bool,
    /// The recording a chunk belongs to: its closure's readiness changed.
    pub(super) chunk_recording: Option<String>,
    /// The chunk the hub confirmed, whose audio can now leave this Mac.
    pub(super) prune: Option<String>,
}

/// The settlement of `job` after a transfer that ended with `result`.
pub(super) fn settle(job: &Job, result: &Result<()>) -> Settlement {
    let failure = result.as_ref().err().map(UploadFailure::of);
    let notice = failure
        .as_ref()
        .and_then(|failure| failure.notice(&job.kind));
    let chunk = job.kind == "chunk_upload";
    let next_state = match &failure {
        None => JobState::Succeeded,
        // Kept on the Mac and never sent again: the hub's answer to the
        // same bytes would not change. A refused chunk no longer holds
        // its meeting's closure (`finalize_is_ready`).
        Some(UploadFailure::Refused { .. }) => JobState::Blocked,
        Some(_) => JobState::RetryWait,
    };
    let succeeded = result.is_ok();
    Settlement {
        succeeded,
        next_state,
        notice,
        out_of_space: matches!(failure, Some(UploadFailure::OutOfSpace(_))),
        chunk_recording: chunk
            .then(|| {
                job.payload["metadata"]["recording_id"]
                    .as_str()
                    .map(str::to_owned)
            })
            .flatten(),
        prune: (succeeded && chunk)
            .then(|| job.payload["metadata"]["id"].as_str().map(str::to_owned))
            .flatten(),
    }
}

/// Where a chunk goes and who sends it.
pub(super) struct ChunkPut {
    pub(super) base: reqwest::Url,
    pub(super) token: String,
    pub(super) receipt: ChunkReceipt,
    pub(super) bytes: Vec<u8>,
}

/// One chunk to the hub, confirmed only by a durable receipt for the same
/// bytes.
pub(super) async fn put_chunk(put: ChunkPut, op: &orbit_core::notice::Op) -> Result<()> {
    let ChunkPut {
        base,
        token,
        receipt,
        bytes,
    } = put;
    let metadata = &receipt.metadata;
    let mut request = crate::hub::request(
        reqwest::Method::PUT,
        base.join(&format!("v1/chunks/{}", metadata.id))?,
        Some(&token),
        op,
    )?
    .header("Content-Type", "audio/wav")
    .header("X-Orbit-Recording", &metadata.recording_id)
    .header("X-Orbit-Track", &metadata.track)
    .header("X-Orbit-Sequence", metadata.sequence)
    .header("X-Orbit-Started-At-Ms", metadata.started_at_ms)
    .header("X-Orbit-Monotonic-Ns", metadata.monotonic_ns)
    .header("X-Orbit-First-Sample", metadata.first_sample)
    .header("X-Orbit-Frames", metadata.frames)
    .header("X-Orbit-Sample-Rate", metadata.sample_rate)
    .header("X-Orbit-Channels", metadata.channels)
    .header("X-Orbit-SHA256", &receipt.sha256);
    if let Some(epoch) = &metadata.capture_epoch_id {
        request = request.header("X-Orbit-Capture-Epoch-Id", epoch);
    }
    let response = crate::retry::check_response(request.body(bytes).send().await?).await?;
    let ack = crate::hub::bounded_json(response).await?;
    if ack["durable"] != true || ack["sha256"] != receipt.sha256 || ack["chunk_id"] != metadata.id {
        bail!("El servidor no confirmó un recibo durable válido");
    }
    Ok(())
}

/// The job's new state and, for a confirmed chunk, its audio pruned: one
/// writer operation.
pub(super) fn record_settlement(
    store: &mut Store,
    job_id: &str,
    settlement_state: JobState,
    prune: Option<&str>,
    root: &std::path::Path,
) -> Result<()> {
    store.update_job(job_id, settlement_state)?;
    if let Some(id) = prune {
        prune_uploaded_audio(store, id, root);
    }
    Ok(())
}

pub(super) fn prune_uploaded_audio(store: &mut Store, id: &str, root: &std::path::Path) {
    let log = crate::diagnostics::DiagnosticLog::in_root(root);
    match store.prune_uploaded_chunk(id) {
        Ok(Some(bytes)) => log.record(
            "uploaded_chunk_deleted",
            &[("chunks", "1"), ("bytes", &bytes.to_string())],
        ),
        Ok(None) => {}
        Err(error) => log.record(
            "uploaded_chunk_delete_failed",
            &[
                ("chunks", "0"),
                ("pending_chunks", "1"),
                ("error", &error.to_string()),
            ],
        ),
    }
}

/// How long uploads wait after the hub said it has no room (507). Space does
/// not come back in seconds: someone frees or grows the hub's storage. One
/// try every five minutes keeps the Mac from sending a one-second chunk every
/// minute to a full disk all day, and still resumes within five minutes of
/// space returning. The audio stays on the Mac meanwhile; nothing is dropped.
pub(super) const HUB_FULL_RETRY: std::time::Duration = std::time::Duration::from_secs(300);

/// What a failed transfer means for its job and for the person.
pub(super) enum UploadFailure {
    /// The network, a timeout or the hub's 5xx: retried with the gate's
    /// backoff, silently, as before.
    Transient,
    /// The hub refuses until something is fixed on this side: 401 (pair
    /// this Mac again), 404 (a proxy or a hub mid-deploy without the route).
    /// Retried with the gate's backoff, and said with its status.
    UntilFixed { status: u16, code: Option<String> },
    /// The hub can never take these bytes: 409 (`recording_finalized`,
    /// `chunk_conflict`: the hub keeps an identical copy idempotently, so a
    /// 409 is never a lost receipt), 403 (this Mac revoked), 400, 413, 415,
    /// 422. Kept on the Mac, said, and declared lost in the closure.
    Refused { status: u16, code: Option<String> },
    /// The hub has no room (507): said with its numbers, retried slowly.
    OutOfSpace(crate::retry::OutOfSpace),
}

impl UploadFailure {
    pub(super) fn of(error: &anyhow::Error) -> Self {
        if let Some(full) = error.downcast_ref::<crate::retry::OutOfSpace>() {
            return Self::OutOfSpace(full.clone());
        }
        let Some(crate::retry::Refused(status)) = error.downcast_ref::<crate::retry::Refused>()
        else {
            return Self::Transient;
        };
        let code = error
            .downcast_ref::<crate::retry::HubCode>()
            .map(|code| code.0.clone());
        match status {
            401 | 404 => Self::UntilFixed {
                status: *status,
                code,
            },
            _ => Self::Refused {
                status: *status,
                code,
            },
        }
    }

    /// The notice for a job of `kind` (`chunk_upload`, `recording_finalize`).
    pub(super) fn notice(&self, kind: &str) -> Option<Notice> {
        let step = |notice: Notice| notice.with_token(NoticeKey::Step, kind);
        let coded = |notice: Notice, status: u16, code: &Option<String>| {
            let notice = step(notice).with(NoticeKey::Status, status);
            match code {
                Some(code) => notice.with_token(NoticeKey::Code, code),
                None => notice,
            }
        };
        match self {
            Self::Transient => None,
            Self::UntilFixed { status, code } => Some(coded(
                Notice::new(NoticeCode::UploadRefusedRetrying),
                *status,
                code,
            )),
            Self::Refused { status, code } => {
                Some(coded(Notice::new(NoticeCode::UploadRefused), *status, code))
            }
            Self::OutOfSpace(full) => {
                let mut notice = step(Notice::new(NoticeCode::HubStorageFull))
                    .with_token(NoticeKey::Code, &full.code)
                    .with(NoticeKey::Status, 507_u16);
                if let Some(used) = full.used_bytes {
                    notice = notice.with(NoticeKey::Bytes, used);
                }
                if let Some(max) = full.max_bytes {
                    notice = notice.with(NoticeKey::MaxBytes, max);
                }
                if let Some(free) = full.free_bytes {
                    notice = notice.with(NoticeKey::Remaining, free);
                }
                Some(notice)
            }
        }
    }
}
