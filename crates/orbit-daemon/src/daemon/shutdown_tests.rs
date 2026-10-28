//! Shutdown fixtures: one pending list for every exit door, and a closure that
//! ends on its own bounded budget. Only daemon behavior is exercised here; no
//! clock, disk failure or signal delivery outside these synthetic facts is
//! treated as verified.
use super::*;
use crate::daemon::audio::tests::fixture;
use anyhow::{Context, Result};
use orbit_core::notice::{NoticeCode, NoticeKey};
use std::time::{Duration, Instant};

// A fixture receipt may wait longer than the product's 1128 ms grace without
// turning a disk scheduling stall into a claim about exit order. This test-only
// grace is 64 times the worst 63 ms critical receipt in 30 loaded Linux runs;
// it is not a new product deadline. APFS fullfsync on a Mac is not measured
// here and must be remeasured there.
const TEST_CRITICAL_RECEIPT_GRACE: Duration = Duration::from_millis(4_032);
// Twice that receipt grace bounds a missing timeout in the fixture, not fsync.
const TEST_RECEIPT_JAM_GUARD: Duration = Duration::from_millis(8_064);
// Four times the worst 4777 ms complete loaded snapshot test bounds a writer
// that never reaches our synthetic barrier; it is not a store speed target.
const WRITER_ENTRY_JAM_GUARD: Duration = Duration::from_millis(19_108);

fn assert_latched_budget(closing: &Shutdown, before: Instant, after: Instant) {
    assert!(
        closing
            .first_press_at
            .is_some_and(|at| before <= at && at <= after),
        "first press {:?} was outside the latch call [{before:?}, {after:?}]",
        closing.first_press_at
    );
    assert_eq!(
        closing
            .first_press_at
            .map(|at| closing.exit_now(at + SHUTDOWN_BUDGET - Duration::from_nanos(1))),
        Some(false),
        "closure was not latched or exited early"
    );
    assert_eq!(
        closing
            .first_press_at
            .map(|at| closing.exit_now(at + SHUTDOWN_BUDGET)),
        Some(true),
        "closure did not exit at the latched budget"
    );
}

/// Makes every future insert into `table` fail, forever, like a dying disk.
fn refuse_writes(runtime: &Runtime, table: &str) -> Result<()> {
    let database = rusqlite::Connection::open(runtime.paths.store.join("orbit.sqlite3"))?;
    database.execute_batch(&format!(
        "CREATE TRIGGER fixture_p2_04_failure BEFORE INSERT ON {table} BEGIN SELECT RAISE(ABORT, 'synthetic write failure'); END;"
    ))?;
    Ok(())
}

fn unsaved_voice_tail(runtime: &mut Runtime) -> Result<()> {
    runtime.capture.buffer.begin_voice()?;
    let voice = runtime
        .capture
        .buffer
        .voice
        .as_mut()
        .context("voice absent")?;
    voice.pcm = vec![3; 200];
    voice.sample_rate = 8000;
    Ok(())
}

#[tokio::test]
async fn shutdown_command_and_exit_gate_agree_on_every_pending_vertical() -> Result<()> {
    // A runtime with nothing pending: the order is accepted, the single list
    // is empty and the exit gate opens.
    {
        let directory = tempfile::tempdir()?;
        let (mut runtime, _receiver) = fixture(directory.path())?;
        {
            let fixture_value = serde_json::json!(false);
            crate::daemon::authority_fixture::set(
                &mut runtime.state,
                "capabilities.native_helper",
                fixture_value,
            );
        }
        assert!(runtime.has_any_pending().is_empty());
        let mut closing = Shutdown::default();
        closing.command(Command::Shutdown, &mut runtime).await?;
        assert!(closing.ready(&runtime));
    }
    // Seals the disk will never accept: the order refuses and the gate stays
    // shut, exactly like the single pending list says.
    {
        let directory = tempfile::tempdir()?;
        let (mut runtime, _receiver) = fixture(directory.path())?;
        {
            let fixture_value = serde_json::json!(false);
            crate::daemon::authority_fixture::set(
                &mut runtime.state,
                "capabilities.native_helper",
                fixture_value,
            );
        }
        unsaved_voice_tail(&mut runtime)?;
        refuse_writes(&runtime, "jobs")?;
        assert!(!runtime.has_any_pending().is_empty());
        let mut closing = Shutdown::default();
        assert!(
            closing
                .command(Command::Shutdown, &mut runtime)
                .await
                .is_err()
        );
        assert!(!closing.ready(&runtime));
    }
    Ok(())
}

