//! Preserve a permanent store refusal as typed data across the writer receipt.
use orbit_store::{Deleted, DeletionScope};

pub(in crate::daemon) enum PurgeFailure {
    Permanent(String),
    Retry(String),
}

pub(super) async fn purge_steps(
    port: crate::daemon::store_actor::CapturePort,
    recording: String,
    scope: DeletionScope,
) -> Result<Deleted, PurgeFailure> {
    let mut total = Deleted::default();
    loop {
        let id = recording.clone();
        let receipt = port
            .begin_run(move |store| {
                match store.delete_recording(&id, scope, super::PURGE_STEP_CHUNKS) {
                    Ok(deleted) => Ok(Ok(deleted)),
                    // A request that can never work must not become a generic
                    // uncertain writer failure, which would be retried forever.
                    Err(
                        error @ (orbit_store::Error::Invalid(_)
                        | orbit_store::Error::CountOutOfRange { .. }),
                    ) => Ok(Err(PurgeFailure::Permanent(error.to_string()))),
                    Err(error) => Err(error.into()),
                }
            })
            .map_err(|error| PurgeFailure::Retry(format!("{error:#}")))?;
        let step = receipt
            .wait()
            .await
            .map_err(|error| PurgeFailure::Retry(format!("{error:#}")))??;
        total.chunks += step.chunks;
        total.jobs += step.jobs;
        total.records += step.records;
        total.orders += step.orders;
        if step.done {
            return Ok(total);
        }
    }
}
