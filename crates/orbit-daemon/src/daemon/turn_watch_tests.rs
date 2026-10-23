//! The runtime's waits reach the loop's turn under their call sites: the
//! store writer's, through `StoreActor`, and the synchronous reads, through
//! `TimedReader`. Only the held-writer test depends on the threshold, and it
//! holds the writer for twice it; the others read the slowest turn of the
//! period, which is kept whatever its length.
use super::audio::tests::fixture;
use super::turn_watch::{SLOW_TURN_THRESHOLD, TurnSource};
use anyhow::{Context, Result};
use std::time::{Duration, Instant};

fn here(line: u32) -> String {
    format!("daemon/turn_watch_tests.rs:{line}")
}

#[tokio::test]
async fn a_wait_on_the_store_writer_is_named_by_the_line_that_awaited_it() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (runtime, _helper) = fixture(directory.path())?;
    for waits in 0..2 {
        runtime.turn_watch.begin(TurnSource::note_answer());
        let line = if waits == 0 {
            let line = line!() + 1;
            runtime.store_actor.run(|_store| Ok(())).await?;
            line
        } else {
            let line = line!() + 1;
            runtime.store_actor.snapshot(&runtime.state).await?;
            line
        };
        // Whether it passed the threshold is the disk's business (a snapshot
        // once took 972 ms here under load); the name is what is tested.
        let _ = runtime.turn_watch.end();
        let turn = runtime
            .turn_watch
            .take_slowest_since_tick()
            .context("the turn ended")?;
        let wait = turn.wait.context("the wait was reported")?;
        assert_eq!(wait.name(), format!("store@{}", here(line)));
    }
    Ok(())
}

#[tokio::test]
async fn a_held_writer_makes_a_slow_turn_with_its_wait() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (runtime, _helper) = fixture(directory.path())?;
    let release = super::store_actor_tests::hold_the_actor(&runtime)?;
    let held_for = SLOW_TURN_THRESHOLD * 2;
    let releaser = std::thread::spawn(move || {
        std::thread::sleep(held_for);
        let _ = release.send(());
    });
    let started = Instant::now();
    runtime.turn_watch.begin(TurnSource::tick());
    let line = line!() + 1;
    runtime.store_actor.run(|_store| Ok(())).await?;
    let slow = runtime
        .turn_watch
        .end()
        .context("a turn held twice the threshold is slow")?;
    releaser
        .join()
        .map_err(|_| anyhow::anyhow!("the releaser panicked"))?;
    let wait = slow.wait.context("the held wait was reported")?;
    assert_eq!(wait.name(), format!("store@{}", here(line)));
    // The release came `held_for` after the hold began, just before the
    // wait: what was left of it is well over the threshold.
    assert!(wait.took() > SLOW_TURN_THRESHOLD, "{slow:?}");
    assert!(slow.took >= wait.took() && started.elapsed() >= slow.took);
    assert_eq!(slow.step.name(), "handler");
    Ok(())
}

#[tokio::test]
async fn a_read_on_the_loop_is_named_by_its_call_site() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (runtime, _helper) = fixture(directory.path())?;
    runtime.turn_watch.begin(TurnSource::work("upload"));
    let line = line!() + 1;
    let pending = runtime.reader.pending_job_count()?;
    assert_eq!(pending, 0);
    let _ = runtime.turn_watch.end();
    let turn = runtime
        .turn_watch
        .take_slowest_since_tick()
        .context("the turn ended")?;
    let wait = turn.wait.context("the read was reported")?;
    assert_eq!(wait.name(), format!("read@{}", here(line)));
    assert!(wait.took() < Duration::from_secs(2));
    Ok(())
}