#[tokio::test]
async fn a_seal_that_never_succeeds_still_ends_the_exit_within_the_budget() -> Result<()> {
    // A closure that has not started has no budget to spend.
    let idle = Shutdown::default();
    assert!(!idle.exit_now(Instant::now() + Duration::from_secs(3600)));
    let directory = tempfile::tempdir()?;
    let (mut runtime, _receiver) = fixture(directory.path())?;
    {
        let fixture_value = serde_json::json!(false);
        crate::daemon::authority_fixture::set(
            &mut runtime.state,
            "capabilities.native_helper",
            fixture_value,
        );
    }
    unsaved_voice_tail(&mut runtime)?;
    refuse_writes(&runtime, "jobs")?;
    let mut closing = Shutdown::default();
    let before = Instant::now();
    assert!(
        closing
            .command(Command::Shutdown, &mut runtime)
            .await
            .is_err()
    );
    let after = Instant::now();
    assert!(!closing.ready(&runtime));
    assert!(!runtime.has_any_pending().is_empty());
    // The one closing budget, counted from the latch.
    assert_latched_budget(&closing, before, after);
    // Crossing the budget declares what stayed in memory — counts, not
    // content. The cause is the unconfirmed storage write, not an exhausted
    // reserve. Declaring is not discarding: the fragment remains counted.
    runtime.declare_bounded_exit().await?;
    assert_eq!(
        runtime.state.durable.notice,
        Some(
            orbit_core::notice::Notice {
                sequence: runtime
                    .state
                    .durable
                    .notice
                    .as_ref()
                    .and_then(|notice| notice.sequence),
                op: runtime
                    .state
                    .durable
                    .notice
                    .as_ref()
                    .and_then(|notice| notice.op.clone()),
                ..orbit_core::notice::Notice::new(NoticeCode::ClosedWithPendingWork)
            }
            .with(NoticeKey::AudioUnsaved, true)
            .with(NoticeKey::ReserveExhausted, false)
        ),
        "{:?}",
        runtime.state.durable.notice
    );
    assert!(!runtime.has_any_pending().is_empty());
    Ok(())
}

/// The first termination signal — SIGINT from a terminal or
/// SIGTERM from launchd, they are one request — latches the closure with the
/// one budget; a second one means "now" and the daemon leaves at once. There
/// is no third, shorter budget any more.
#[test]
fn a_first_signal_latches_the_budget_and_a_second_one_leaves_at_once() {
    let mut closing = Shutdown::default();
    let before = Instant::now();
    closing.signal();
    let after = Instant::now();
    assert!(closing.requested);
    assert_latched_budget(&closing, before, after);
    closing.signal();
    assert!(closing.exit_now(Instant::now()));
}

/// A signal that arrives after an IPC `Shutdown` is the first
/// signal, not the second: a SIGTERM that lands while that closure runs must
/// not cut it short.
#[test]
fn a_signal_after_an_ipc_shutdown_does_not_cut_the_closure_short() {
    let mut closing = Shutdown::default();
    let before = Instant::now();
    closing.latch();
    let after = Instant::now();
    closing.signal();
    assert_latched_budget(&closing, before, after);
}

#[tokio::test]
async fn reserve_loss_keeps_its_own_cause_at_bounded_exit() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _receiver) = fixture(directory.path())?;
    runtime.capture.reserve_exhausted = true;
    runtime.declare_bounded_exit().await?;
    assert_eq!(flag(&runtime, NoticeKey::ReserveExhausted), Some(true));
    assert_eq!(flag(&runtime, NoticeKey::AudioUnsaved), Some(false));
    Ok(())
}

