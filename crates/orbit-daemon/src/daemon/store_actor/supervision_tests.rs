//! A panic of the store writer: the operation it was running is abandoned
//! with a failure, never answered as written; the writer goes on with the
//! next one; a seal it was writing is owed and written on its retry; past
//! its budget it stops and refuses everything. Synthetic store and audio.
use super::*;
use crate::daemon::store_actor_tests::pending_segment;
use crate::supervision::{Actor, Exhausted, RESTART_BUDGET};
use anyhow::Context;
use std::time::Duration;

/// A jam guard for the writer's own five-second retry, not a measured
/// budget: the suite runs under load.
const JAM: Duration = Duration::from_secs(60);

fn actor(directory: &std::path::Path) -> Result<(StoreActor, Supervision)> {
    let store = Store::open(directory.join("store"), [0x42; 32])?;
    let (events, _received) = tokio::sync::mpsc::channel(crate::daemon::WORK_EVENTS_CAPACITY);
    let supervision = Supervision::default();
    let actor = StoreActor::spawn_supervised(store, events, supervision.clone())?;
    Ok((actor, supervision))
}

#[allow(clippy::panic, reason = "the operation panics on purpose")]
fn panicking(_: &mut Store) -> Result<()> {
    panic!("injected store writer panic")
}

#[tokio::test]
async fn an_operation_that_panics_fails_and_the_writer_goes_on() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (actor, supervision) = actor(directory.path())?;
    let abandoned = actor.begin_run(panicking)?.wait().await;
    assert!(abandoned.is_err(), "a panicked operation answered as done");
    let job = orbit_store::Job {
        id: "finalize-after-a-panic".into(),
        kind: "recording_finalize".into(),
        state: orbit_store::JobState::Queued,
        created_at: 1,
        payload: serde_json::json!({"recording_id":"after-a-panic"}),
    };
    let written = job.clone();
    actor
        .run(move |store| Ok(store.enqueue_job(&written)?))
        .await
        .context("the writer did not go on after the panic")?;
    let read = actor
        .run(|store| Ok(store.get_job("finalize-after-a-panic")?))
        .await?;
    assert_eq!(read.map(|job| job.id), Some(job.id));
    assert_eq!(supervision.restarts(Actor::StoreWriter), 1);
    assert_eq!(supervision.exhausted(), None);
    Ok(())
}

/// What the writer has logged for the capture lane, waited for.
async fn completion(actor: &StoreActor) -> Result<StoreCompletion> {
    let started = std::time::Instant::now();
    loop {
        if let Some(completion) = actor.take_completions().into_iter().next() {
            return Ok(completion);
        }
        anyhow::ensure!(started.elapsed() < JAM, "the writer logged nothing");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[tokio::test]
async fn a_seal_the_writer_panicked_on_is_owed_and_written_on_its_retry() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut actor, supervision) = actor(directory.path())?;
    let segment = Arc::new(pending_segment("grabacion-del-panico", 0));
    faults::panic_on_first_seal(&segment.metadata.id);
    actor.seal(Arc::clone(&segment))?;
    let first = completion(&actor).await?;
    let StoreCompletion::SealFailed { cause, owed, .. } = first else {
        anyhow::bail!("a panicked seal was acknowledged as sealed");
    };
    assert_eq!(cause, WRITER_PANIC_CAUSE);
    assert_eq!(owed, 1);
    let id = segment.metadata.id.clone();
    let stored = actor
        .run(move |store| Ok(store.chunk_receipt(&id)?))
        .await?;
    assert!(stored.is_none(), "the chunk is on disk before its retry");
    // The retry the daemon asks for when the lane is owed audio.
    actor.retry_seals()?;
    let retried = completion(&actor).await?;
    assert!(
        matches!(retried, StoreCompletion::Sealed { ref id, .. } if *id == segment.metadata.id),
        "the owed seal was not written on its retry"
    );
    let id = segment.metadata.id.clone();
    let stored = actor
        .run(move |store| Ok(store.chunk_receipt(&id)?))
        .await?;
    assert!(stored.is_some(), "the retried chunk is not on disk");
    assert_eq!(supervision.restarts(Actor::StoreWriter), 1);
    Ok(())
}

#[tokio::test]
async fn a_writer_past_its_budget_stops_and_refuses_every_write() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (actor, supervision) = actor(directory.path())?;
    for _ in 0..=RESTART_BUDGET {
        assert!(actor.begin_run(panicking)?.wait().await.is_err());
    }
    assert_eq!(
        supervision.exhausted(),
        Some(Exhausted {
            actor: Actor::StoreWriter,
            panics: RESTART_BUDGET + 1,
        })
    );
    let refused = actor.run(|_| Ok(())).await;
    assert!(
        refused.is_err(),
        "a writer past its budget still answered a write as done"
    );
    Ok(())
}