#[tokio::test]
async fn a_drained_old_reserve_loss_is_not_repeated_at_a_later_exit() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _helper) = fixture(directory.path())?;
    runtime.capture.reserve_exhausted = true;
    runtime.collect_ready_seals(Duration::ZERO, false)?;
    runtime.declare_bounded_exit().await?;
    assert_eq!(
        flag(&runtime, NoticeKey::ReserveExhausted),
        Some(false),
        "a past recording's exhausted reserve was reported again: {:?}",
        runtime.state.durable.notice
    );
    Ok(())
}

#[tokio::test]
async fn bounded_exit_receives_the_loss_snapshot_before_detaching_writer() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _helper) = fixture(directory.path())?;
    runtime.capture.reserve_exhausted = true;
    runtime.store_actor.defer_snapshot(&runtime.state)?;
    tokio::time::timeout(
        TEST_RECEIPT_JAM_GUARD,
        runtime.declare_bounded_exit_with_grace(
            BoundedExitCause::BudgetExpired,
            TEST_CRITICAL_RECEIPT_GRACE,
        ),
    )
    .await
    .context(format!(
        "critical snapshot did not finish within the {} ms guard",
        TEST_RECEIPT_JAM_GUARD.as_millis()
    ))??;
    let saved: orbit_core::State = runtime
        .fixture_store
        .load_snapshot()?
        .context("closing snapshot absent on disk")?;
    assert_eq!(
        saved.notice_code(),
        Some(NoticeCode::ClosedWithPendingWork),
        "saved: {:?}",
        saved.durable.notice
    );
    assert!(runtime.store_actor.bounded_exit_declared());
    Ok(())
}

#[tokio::test]
async fn bounded_exit_reports_the_passed_grace_and_detaches_a_stalled_writer() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _helper) = fixture(directory.path())?;
    crate::daemon::turn::said::trace(&mut runtime, directory.path());
    runtime.capture.reserve_exhausted = true;
    let (entered_tx, entered_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
    let _blocked = runtime.store_actor.begin_run(move |_| {
        entered_tx.send(())?;
        release_rx.recv()?;
        Ok(())
    })?;
    entered_rx.recv_timeout(WRITER_ENTRY_JAM_GUARD)?;
    // The worst observed critical receipt is enough to prove that this
    // permanently blocked writer reports the passed value, not the product's.
    let test_grace = Duration::from_millis(63);
    let result = tokio::time::timeout(
        TEST_RECEIPT_JAM_GUARD,
        runtime.declare_bounded_exit_with_grace(BoundedExitCause::BudgetExpired, test_grace),
    )
    .await;
    drop(release_tx);
    result.context(format!(
        "bounded exit did not detach a blocked writer within the {} ms guard",
        TEST_RECEIPT_JAM_GUARD.as_millis()
    ))??;
    assert!(runtime.store_actor.bounded_exit_declared());
    // The step that failed is said; the passed grace is in daemon.log.
    assert_eq!(
        runtime.state.notice_code(),
        Some(NoticeCode::StopStepFailed),
        "{:?}",
        runtime.state.durable.notice
    );
    let told = crate::daemon::turn::said::told(&runtime);
    assert!(told.contains("unconfirmed after 63 ms"), "{told}");
    Ok(())
}

#[tokio::test]
async fn expired_shutdown_names_an_unfinished_close_without_inventing_a_disk_fault() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _helper) = fixture(directory.path())?;
    let mut closing = Shutdown::default();
    closing.latch();
    closing.deadline = Some(Instant::now() - Duration::from_secs(1));
    let refusal = closing
        .command(Command::Shutdown, &mut runtime)
        .await
        .err()
        .context("expired close was accepted")?
        .to_string();
    assert!(refusal.contains("deadline"), "refusal: {refusal}");
    assert!(
        refusal.contains("pending: snapshot: 1"),
        "refusal: {refusal}"
    );
    assert!(
        !refusal.contains("storage"),
        "the deadline alone cannot diagnose storage: {refusal}"
    );
    Ok(())
}

/// A boolean the current notice carries.
fn flag(runtime: &crate::daemon::Runtime, key: NoticeKey) -> Option<bool> {
    match runtime.state.durable.notice.as_ref()?.data.get(&key)? {
        orbit_core::notice::DataValue::Bool(value) => Some(*value),
        _ => None,
    }
}
