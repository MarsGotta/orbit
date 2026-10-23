//! Regressions: the daemon actor stays responsive while the store is
//! slow, and the snapshot policy stops paying for chat deltas and idle ticks.
//! Real encrypted SQLite in a fixture directory; synthetic native channels.
use super::*;
use crate::daemon::audio::tests::{SEAL_RECEIPT_GUARD, await_seal_receipts, fixture};
use base64::{Engine, engine::general_purpose::STANDARD};
use orbit_core::notice::NoticeCode;
use orbit_store::ChunkMetadata;

#[path = "fixtures/durable_state_v0_7_0.rs"]
mod v0_7_0;

fn previous_snapshot() -> Result<serde_json::Value> {
    // Frozen flat 0.7 shape, with synthetic data and presentation views.
    Ok(serde_json::from_str(include_str!(
        "fixtures/snapshot_v0_7_0.json"
    ))?)
}

fn retained_durable_fields(previous: v0_7_0::DurableState) -> Result<serde_json::Value> {
    let mut value = serde_json::to_value(previous)?;
    // These three Deimos reads were already retired from the Mac's store.
    let fields = value
        .as_object_mut()
        .context("flat previous durable object")?;
    for field in ["source_inventory", "source_request", "source_receipt"] {
        assert!(
            fields.remove(field).is_some(),
            "previous reader field {field}"
        );
    }
    Ok(value)
}

#[test]
fn snapshot_v0_7_0_with_views_loads_without_losing_durable_fields() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("store");
    let key = [0x69; 32];
    let saved = previous_snapshot()?;
    let previous: v0_7_0::DurableState = serde_json::from_value(saved.clone())?;
    let expected = retained_durable_fields(previous)?;
    let mut store = Store::open(&path, key)?;
    store.save_snapshot(&saved)?;
    drop(store);
    let reader = Store::open_existing(&path, key)?;
    let loaded = reader
        .load_snapshot::<DurableState>()?
        .context("snapshot absent")?;
    let actual = serde_json::to_value(&loaded)?;
    assert_eq!(actual, expected, "every retained durable field survives");
    assert_eq!(loaded.generation(), 37);
    assert!(loaded.privacy_stopped());
    assert_eq!(
        loaded.privacy_stop_by(),
        Some(orbit_core::privacy_stop::StopRequester::User)
    );
    Ok(())
}

#[tokio::test]
async fn new_snapshots_are_flat_durable_only_and_readable_by_v0_7_0() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("store");
    let key = [0x69; 32];
    let store = Store::open(&path, key)?;
    let (events, _received) = mpsc::channel(WORK_EVENTS_CAPACITY);
    let actor = store_actor::StoreActor::spawn(store, events)?;
    // Exercise each production write path, including the atomic job write.
    for mode in 0..3 {
        let mut saved = previous_snapshot()?;
        saved["pending_jobs"] = (5 + mode).into();
        let expected = retained_durable_fields(serde_json::from_value(saved.clone())?)?;
        let state: State = serde_json::from_value(saved)?;
        match mode {
            0 => actor.snapshot(&state).await?,
            1 => actor.critical_snapshot(&state).await?,
            2 => {
                actor
                    .snapshot_with_job(
                        &state,
                        orbit_store::Job {
                            id: "synthetic-job".into(),
                            kind: "recording_manifest".into(),
                            state: orbit_store::JobState::Queued,
                            created_at: 0,
                            payload: serde_json::json!({}),
                        },
                    )
                    .await?
            }
            _ => unreachable!(),
        }
        let reader = Store::open_existing(&path, key)?;
        let raw = reader
            .load_snapshot::<serde_json::Value>()?
            .context("snapshot absent")?;
        let previous = reader
            .load_snapshot::<v0_7_0::DurableState>()?
            .context("old reader snapshot absent")?;
        assert_eq!(
            retained_durable_fields(previous)?,
            expected,
            "old reader, mode {mode}"
        );
        assert_eq!(
            raw, expected,
            "only flat durable fields are saved, mode {mode}"
        );
    }
    Ok(())
}

#[tokio::test]
async fn abandoned_store_receipt_does_not_replace_the_user_notice() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let store = Store::open(directory.path().join("store"), [0x42; 32])?;
    let (events, mut received) = mpsc::channel(WORK_EVENTS_CAPACITY);
    let actor = store_actor::StoreActor::spawn_with_diagnostics(
        store,
        events,
        crate::diagnostics::DiagnosticLog::in_root(directory.path()),
        crate::supervision::Supervision::default(),
    )?;
    let abandoned = actor.begin_run(|_| Ok::<_, anyhow::Error>(()))?;
    drop(abandoned);
    actor.run(|_| Ok::<_, anyhow::Error>(())).await?;
    // A StoreProblem would replace the daemon's single notice slot, including
    // the user's privacy-stop notice. This abandoned waiter is diagnostic only.
    assert!(received.try_recv().is_err());
    let log = std::fs::read_to_string(directory.path().join("logs/daemon.log"))?;
    assert_eq!(
        log.matches("store_receipt_receiver_gone").count(),
        1,
        "{log}"
    );
    assert!(log.contains("\"operation\":\"run\""), "{log}");
    Ok(())
}

#[tokio::test]
async fn startup_stop_receipt_cannot_be_replaced_by_an_older_runtime_snapshot() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("store");
    let key = [0x42; 32];
    let store = Store::open(&path, key)?;
    let (events, _received) = mpsc::channel(WORK_EVENTS_CAPACITY);
    let actor = store_actor::StoreActor::spawn(store, events)?;
    let mut stopped = State::default();
    crate::daemon::authority_fixture::set(&mut stopped, "privacy_stopped", serde_json::json!(true));
    crate::daemon::authority_fixture::set(
        &mut stopped,
        "privacy_stop_by",
        serde_json::json!(orbit_core::privacy_stop::StopRequester::User),
    );
    crate::daemon::authority_fixture::set(&mut stopped, "generation", serde_json::json!(1));
    actor.startup_stop_writer()?.persist(stopped).await?;
    actor.critical_snapshot(&State::default()).await?;
    let reader = Store::open_existing(&path, key)?;
    let saved = reader
        .load_snapshot::<DurableState>()?
        .context("startup stop snapshot absent")?;
    assert!(saved.privacy_stopped());
    assert_eq!(
        saved.privacy_stop_by(),
        Some(orbit_core::privacy_stop::StopRequester::User)
    );
    actor.finish_startup().await?;
    let mut rearmed = State::default();
    crate::daemon::authority_fixture::set(&mut rearmed, "generation", serde_json::json!(2));
    actor.critical_snapshot(&rearmed).await?;
    let saved = reader
        .load_snapshot::<DurableState>()?
        .context("rearm snapshot absent")?;
    assert!(!saved.privacy_stopped());
    Ok(())
}

#[tokio::test]
async fn two_startup_stops_do_not_replace_a_user_stopped_runtime_snapshot() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("store");
    let key = [0x42; 32];
    let store = Store::open(&path, key)?;
    let (events, _received) = mpsc::channel(WORK_EVENTS_CAPACITY);
    let actor = store_actor::StoreActor::spawn(store, events)?;
    let stop = actor.startup_stop_writer()?;
    for generation in [1, 2] {
        let mut stopped = State::default();
        crate::daemon::authority_fixture::set(
            &mut stopped,
            "privacy_stopped",
            serde_json::json!(true),
        );
        crate::daemon::authority_fixture::set(
            &mut stopped,
            "privacy_stop_by",
            serde_json::json!(orbit_core::privacy_stop::StopRequester::User),
        );
        crate::daemon::authority_fixture::set(
            &mut stopped,
            "generation",
            serde_json::json!(generation),
        );
        stop.persist(stopped).await?;
    }
    let mut runtime = State::default();
    crate::daemon::authority_fixture::set(&mut runtime, "privacy_stopped", serde_json::json!(true));
    crate::daemon::authority_fixture::set(
        &mut runtime,
        "privacy_stop_by",
        serde_json::json!(orbit_core::privacy_stop::StopRequester::User),
    );
    crate::daemon::authority_fixture::set(&mut runtime, "generation", serde_json::json!(1));
    runtime.durable.active_request = Some("runtime-recovered-request".into());
    actor.critical_snapshot(&runtime).await?;
    let saved = Store::open_existing(&path, key)?
        .load_snapshot::<DurableState>()?
        .context("runtime snapshot absent")?;
    assert_eq!(saved.generation(), runtime.generation());
    assert_eq!(saved.active_request, runtime.durable.active_request);
    assert_eq!(saved.privacy_stop_by(), runtime.privacy_stop_by());
    Ok(())
}

/// Opens the fixture's database from outside the daemon, in the same pattern
/// the finalize fixtures use for their failure trigger. Triggers count
/// snapshot writes, which no single read of the row could distinguish.
fn snapshot_probe(runtime: &Runtime) -> Result<rusqlite::Connection> {
    let database = rusqlite::Connection::open(runtime.paths.store.join("orbit.sqlite3"))?;
    database.execute_batch(
        "CREATE TABLE probe_snapshot_writes(n INTEGER NOT NULL);
         INSERT INTO probe_snapshot_writes VALUES (0);
         CREATE TRIGGER probe_snapshot_insert AFTER INSERT ON settings
         WHEN NEW.key = 'snapshot'
         BEGIN UPDATE probe_snapshot_writes SET n = n + 1; END;
         CREATE TRIGGER probe_snapshot_update AFTER UPDATE ON settings
         WHEN NEW.key = 'snapshot'
         BEGIN UPDATE probe_snapshot_writes SET n = n + 1; END;",
    )?;
    Ok(database)
}

fn snapshot_writes(probe: &rusqlite::Connection) -> Result<u64> {
    let count: i64 =
        probe.query_row("SELECT n FROM probe_snapshot_writes", [], |row| row.get(0))?;
    Ok(count.max(0) as u64)
}

/// Holds SQLite's single writer slot so every seal and snapshot write blocks
/// until released: a delayed `seal_chunk`.
fn hold_store_writer(probe: &rusqlite::Connection) -> Result<()> {
    probe.execute_batch("BEGIN IMMEDIATE")?;
    Ok(())
}

fn release_store_writer(probe: &rusqlite::Connection) -> Result<()> {
    probe.execute_batch("ROLLBACK")?;
    Ok(())
}

fn stored_chunks(probe: &rusqlite::Connection, id: &str) -> Result<u64> {
    let count: i64 = probe.query_row("SELECT count(*) FROM chunks WHERE id = ?1", [id], |row| {
        row.get(0)
    })?;
    Ok(count.max(0) as u64)
}

async fn start_recording(
    runtime: &mut Runtime,
    receiver: &mut mpsc::Receiver<Value>,
    recording: &str,
) -> Result<String> {
    runtime
        .apply(Event::StartMeeting {
            id: recording.into(),
            title: "Synthetic recording".into(),
            audio: orbit_core::protocol::MeetingAudio::MicrophoneOnly,
        })
        .await?;
    let command = receiver.recv().await.context("start command absent")?;
    assert_eq!(command["command"], "audio_start");
    runtime
        .native_event(json!({"event":"response","id":command["id"],"ok":true,
            "result":{"stream_id":command["stream_id"]}}))
        .await?;
    Ok(command["stream_id"]
        .as_str()
        .context("stream absent")?
        .to_owned())
}

fn audio_frame(stream: &str, sequence: u64, frames: u64) -> Value {
    json!({"event":"audio","source":"mic_local","stream_id":stream,
        "sample_rate":8000,"channels":1,"frames":frames,"sequence":sequence,
        "monotonic_ns":sequence.saturating_mul(10_000),
        "pcm_s16le_base64":STANDARD.encode(vec![1u8; frames as usize * 2])})
}

pub(super) fn pending_segment(recording: &str, sequence: u64) -> crate::capture::Segment {
    crate::capture::Segment {
        metadata: ChunkMetadata {
            capture_epoch_id: None,
            id: format!("{recording}-mic_local-{sequence}"),
            recording_id: recording.into(),
            track: "mic_local".into(),
            sequence,
            sample_rate: 8000,
            channels: 1,
            frames: 8000,
            monotonic_ns: sequence.saturating_mul(1_000_000_000),
            first_sample: sequence.saturating_mul(8000),
            started_at_ms: 1000,
        },
        pcm: vec![7; 16000],
    }
}

#[tokio::test]
async fn run_preserves_typed_results_and_commits_on_the_writer() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (runtime, _) = fixture(directory.path())?;
    let job = orbit_store::Job {
        id: "finalize-typed-run".into(),
        kind: "recording_finalize".into(),
        state: orbit_store::JobState::Queued,
        created_at: 1,
        payload: json!({"recording_id":"typed-run"}),
    };
    let result = runtime
        .store_actor
        .run(move |store| {
            store.enqueue_job(&job)?;
            Ok(17_u16)
        })
        .await?;
    assert_eq!(result, 17);
    assert!(
        runtime
            .fixture_store
            .get_job("finalize-typed-run")?
            .is_some()
    );
    Ok(())
}

/// A seal that cannot reach the disk for seconds must not occupy the
/// actor loop. One hundred events per second keep arriving, and the person's
/// Status query still answers while the writer remains blocked.
#[tokio::test]
async fn a_delayed_seal_does_not_stall_the_actor_loop() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    let stream = start_recording(&mut runtime, &mut receiver, "delayed-seal").await?;
    let probe = snapshot_probe(&runtime)?;
    hold_store_writer(&probe)?;
    // 110 frames of 10 ms: the hundredth completes one second of PCM and
    // reaches the delayed seal; the rest keep arriving while it is stuck.
    let mut failure = None;
    for sequence in 0..110u64 {
        let outcome = runtime
            .native_event(audio_frame(&stream, sequence, 80))
            .await;
        if let Err(error) = outcome {
            failure = Some(error);
            break;
        }
    }
    assert!(
        failure.is_none(),
        "audio must be accepted while the seal is delayed: {failure:?}"
    );
    runtime.command(Command::Status).await?;
    assert!(
        !runtime.state.privacy_stopped(),
        "a delayed seal is backpressure on the store, never an integrity brake"
    );
    while let Ok(command) = receiver.try_recv() {
        assert_ne!(
            command["command"], "privacy_stop",
            "the helper must not be asked to stop while a seal is delayed"
        );
    }
    release_store_writer(&probe)?;
    Ok(())
}

/// Forty seals may be waiting on the writer, but a Status turn
/// only reports receipts that are already available. It must not spend part
/// of the IPC caller's deadline waiting for a future disk receipt.
///
/// SQLite's writer lock is the deterministic slow-disk boundary: it stops the
/// same transaction that contains `seal_chunk` without a production sleep or
/// an effect on the host. The shared production wait is asserted as zero, and
/// every command must finish on its first poll, so neither assertion depends
/// on how quickly the loaded test machine happens to run.
///
/// What this test cannot see: a literal wait hardcoded in the Status arm.
/// `collect_ready_seals` parks inside the poll, so even a 75 ms wait finishes
/// its first poll `Ready` and these assertions stay green — a review proved it
/// by mutating the arm that way. This test fixes the constant's value and
/// measures the wired behaviour.
///
#[tokio::test]
async fn status_does_not_wait_for_forty_slow_seals() -> Result<()> {
    assert_eq!(
        sealing::OPPORTUNISTIC_RECEIPT_WAIT,
        Duration::ZERO,
        "an IPC status turn may only collect receipts already present"
    );
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    start_recording(&mut runtime, &mut receiver, "status-seals").await?;
    // Pay every setup write before the probe takes SQLite's writer slot.
    runtime.store_actor.snapshot(&runtime.state).await?;
    runtime.seal(
        (0..5)
            .map(|sequence| pending_segment("status-seals", sequence))
            .collect(),
    )?;
    await_seal_receipts(&mut runtime)?;
    let probe = snapshot_probe(&runtime)?;
    hold_store_writer(&probe)?;
    runtime.seal(
        (5..40)
            .map(|sequence| pending_segment("status-seals", sequence))
            .collect(),
    )?;

    for _ in 0..10 {
        let outcome = {
            let mut status = std::pin::pin!(runtime.command(Command::Status));
            let mut context = std::task::Context::from_waker(std::task::Waker::noop());
            status.as_mut().poll(&mut context)
        };
        assert!(
            matches!(outcome, std::task::Poll::Ready(Ok(()))),
            "Status waited for a future disk receipt: {outcome:?}"
        );
        assert_eq!(
            runtime
                .state
                .durable
                .recording
                .as_ref()
                .context("recording absent")?
                .sealed_chunks,
            5,
            "Status may only report durable receipts; SQLite is still locked"
        );
    }
    release_store_writer(&probe)?;
    await_seal_receipts(&mut runtime)?;
    assert_eq!(
        runtime
            .state
            .durable
            .recording
            .as_ref()
            .context("recording absent")?
            .sealed_chunks,
        40,
        "every admitted seal must be counted after its durable receipt"
    );
    let mut sequences = runtime
        .fixture_store
        .list_chunks("status-seals", None, 100)?
        .into_iter()
        .map(|receipt| receipt.metadata.sequence)
        .collect::<Vec<_>>();
    sequences.sort_unstable();
    assert_eq!(sequences, (0..40).collect::<Vec<_>>());
    Ok(())
}

/// The maintenance tick must not start an upload admission that the ordered
/// writer will correctly place behind seals already handed to it. The
/// first poll is the daemon loop's turn: it must finish from RAM, without a
/// clock deciding whether the store was fast enough.
#[tokio::test]
async fn upload_scheduler_does_not_await_behind_admitted_seals() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    start_recording(&mut runtime, &mut receiver, "schedule-seals").await?;
    runtime.seal(vec![pending_segment("schedule-seals", 0)])?;
    await_seal_receipts(&mut runtime)?;
    runtime.token = Some("synthetic-token".into());
    runtime.settings.hub_url = Some("http://127.0.0.1:1".into());

    let probe = snapshot_probe(&runtime)?;
    hold_store_writer(&probe)?;
    runtime.seal(
        (1..40)
            .map(|sequence| pending_segment("schedule-seals", sequence))
            .collect(),
    )?;
    runtime
        .state
        .durable
        .recording
        .as_mut()
        .context("recording absent")?
        .phase = CapturePhase::Finalizing;
    // The chunks go up from the capture task, whose admission waits for the
    // held writer on its own thread.
    let (mut task, _bridge) = crate::daemon::capture_lane::test_support::spawn_task(&runtime)?;
    let outcome = {
        let mut scheduling = std::pin::pin!(runtime.schedule_upload());
        let mut context = std::task::Context::from_waker(std::task::Waker::noop());
        scheduling.as_mut().poll(&mut context)
    };
    release_store_writer(&probe)?;
    await_seal_receipts(&mut runtime)?;
    assert!(
        matches!(outcome, std::task::Poll::Ready(Ok(()))),
        "the upload scheduler held the daemon turn behind admitted seals: {outcome:?}"
    );
    // Its transfer was tried once the writer let it: nothing listens there.
    crate::daemon::capture_lane::test_support::job_reaches(
        &runtime,
        "upload-schedule-seals-mic_local-0",
        JobState::RetryWait,
    )
    .await?;
    task.stop();
    Ok(())
}

/// Collecting durable seal receipts changes the in-memory
/// projection. Its deferred snapshot must be recorded as admitted, otherwise
/// the immediately following maintenance Tick tries to wait for the writer.
#[tokio::test]
async fn tick_does_not_wait_for_a_snapshot_behind_seals() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    start_recording(&mut runtime, &mut receiver, "tick-seals").await?;
    runtime.store_actor.snapshot(&runtime.state).await?;
    runtime.seal(
        (0..5)
            .map(|sequence| pending_segment("tick-seals", sequence))
            .collect(),
    )?;
    await_seal_receipts(&mut runtime)?;
    let probe = snapshot_probe(&runtime)?;
    hold_store_writer(&probe)?;
    runtime.seal(
        (5..40)
            .map(|sequence| pending_segment("tick-seals", sequence))
            .collect(),
    )?;

    let outcome = {
        let mut tick = std::pin::pin!(runtime.apply(Event::Tick));
        let mut context = std::task::Context::from_waker(std::task::Waker::noop());
        tick.as_mut().poll(&mut context)
    };

    release_store_writer(&probe)?;
    await_seal_receipts(&mut runtime)?;
    assert!(
        matches!(outcome, std::task::Poll::Ready(Ok(()))),
        "apply(Tick) did not finish successfully behind 35 seals: {outcome:?}"
    );
    Ok(())
}

/// Durable seal rows read from outside the daemon, in the
/// fixture's own database: the fact these probes wait for, independent of
/// both channels and of any clock.
fn durable_seals(probe: &rusqlite::Connection) -> Result<u64> {
    let count: i64 = probe.query_row("SELECT count(*) FROM chunks", [], |row| row.get(0))?;
    Ok(count.max(0) as u64)
}

/// Waits on the durable rows themselves. The bound is the
/// measured stall guard for a seal's durable receipt, so it never decides a
/// passing run; it only names a writer that stopped advancing, with the count
/// it stopped at.
fn wait_for_durable_seals(probe: &rusqlite::Connection, at_least: u64) -> Result<()> {
    let stalled = std::time::Instant::now() + SEAL_RECEIPT_GUARD;
    loop {
        let durable = durable_seals(probe)?;
        if durable >= at_least {
            return Ok(());
        }
        if std::time::Instant::now() >= stalled {
            bail!(
                "the writer stopped at {durable} durable seal(s); wanted {at_least} within the measured receipt guard"
            );
        }
        std::thread::park_timeout(Duration::from_millis(1));
    }
}

/// The receiver is drained an exact, counted number of
/// times, so a writer that blocks on a full wake-up channel can never fit its
/// last notice: the production capacity plus these drains carries at most
/// `EXPECTED - 1` of the `EXPECTED` notices (16 + 3 = 19 < 20). The healthy
/// writer drops the excess notice and keeps going — its receipt lives in the
/// actor's completion log, not in the channel — and that is what the probes
/// below assert. The earlier version drained whenever the channel looked
/// full, which freed a slot before the twentieth notice: a blocking writer
/// sailed through, and the deadlock went uncaught (3 of 3 mutated runs
/// passed).
async fn await_durable_seals(
    probe: &rusqlite::Connection,
    work_receiver: &mut mpsc::Receiver<WorkResult>,
) -> Result<()> {
    const EXPECTED: usize = 20;
    // 16 channel slots + 3 drains = 19 < 20 notices, recomputed if the
    // production capacity ever changes.
    const DRAINS: usize = EXPECTED - WORK_EVENTS_CAPACITY - 1;
    let before = durable_seals(probe)?;
    // One durable row more than the channel holds notices: exactly where a
    // blocking writer stands stuck, and where a dropping writer has already
    // proven that coalescing is safe.
    wait_for_durable_seals(probe, before + WORK_EVENTS_CAPACITY as u64 + 1)?;
    for _ in 0..DRAINS {
        work_receiver
            .recv()
            .await
            .context("expected a queued seal wake-up")?;
    }
    wait_for_durable_seals(probe, before + EXPECTED as u64)?;
    Ok(())
}

/// Bounds only the two deadlock probes (a writer blocked sending into the full
/// wake-up channel while the daemon awaited it) so a regression cannot hang the
/// suite forever. A review observed no answer in 20 s on the blocking writer,
/// while the fixed writer answers an await behind 20 seals in 7–28 µs and a
/// privacy stop in 293–981 ms (measured over two later reviews); this
/// preserves that measured discriminator at over 800 times the healthy
/// control. It is not a daemon responsiveness budget or a disk deadline.
const SEAL_DEADLOCK_GUARD: Duration = Duration::from_secs(20);

/// This is the production work-channel capacity and a live
/// receiver which the daemon cannot drain while it awaits the writer.
/// [`await_durable_seals`] frees exactly three slots and no more, so the
/// twentieth notice can never fit a writer that blocks. The timeout is only a
/// deadlock guard: it returns the instant the actor receipt arrives, and on
/// the old blocking writer it fails bounded, naming the seals and the queue.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn actor_wait_behind_twenty_seals_does_not_need_the_event_receiver() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut native_receiver) = fixture(directory.path())?;
    start_recording(&mut runtime, &mut native_receiver, "live-channel").await?;
    let (events, mut work_receiver) = tokio::sync::mpsc::channel(WORK_EVENTS_CAPACITY);
    let store = crate::daemon::test_store::open_existing(&runtime.paths.store, [0x42; 32])?;
    runtime.store_actor = crate::daemon::store_actor::StoreActor::spawn(store, events)?;
    let probe = rusqlite::Connection::open(runtime.paths.store.join("orbit.sqlite3"))?;
    runtime.seal(
        (0..20)
            .map(|sequence| pending_segment("live-channel", sequence))
            .collect(),
    )?;
    let mut waiting = std::pin::pin!(runtime.store_actor.run(|_store| Ok(())));
    hand_over(&mut waiting);
    await_durable_seals(&probe, &mut work_receiver).await?;
    let answer = tokio::time::timeout(SEAL_DEADLOCK_GUARD, &mut waiting).await;
    let queued = work_receiver.len();
    drop(work_receiver);

    assert!(
        matches!(answer, Ok(Ok(()))),
        "the actor wait did not answer behind 20 seals within {SEAL_DEADLOCK_GUARD:?}; \
         a writer blocked sending into the full wake-up channel ({queued} events queued) \
         reproduces exactly this: {answer:?}"
    );
    let mut sequences = runtime
        .fixture_store
        .list_chunks("live-channel", None, 100)?
        .into_iter()
        .map(|receipt| receipt.metadata.sequence)
        .collect::<Vec<_>>();
    sequences.sort_unstable();
    assert_eq!(
        sequences,
        (0..20u64).collect::<Vec<_>>(),
        "the ordered writer lost or duplicated a seal"
    );
    Ok(())
}

/// Privacy stop uses the same production wiring. It must
/// durably latch and notify the helper even when more seals precede its store
/// operation than the wake-up channel can carry; the three freed slots of
/// [`await_durable_seals`] leave the twentieth notice without room, so a
/// blocking writer holds the stop's own snapshot instead.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn privacy_stop_behind_twenty_seals_finishes_and_reaches_the_helper() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut native_receiver) = fixture(directory.path())?;
    start_recording(&mut runtime, &mut native_receiver, "privacy-channel").await?;
    let (events, mut work_receiver) = tokio::sync::mpsc::channel(WORK_EVENTS_CAPACITY);
    let store = crate::daemon::test_store::open_existing(&runtime.paths.store, [0x42; 32])?;
    runtime.store_actor = crate::daemon::store_actor::StoreActor::spawn(store, events)?;
    let probe = rusqlite::Connection::open(runtime.paths.store.join("orbit.sqlite3"))?;
    runtime.seal(
        (0..20)
            .map(|sequence| pending_segment("privacy-channel", sequence))
            .collect(),
    )?;
    await_durable_seals(&probe, &mut work_receiver).await?;
    let (stopped, queued) = {
        let mut stopping = std::pin::pin!(runtime.command(Command::PrivacyStop));
        hand_over(&mut stopping);
        let stopped = tokio::time::timeout(SEAL_DEADLOCK_GUARD, &mut stopping).await;
        (stopped, work_receiver.len())
    };
    drop(work_receiver);
    let mut helper_stopped = false;
    while let Ok(command) = native_receiver.try_recv() {
        helper_stopped |= command["command"] == "privacy_stop";
    }

    assert!(
        matches!(stopped, Ok(Ok(()))) && runtime.state.privacy_stopped() && helper_stopped,
        "privacy stop did not latch and reach the helper behind 20 seals within \
         {SEAL_DEADLOCK_GUARD:?}; a writer blocked sending into the full wake-up channel \
         ({queued} events queued) reproduces exactly this: {stopped:?}"
    );
    Ok(())
}

/// After the final seal receipt, admitting its upload still writes
/// `Running` through the actor. That receipt is worker work; the daemon turn
/// must end immediately even when the actor is occupied.
#[tokio::test]
async fn upload_admission_waits_off_loop_after_the_last_seal() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    start_recording(&mut runtime, &mut receiver, "last-seal-admission").await?;
    runtime.seal(vec![pending_segment("last-seal-admission", 0)])?;
    await_seal_receipts(&mut runtime)?;
    runtime.token = Some("synthetic-token".into());
    runtime.settings.hub_url = Some("http://127.0.0.1:1".into());

    let release = hold_the_actor(&runtime)?;
    // The capture task admits the upload while the writer is held, on its
    // own thread; the loop's turn ends at once.
    let (mut task, _bridge) = crate::daemon::capture_lane::test_support::spawn_task(&runtime)?;
    let outcome = {
        let mut scheduling = std::pin::pin!(runtime.schedule_upload());
        let mut context = std::task::Context::from_waker(std::task::Waker::noop());
        scheduling.as_mut().poll(&mut context)
    };
    release.send(())?;
    assert!(
        matches!(outcome, std::task::Poll::Ready(Ok(()))),
        "upload admission held the loop after the last seal: {outcome:?}"
    );
    crate::daemon::capture_lane::test_support::job_reaches(
        &runtime,
        "upload-last-seal-admission-mic_local-0",
        JobState::RetryWait,
    )
    .await?;
    task.stop();
    Ok(())
}

/// A transfer that was already in flight may finish while new audio
/// is recording. Its local receipt waits off-loop; handling the work event
/// itself completes in the current turn even after the last seal, and the job
/// state becomes durable once the actor is released.
#[tokio::test]
async fn upload_result_settles_off_loop_after_last_seal() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    start_recording(&mut runtime, &mut receiver, "last-seal-result").await?;
    runtime.seal(vec![pending_segment("last-seal-result", 0)])?;
    await_seal_receipts(&mut runtime)?;
    let upload = "upload-last-seal-result-mic_local-0";
    let upload_id = upload.to_owned();
    runtime
        .store_actor
        .run(move |store| Ok(store.update_job(&upload_id, JobState::Running)?))
        .await?;
    let generation = runtime.state.generation();
    let ticket = runtime
        .work
        .worker
        .attach_test(generation, (), tokio::spawn(async {}));

    let release = hold_the_actor(&runtime)?;
    let outcome = {
        let mut settling = std::pin::pin!(runtime.work_event(WorkResult::Store(
            crate::daemon::StoreWork::Uploaded {
                ticket,
                job_id: upload.into(),
                result: Ok(()),
            },
        )));
        let mut context = std::task::Context::from_waker(std::task::Waker::noop());
        settling.as_mut().poll(&mut context)
    };
    release.send(())?;
    assert!(
        matches!(outcome, std::task::Poll::Ready(Ok(()))),
        "applying an upload result held the daemon turn behind admitted seals: {outcome:?}"
    );
    runtime
        .work
        .worker
        .take()
        .context("missing durable settlement worker")?
        .await?;
    assert_eq!(
        runtime.fixture_store.get_job(upload)?.map(|job| job.state),
        Some(JobState::Succeeded)
    );
    Ok(())
}

/// The settlement receipt releases the only upload-worker
/// slot. Keeping a finished handle there makes every later upload ineligible.
#[tokio::test]
async fn upload_settlement_releases_the_worker_slot() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _receiver) = fixture(directory.path())?;
    let generation = runtime.state.generation();
    let ticket = runtime
        .work
        .worker
        .attach_test(generation, (), tokio::spawn(async {}));

    runtime
        .work_event(WorkResult::Store(crate::daemon::StoreWork::UploadSettled {
            ticket,
            succeeded: false,
            error: None,
            notice: None,
            result: Ok(()),
        }))
        .await?;

    assert!(
        runtime.work.worker.active().is_none(),
        "UploadSettled kept the finished worker slot occupied"
    );
    Ok(())
}

/// Chat deltas are not durable state. Two hundred of them write no
/// snapshot; the completed turn writes exactly one.
#[tokio::test]
async fn delta_bursts_write_no_snapshot_and_a_completed_turn_writes_one() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _) = fixture(directory.path())?;
    let (hub, _commands) = mpsc::channel(16);
    runtime.hub.install(hub);
    runtime
        .apply(Event::Submit {
            attachments: Vec::new(),
            id: "delta-burst-request".into(),
            text: "SYNTHETIC QUESTION".into(),
        })
        .await?;
    runtime
        .apply(Event::Session {
            id: "delta-burst-session".into(),
            agent_id: None,
        })
        .await?;
    let probe = snapshot_probe(&runtime)?;
    // Run flushes every snapshot admitted before it and adds no snapshot.
    runtime.store_actor.run(|_| Ok(())).await?;
    let baseline = snapshot_writes(&probe)?;
    for index in 0..200 {
        runtime
            .apply(Event::Delta {
                request_id: "delta-burst-request".into(),
                text: format!("SYNTHETIC PARTIAL {index}"),
            })
            .await?;
    }
    runtime.store_actor.run(|_| Ok(())).await?;
    let after_deltas = snapshot_writes(&probe)?;
    assert_eq!(
        after_deltas,
        baseline,
        "200 Delta events wrote {} snapshot(s)",
        after_deltas - baseline
    );
    runtime
        .apply(Event::Completed {
            request_id: "delta-burst-request".into(),
            text: Some("SYNTHETIC ANSWER".into()),
        })
        .await?;
    runtime.store_actor.run(|_| Ok(())).await?;
    let final_count = snapshot_writes(&probe)?;
    assert_eq!(
        final_count - baseline,
        1,
        "the completed turn wrote {} snapshot(s); exactly one is allowed",
        final_count - baseline
    );
    Ok(())
}

/// Sixty idle ticks neither write a snapshot nor wait on the store.
#[tokio::test]
async fn sixty_idle_ticks_write_nothing_and_never_wait_on_the_store() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _) = fixture(directory.path())?;
    runtime
        .apply(Event::Session {
            id: "idle-ticks-session".into(),
            agent_id: None,
        })
        .await?;
    let probe = snapshot_probe(&runtime)?;
    runtime.store_actor.run(|_| Ok(())).await?;
    let baseline = snapshot_writes(&probe)?;
    hold_store_writer(&probe)?;
    for index in 0..60 {
        let outcome = runtime.apply(Event::Tick).await;
        assert!(
            outcome.is_ok(),
            "idle tick {index} reached the store: {outcome:?}"
        );
    }
    release_store_writer(&probe)?;
    runtime.store_actor.run(|_| Ok(())).await?;
    assert_eq!(
        snapshot_writes(&probe)?,
        baseline,
        "60 idle ticks wrote snapshots"
    );
    Ok(())
}

/// `sealed_chunks` waits for the durable receipt. While no receipt can
/// exist the counter must not move; once the store is free the segment must
/// become durable on the storage actor's own thread.
///
#[tokio::test]
async fn sealed_chunks_wait_for_the_durable_receipt() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    start_recording(&mut runtime, &mut receiver, "durable-receipt").await?;
    let probe = snapshot_probe(&runtime)?;
    hold_store_writer(&probe)?;
    let admitted = runtime.seal(vec![pending_segment("durable-receipt", 0)]);
    assert!(
        admitted.is_ok(),
        "handing PCM to the storage actor must not wait on the disk: {admitted:?}"
    );
    assert_eq!(
        runtime
            .state
            .durable
            .recording
            .as_ref()
            .context("recording absent")?
            .sealed_chunks,
        0,
        "no durable receipt exists, so sealed_chunks must not advance"
    );
    release_store_writer(&probe)?;
    runtime.store_actor.snapshot(&runtime.state).await?;
    let chunk = "durable-receipt-mic_local-0";
    // A Run is ordered after every earlier seal and flushes a prior snapshot.
    // Its receipt means the real writer has finished the admitted operations.
    runtime.store_actor.run(|_| Ok(())).await?;
    let receipts = stored_chunks(&probe, chunk)?;
    assert_eq!(
        receipts, 1,
        "the segment must reach a durable receipt once the store is free"
    );
    assert_eq!(
        runtime
            .state
            .durable
            .recording
            .as_ref()
            .context("recording absent")?
            .sealed_chunks,
        0,
        "the counter advances when the Sealed receipt event is applied, never before"
    );
    runtime
        .work_event(WorkResult::Store(crate::daemon::StoreWork::Sealed {
            recording: "durable-receipt".into(),
            sequence: 0,
        }))
        .await?;
    assert_eq!(
        runtime
            .state
            .durable
            .recording
            .as_ref()
            .context("recording absent")?
            .sealed_chunks,
        1,
        "applying the durable Sealed receipt advances the counter once"
    );
    Ok(())
}

/// The 64 MiB volatile reserve and its loss declaration survive the
/// actor split: the overflowing segment is dropped with one declared gap and
/// nothing counts as sealed.
#[tokio::test]
async fn exhausting_the_64mib_reserve_still_declares_the_loss() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    start_recording(&mut runtime, &mut receiver, "reserve-overflow").await?;
    let overflow = runtime.seal(
        (0..33u64)
            .map(|sequence| {
                let mut segment = pending_segment("reserve-overflow", sequence);
                segment.pcm = vec![7; 2 * 1024 * 1024];
                segment
            })
            .collect(),
    );
    assert!(
        overflow.is_err(),
        "exhausting the reserve must refuse the batch"
    );
    assert_eq!(
        runtime.state.notice_code(),
        Some(NoticeCode::AudioReserveExhausted),
        "the loss must be declared: {:?}",
        runtime.state.durable.notice
    );
    let recording = runtime
        .state
        .durable
        .recording
        .as_ref()
        .context("recording absent")?;
    assert_eq!(
        recording.gaps, 1,
        "the dropped segment is exactly one declared gap"
    );
    assert_eq!(
        recording.sealed_chunks, 0,
        "nothing sealed while the reserve overflowed"
    );
    Ok(())
}

/// Polls a store operation once, which is when it is handed to the
/// actor, and returns it still pending so the caller decides when to await it.
fn hand_over<F: std::future::Future>(operation: &mut std::pin::Pin<&mut F>) {
    let mut context = std::task::Context::from_waker(std::task::Waker::noop());
    assert!(
        operation.as_mut().poll(&mut context).is_pending(),
        "the operation finished before the actor was released"
    );
}

/// Holds the store actor's thread inside a `Run` until the returned
/// sender is dropped or signalled, so that every operation queued meanwhile is
/// found by the actor together, in one drain. What it then executes first is
/// decided by its ordering alone, never by which thread woke up first: the
/// wait is for the actor having entered the `Run`, not for a deadline.
pub(super) fn hold_the_actor(runtime: &Runtime) -> Result<std::sync::mpsc::Sender<()>> {
    let (entered, inside) = std::sync::mpsc::channel();
    let (release, released) = std::sync::mpsc::channel::<()>();
    let mut holding = std::pin::pin!(runtime.store_actor.run(move |_store| {
        let _ = entered.send(());
        let _ = released.recv();
        Ok(())
    }));
    hand_over(&mut holding);
    // The receipt of the holding `Run` is not needed: dropping its future
    // leaves the queued operation to run and discards only the reply.
    inside
        .recv()
        .context("the store actor never entered the holding Run")?;
    Ok(release)
}

#[tokio::test]
async fn a_failed_write_names_its_cause() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (runtime, _helper) = fixture(directory.path())?;
    let failed = runtime
        .store_actor
        .begin_run(|_store| Err::<(), _>(anyhow::anyhow!("synthetic disk refusal")))?
        .wait()
        .await
        .err()
        .context("the refused write was reported as confirmed")?
        .to_string();
    assert!(failed.contains("synthetic disk refusal"), "{failed}");
    Ok(())
}

/// The writer has entered a store operation and cannot answer
/// any snapshot. A single poll is the daemon's turn, independently of host
/// load: the privacy latch and helper order must already exist at that point.
#[tokio::test]
async fn stop_reaches_helper_while_store_actor_is_inside_a_write() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut helper) = fixture(directory.path())?;
    start_recording(&mut runtime, &mut helper, "stop-inside-write").await?;
    let release = hold_the_actor(&runtime)?;
    let outcome = {
        let mut stopping =
            std::pin::pin!(runtime.privacy_stop(orbit_core::privacy_stop::UserStop::Order,));
        let mut context = std::task::Context::from_waker(std::task::Waker::noop());
        stopping.as_mut().poll(&mut context)
    };
    let orders: Vec<Value> = std::iter::from_fn(|| helper.try_recv().ok()).collect();
    let latched = runtime.state.privacy_stopped();
    let _ = release.send(());
    assert!(
        outcome.is_pending(),
        "the fixture must hold a durable write"
    );
    assert!(latched, "the in-memory privacy latch waited for the writer");
    assert!(
        orders
            .iter()
            .any(|order| order["command"] == "privacy_stop"),
        "the helper waited for the writer: {orders:?}"
    );
    let durable: State = runtime
        .store_actor
        .run(|store| Ok(store.load_snapshot()?))
        .await?
        .context("the queued privacy snapshot did not reach storage")?;
    assert!(
        durable.privacy_stopped(),
        "the queued snapshot predated her stop"
    );
    Ok(())
}

#[tokio::test]
async fn stop_retires_playback_and_volatile_work_before_a_blocked_write() -> Result<()> {
    use tokio::io::AsyncBufReadExt;
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut helper) = fixture(directory.path())?;
    start_recording(&mut runtime, &mut helper, "stop-volatile").await?;
    let (_helper_input, daemon_input) = tokio::io::duplex(4096);
    let (daemon_output, helper_output) = tokio::io::duplex(4096);
    let (native, commands) = tokio::sync::mpsc::channel(64);
    let (events, _received_events) = tokio::sync::mpsc::channel(16);
    runtime.native = native;
    let bridge = tokio::spawn(crate::native::bridge(
        tokio::io::BufReader::new(daemon_input),
        daemon_output,
        commands,
        events,
    ));
    let mut written = tokio::io::BufReader::new(helper_output);
    runtime.conversation.speak_next = true;
    let pending_task = || {
        let (held, dropped) = tokio::sync::oneshot::channel::<()>();
        let task = tokio::spawn(async move {
            let _held = held;
            std::future::pending::<()>().await;
        });
        (task, dropped)
    };
    let (worker, worker_dropped) = pending_task();
    let generation = runtime.state.generation();
    runtime.work.worker.attach_test(generation, (), worker);
    let release = hold_the_actor(&runtime)?;
    let outcome = {
        let mut stopping =
            std::pin::pin!(runtime.privacy_stop(orbit_core::privacy_stop::UserStop::Order));
        let mut context = std::task::Context::from_waker(std::task::Waker::noop());
        stopping.as_mut().poll(&mut context)
    };
    let orders = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        let mut orders = Vec::new();
        while !orders
            .iter()
            .any(|order: &Value| order["command"] == "privacy_stop")
            || !orders
                .iter()
                .any(|order: &Value| order["command"] == "playback_stop")
        {
            let mut line = String::new();
            written.read_line(&mut line).await?;
            orders.push(serde_json::from_str::<Value>(&line)?);
        }
        anyhow::Ok(orders)
    })
    .await;
    let worker_retired = runtime.work.worker.active().is_none();
    let playback_retired = !runtime.conversation.speak_next;
    let _ = release.send(());
    let orders = orders.context("helper pipe did not receive both stop orders")??;
    let dropped = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        tokio::join!(worker_dropped)
    })
    .await
    .context("one retired task kept running after the disk was blocked")?;
    assert!(outcome.is_pending(), "fixture did not block the writer");
    assert!(
        orders
            .iter()
            .any(|order| order["command"] == "privacy_stop")
    );
    assert!(
        orders
            .iter()
            .any(|order| order["command"] == "playback_stop"),
        "helper orders: {orders:?}"
    );
    assert!(
        worker_retired && playback_retired,
        "volatile work survived a blocked writer"
    );
    assert!(dropped.0.is_err());
    bridge.abort();
    Ok(())
}

#[tokio::test]
async fn reentered_stop_is_the_users_even_when_retirement_runs_out_of_time() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _helper) = fixture(directory.path())?;
    {
        let fixture_value = serde_json::json!(true);
        crate::daemon::authority_fixture::set(&mut runtime.state, "privacy_stopped", fixture_value);
    }
    {
        let fixture_value =
            serde_json::json!(Some(orbit_core::privacy_stop::StopRequester::Revocation));
        crate::daemon::authority_fixture::set(&mut runtime.state, "privacy_stop_by", fixture_value);
    }
    let release = hold_the_actor(&runtime)?;
    let outcome = {
        let mut stopping =
            std::pin::pin!(runtime.privacy_stop(orbit_core::privacy_stop::UserStop::Order));
        let mut context = std::task::Context::from_waker(std::task::Waker::noop());
        stopping.as_mut().poll(&mut context)
    };
    let by = runtime.state.privacy_stop_by();
    release.send(())?;
    assert_eq!(by, Some(orbit_core::privacy_stop::StopRequester::User));
    assert!(
        outcome.is_pending(),
        "fixture did not hold the durable retirement"
    );
    Ok(())
}

#[tokio::test]
async fn owed_retirement_persists_after_the_writer_recovers() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _helper) = fixture(directory.path())?;
    crate::daemon::turn::said::trace(&mut runtime, directory.path());
    let release = hold_the_actor(&runtime)?;
    // The writer remains held until the real ten-second durable budget
    // expires. The 60 s outer guard only names a lost timeout on this shared
    // loaded host; it does not decide whether the stop or recovery passes.
    let stopped = tokio::time::timeout(
        std::time::Duration::from_secs(60),
        runtime.privacy_stop(orbit_core::privacy_stop::UserStop::Order),
    )
    .await;
    if stopped.is_err() {
        release.send(())?;
        anyhow::bail!("privacy stop did not return after its durable budget");
    }
    assert!(runtime.state.privacy_stopped());
    assert!(runtime.privacy_retirement_owed);
    // A privacy stop retains its pending-work diagnostic without
    // asking the user to keep open an app they have not asked to close.
    assert!(!crate::daemon::turn::said::has(
        &runtime.state,
        NoticeCode::PendingWorkKeepOpen
    ));
    let logged = std::fs::read_to_string(directory.path().join("logs/daemon.log"))?;
    assert!(logged.contains("stop_durable_pending"), "{logged}");
    assert!(logged.contains("budget_s"), "{logged}");
    release.send(())?;
    runtime.run_maintenance().await;
    let saved: State = runtime
        .store_actor
        .run(|store| Ok(store.load_snapshot()?))
        .await?
        .context("retirement did not reach the writer")?;
    assert!(!runtime.privacy_retirement_owed);
    assert!(saved.privacy_stopped());
    Ok(())
}

#[tokio::test]
async fn privacy_snapshot_waits_for_writer_without_holding_maintenance() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _helper) = fixture(directory.path())?;
    runtime.store_actor.snapshot(&runtime.state).await?;
    let release = hold_the_actor(&runtime)?;
    let stopped = tokio::time::timeout(
        std::time::Duration::from_secs(60),
        runtime.privacy_stop(orbit_core::privacy_stop::UserStop::Order),
    )
    .await;
    if stopped.is_err() {
        release.send(())?;
        anyhow::bail!("privacy stop did not return after its durable budget");
    }
    assert!(runtime.state.privacy_stopped());
    assert!(runtime.privacy_retirement_owed);
    // The stop already queued its snapshot. Isolate retirement's retry from
    // maintain_state's separate snapshot policy, as the original fixture did.
    runtime.work.durable_snapshot = DurableSnapshot::from(&runtime.state);
    let tick =
        tokio::time::timeout(std::time::Duration::from_secs(1), runtime.run_maintenance()).await;
    let reader = Store::open_existing(runtime.paths.store.clone(), [0x42; 32])?;
    let blocked: State = reader.load_snapshot()?.context("initial snapshot absent")?;
    assert!(
        !blocked.privacy_stopped(),
        "fixture did not hold the privacy snapshot"
    );
    release.send(())?;
    assert!(
        tick.is_ok(),
        "one maintenance tick waited for the ten-second closure budget"
    );
    let saved: State = runtime
        .store_actor
        .run(|store| Ok(store.load_snapshot()?))
        .await?
        .context("privacy snapshot was lost behind the blocked writer")?;
    assert!(saved.privacy_stopped());
    assert_eq!(
        saved.privacy_stop_by(),
        Some(orbit_core::privacy_stop::StopRequester::User)
    );
    Ok(())
}

#[tokio::test]
async fn closing_does_not_retry_an_owed_privacy_retirement() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _helper) = fixture(directory.path())?;
    {
        let fixture_value = serde_json::json!(true);
        crate::daemon::authority_fixture::set(
            &mut runtime.state,
            "capabilities.native_helper",
            fixture_value,
        );
    }
    runtime.source_windows().await?;
    assert!(runtime.source_operation_pending());
    {
        let fixture_value = serde_json::json!(true);
        crate::daemon::authority_fixture::set(&mut runtime.state, "privacy_stopped", fixture_value);
    }
    runtime.privacy_retirement_owed = true;
    runtime.closing = true;
    runtime.work.durable_snapshot = DurableSnapshot::from(&runtime.state);
    let release = hold_the_actor(&runtime)?;
    let tick =
        tokio::time::timeout(std::time::Duration::from_secs(1), runtime.run_maintenance()).await;
    release.send(())?;
    assert!(
        tick.is_ok(),
        "closing retried a privacy retirement behind the writer"
    );
    assert!(runtime.privacy_retirement_owed);
    Ok(())
}

/// Closing shares the same physical ordering as the user's stop. A
/// second signal can only accelerate an exit after the helper was told to
/// close capture, including when the store is still inside a system call.
#[tokio::test]
async fn closing_orders_helper_before_waiting_for_the_writer() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut helper) = fixture(directory.path())?;
    start_recording(&mut runtime, &mut helper, "close-inside-write").await?;
    let release = hold_the_actor(&runtime)?;
    let outcome = {
        let mut closing = std::pin::pin!(runtime.close_for_exit(
            orbit_core::privacy_stop::StopCause::Shutdown,
            "Orbit se cierra por SIGTERM.",
        ));
        let mut context = std::task::Context::from_waker(std::task::Waker::noop());
        closing.as_mut().poll(&mut context)
    };
    let orders: Vec<Value> = std::iter::from_fn(|| helper.try_recv().ok()).collect();
    let closing = runtime.closing;
    let _ = release.send(());
    assert!(
        outcome.is_pending(),
        "the fixture must hold a durable write"
    );
    assert!(closing);
    assert!(
        orders
            .iter()
            .any(|order| order["command"] == "capture_stop"),
        "the helper waited for the writer during closure: {orders:?}"
    );
    Ok(())
}

#[tokio::test]
async fn second_signal_cancels_a_close_waiting_on_the_writer() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut helper) = fixture(directory.path())?;
    runtime.diagnostics = Some(crate::diagnostics::DiagnosticLog::in_root(directory.path()));
    start_recording(&mut runtime, &mut helper, "close-second-signal").await?;
    let release = hold_the_actor(&runtime)?;
    let mut closing = shutdown::Shutdown::default();
    closing.signal();
    let (signal, received) = tokio::sync::oneshot::channel::<()>();
    let result = {
        let mut close = std::pin::pin!(closing.close_until_exit(&mut runtime, async {
            let _ = received.await;
            "SIGTERM"
        }));
        let mut context = std::task::Context::from_waker(std::task::Waker::noop());
        assert!(close.as_mut().poll(&mut context).is_pending());
        let orders: Vec<Value> = std::iter::from_fn(|| helper.try_recv().ok()).collect();
        assert!(
            orders
                .iter()
                .any(|order| order["command"] == "capture_stop")
        );
        let _ = signal.send(());
        close.await
    };
    assert!(matches!(
        result,
        shutdown::CloseWait::SecondSignal("SIGTERM")
    ));
    runtime
        .declare_bounded_exit_for(shutdown::BoundedExitCause::SecondSignal("SIGTERM"))
        .await?;
    assert!(runtime.store_actor.bounded_exit_declared());
    // The close with work pending is said; its reason (the second
    // signal, not the budget) is in daemon.log under the same op.
    assert!(crate::daemon::turn::said::has(
        &runtime.state,
        NoticeCode::ClosedWithPendingWork
    ));
    let logged = std::fs::read_to_string(directory.path().join("logs/daemon.log"))?;
    assert!(logged.contains("segunda señal SIGTERM"), "{logged}");
    assert!(!logged.contains("tras 10 s"), "{logged}");
    let _ = release.send(());
    Ok(())
}

/// Twenty 250 ms retry reports represent five seconds of a failed
/// snapshot. The report's changing elapsed field must not evict the stop and
/// recovery instruction or produce a new presentation for each retry.
#[tokio::test]
async fn repeated_snapshot_failures_keep_the_stop_and_recovery_notice() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _helper) = fixture(directory.path())?;
    runtime.state.set_notice(NoticeCode::PrivacyStopped);
    runtime.state.set_notice(NoticeCode::PendingWorkKeepOpen);
    let mut first = None;
    for retry in 0..20 {
        runtime
            .work_event(WorkResult::Store(StoreWork::StoreProblem {
                message: format!(
                    "No se pudo guardar el estado local: disco sin respuesta. \
                     1 fragmento de audio desde hace {} ms; se reintenta cada 5 s.",
                    retry * 250
                ),
            }))
            .await?;
        let view = super::topic(&runtime, orbit_core::topics::TopicId::Notices);
        if retry == 0 {
            first = Some(view);
        } else {
            assert_eq!(
                Some(view),
                first,
                "retry {retry} published an unchanged fault"
            );
        }
    }
    let said = runtime.state.durable.notice_log.records();
    assert!(
        crate::daemon::turn::said::has(&runtime.state, NoticeCode::PrivacyStopped),
        "{said:?}"
    );
    assert!(
        crate::daemon::turn::said::has(&runtime.state, NoticeCode::PendingWorkKeepOpen),
        "{said:?}"
    );
    Ok(())
}

/// A `Run` admitted after a `Seal` sees that seal's write. This is
/// the cancelled voice turn in `dispatch_voice_transcription`: it seals the
/// source and then cancels the upload job that the seal itself creates. When
/// the `Run` went first, the job did not exist yet and the turn failed with
/// `record not found`.
#[tokio::test]
async fn a_run_never_overtakes_a_seal_admitted_before_it() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _receiver) = fixture(directory.path())?;
    let segment = pending_segment("run-after-seal", 0);
    let upload = format!("upload-{}", segment.metadata.id);
    let chunk = segment.metadata.id.clone();
    let release = hold_the_actor(&runtime)?;
    runtime.store_actor.seal(std::sync::Arc::new(segment))?;
    let cancelled = upload.clone();
    let mut cancelling = std::pin::pin!(
        runtime
            .store_actor
            .run(move |store| Ok(store.update_job(&cancelled, JobState::Cancelled)?))
    );
    hand_over(&mut cancelling);
    release.send(())?;
    let outcome = cancelling.await;
    assert!(
        outcome.is_ok(),
        "the Run queued after the Seal ran before it and found no upload job: {outcome:?}"
    );
    assert_eq!(
        runtime.fixture_store.get_job(&upload)?.map(|job| job.state),
        Some(JobState::Cancelled)
    );
    assert_eq!(
        &runtime.fixture_store.read_chunk(&chunk)?[44..],
        &vec![7; 16000]
    );
    Ok(())
}

/// A `Run` admitted after a deferred snapshot finds that snapshot
/// written. Coalescing may delay a snapshot, but never behind a later `Run`.
#[tokio::test]
async fn a_run_never_overtakes_a_snapshot_admitted_before_it() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _receiver) = fixture(directory.path())?;
    let release = hold_the_actor(&runtime)?;
    runtime.state.set_notice(NoticeCode::SessionOpened);
    runtime.store_actor.defer_snapshot(&runtime.state)?;
    let mut reading = std::pin::pin!(
        runtime
            .store_actor
            .run(|store| Ok(store.load_snapshot::<State>()?))
    );
    hand_over(&mut reading);
    release.send(())?;
    let observed = reading.await?.and_then(|state| state.notice_code());
    assert_eq!(
        observed,
        Some(NoticeCode::SessionOpened),
        "the Run queued after the deferred snapshot ran before it"
    );
    Ok(())
}

/// A seal that fails is executed, reported and parked for retry: it
/// holds no later `Run`, which would otherwise wait on a broken disk with no
/// deadline. A retry that was asked for before a `Run` is served before it.
#[tokio::test]
async fn a_failed_seal_is_reported_before_a_later_run_and_retried_in_order() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _receiver) = fixture(directory.path())?;
    let segment = pending_segment("run-after-retry", 0);
    let upload = format!("upload-{}", segment.metadata.id);
    let blocked = runtime
        .paths
        .store
        .join(format!("chunks/{}.orbit", segment.metadata.id));
    std::fs::create_dir(&blocked)?;

    let release = hold_the_actor(&runtime)?;
    runtime.store_actor.seal(std::sync::Arc::new(segment))?;
    let absent = upload.clone();
    let mut reading = std::pin::pin!(
        runtime
            .store_actor
            .run(move |store| Ok(store.get_job(&absent)?))
    );
    hand_over(&mut reading);
    release.send(())?;
    assert_eq!(
        reading.await?,
        None,
        "the seal could not be written, so its upload job cannot exist"
    );
    let failures = runtime
        .store_actor
        .take_completions()
        .into_iter()
        .filter(|completion| matches!(completion, store_actor::StoreCompletion::SealFailed { .. }))
        .count();
    assert_eq!(
        failures, 1,
        "the Run queued after the Seal ran before the seal's attempt was reported"
    );

    std::fs::remove_dir(&blocked)?;
    let release = hold_the_actor(&runtime)?;
    runtime.store_actor.retry_seals()?;
    let present = upload.clone();
    let mut reading = std::pin::pin!(
        runtime
            .store_actor
            .run(move |store| Ok(store.get_job(&present)?))
    );
    hand_over(&mut reading);
    release.send(())?;
    assert_eq!(
        reading.await?.map(|job| job.state),
        Some(JobState::Queued),
        "the Run queued after the retry ran before the parked seal was retried"
    );
    Ok(())
}

/// Delivery guard for a `StoreProblem` notice the writer holds
/// pending: 2 s is eight times the 250 ms pending-delivery retry the fixed
/// writer makes, and shorter than the writer's 5 s seal retry, so the
/// pre-fix writer — which only wakes a notice at a failed attempt — cannot
/// deliver the retry's notice inside it. It only names a notice that never
/// arrives; it never decides a passing run.
const PROBLEM_DELIVERY_GUARD: Duration = Duration::from_secs(2);

/// A store problem the full work channel cannot take is not
/// lost. The writer keeps it pending and delivers it as soon as the daemon's
/// loop frees a slot; before, the notice was dropped at the `try_send`, and
/// the snapshot-refusal variant had no completion-log backup at all — the
/// person could close Orbit believing its state was being persisted.
///
/// The failing seal is observed first through the completion log the writer
/// always records, so the red is the lost notice, never a seal that never
/// ran. The one slot is freed only after that observation, while the seal
/// retry is still five seconds away.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_store_problem_waits_for_room_and_is_delivered() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _native) = fixture(directory.path())?;
    let (events, mut work_receiver) = tokio::sync::mpsc::channel(1);
    events
        .send(WorkResult::Problem {
            ticket: crate::daemon::ticketed::WorkTicket::test(runtime.state.generation()),
            message: "synthetic occupant".into(),
        })
        .await
        .map_err(|_| anyhow::anyhow!("the occupying event could not be queued"))?;
    let store = crate::daemon::test_store::open_existing(&runtime.paths.store, [0x42; 32])?;
    runtime.store_actor = crate::daemon::store_actor::StoreActor::spawn(store, events)?;
    let blocked = runtime
        .paths
        .store
        .join("chunks/problem-waits-mic_local-0.orbit");
    std::fs::create_dir(&blocked)?;
    runtime
        .store_actor
        .seal(std::sync::Arc::new(pending_segment("problem-waits", 0)))?;
    let mut failed = 0;
    await_seal_attempts(&runtime.store_actor, &mut failed, 1)?;
    let occupant = tokio::time::timeout(PROBLEM_DELIVERY_GUARD, work_receiver.recv())
        .await
        .context("the occupying event never arrived")?
        .context("the work channel closed before the slot was freed")?;
    assert!(
        matches!(occupant, WorkResult::Problem { .. }),
        "another event took the only slot first"
    );
    let delivered = tokio::time::timeout(PROBLEM_DELIVERY_GUARD, work_receiver.recv())
        .await
        .map_err(|_| anyhow::anyhow!("the StoreProblem stayed lost after the channel had room"))?
        .context("the work channel closed with a pending notice")?;
    let WorkResult::Store(crate::daemon::StoreWork::StoreProblem { message }) = delivered else {
        bail!("expected the pending StoreProblem, got another event");
    };
    assert!(
        message.starts_with("Audio pendiente en memoria: 1 fragmento"),
        "the delivered notice must carry its cause and numbers: {message}"
    );
    Ok(())
}

/// A stall guard on an answer the daemon owes while its store refuses
/// every write, not a budget. It never decides a passing run: each wait below
/// returns the instant the answer arrives. It only turns an answer that never
/// comes, which is what the actor gave while a parked seal held the snapshot
/// back, into a named failure instead of a hung suite.
///
/// Measured, not chosen round. Over 500 runs of the two snapshot-refusal
/// tests on this 32-core box, 200 of them six at a time and 300 sixteen at a
/// time beside 8 fsync writers, at a load average of 44 to 73: the refused snapshot p99
/// 13.0 ms, worst 17.6 ms; an audio frame p99 0.2 ms, worst 21.1 ms; the
/// privacy stop, which waits on two snapshots, p50 401.9 ms, p99 3 414.0 ms,
/// worst 3 642.2 ms; the shutdown order worst 31.3 ms; the closing snapshot
/// p99 510.4 ms, worst 1 112.7 ms. The guard is 40× the worst of them all,
/// the same factor earlier stall guards of the daemon's tests used.
const REFUSAL_GUARD: Duration = Duration::from_millis(145_688);

const SYNTHETIC_DISK_FAILURE: &str = "synthetic disk failure";

/// Every write the actor attempts for audio and state fails, forever,
/// like a disk that never comes back: a seal's chunk row and upload job, and
/// the snapshot. What `seal_chunk` writes before its row (the sealed file) is
/// left alone; the durable receipt is what can never land.
fn fail_every_write(database: &rusqlite::Connection) -> Result<()> {
    let refuse = |name: &str, event: &str, table: &str| {
        format!(
            "CREATE TRIGGER {name} BEFORE {event} ON {table} \
             BEGIN SELECT RAISE(ABORT, '{SYNTHETIC_DISK_FAILURE}'); END;"
        )
    };
    database.execute_batch(
        &[
            refuse("refuse_chunk_insert", "INSERT", "chunks"),
            refuse("refuse_chunk_update", "UPDATE", "chunks"),
            refuse("refuse_job_insert", "INSERT", "jobs"),
            refuse("refuse_job_update", "UPDATE", "jobs"),
            refuse("refuse_setting_insert", "INSERT", "settings"),
            refuse("refuse_setting_update", "UPDATE", "settings"),
        ]
        .concat(),
    )?;
    Ok(())
}

/// What the writer reported, attempt by attempt. Every failed
/// attempt sends exactly one `StoreProblem`, so the reports are the count of
/// attempts: a seal's names the audio still in memory, a snapshot's names the
/// local state.
#[derive(Default)]
struct Attempts {
    seals: u64,
    snapshots: u64,
    other: Vec<String>,
}

impl Attempts {
    fn read(&mut self, reports: &mut mpsc::Receiver<WorkResult>) {
        while let Ok(report) = reports.try_recv() {
            match report {
                WorkResult::Store(crate::daemon::StoreWork::StoreProblem { message })
                    if message.starts_with("Audio pendiente en memoria") =>
                {
                    self.seals += 1;
                }
                WorkResult::Store(crate::daemon::StoreWork::StoreProblem { message })
                    if message.starts_with("No se pudo guardar el estado local") =>
                {
                    self.snapshots += 1;
                }
                WorkResult::Store(crate::daemon::StoreWork::StoreProblem { message }) => {
                    self.other.push(message);
                }
                _ => self
                    .other
                    .push("a report that is not a store problem".into()),
            }
        }
    }
}

/// Waits for the writer to have made `attempts` seal attempts in all,
/// on the completions production itself collects, never on a deadline. The
/// bound is a stall guard: the retry delay the lane owes, plus the measured
/// stall guard for one seal attempt reaching its outcome.
fn await_seal_attempts(
    actor: &store_actor::StoreActor,
    failed: &mut u64,
    attempts: u64,
) -> Result<()> {
    let guard = store_actor::SEAL_RETRY_DELAY + SEAL_RECEIPT_GUARD;
    let stalled_at = std::time::Instant::now() + guard;
    loop {
        for completion in actor.take_completions() {
            match completion {
                store_actor::StoreCompletion::SealFailed { .. } => *failed += 1,
                store_actor::StoreCompletion::Sealed { id, .. } => {
                    bail!("the disk refuses every write, yet {id} was sealed")
                }
            }
        }
        if *failed >= attempts {
            return Ok(());
        }
        if std::time::Instant::now() >= stalled_at {
            bail!("waited {guard:?} for seal attempt {attempts}; the writer reported {failed}");
        }
        std::thread::park_timeout(Duration::from_millis(1));
    }
}

/// Awaits an answer the daemon owes, bounded by [`REFUSAL_GUARD`].
async fn answer_within<T>(
    what: &str,
    operation: impl std::future::Future<Output = T>,
) -> Result<T> {
    tokio::time::timeout(REFUSAL_GUARD, operation)
        .await
        .map_err(|_| anyhow::anyhow!("{what} got no answer in {REFUSAL_GUARD:?}"))
}

/// The numbers a refusal must carry while seals are parked: how many
/// fragments are still in memory, and for how long, in milliseconds.
fn parked_numbers(message: &str) -> Option<(u64, u64)> {
    let count = message
        .split(" fragmento(s) de audio")
        .next()?
        .rsplit(' ')
        .next()?
        .parse()
        .ok()?;
    let since = message
        .split("desde hace ")
        .nth(1)?
        .split(" ms")
        .next()?
        .parse()
        .ok()?;
    Some((count, since))
}

/// With a seal parked for retry and a snapshot pending, the
/// writer sleeps until its next real deadline. Before the fix the snapshot,
/// due but held back by the parked seal, made the loop compute a deadline of
/// zero and turn without sleeping for the whole `SEAL_RETRY_DELAY`.
///
/// Counted, not timed. Every turn after the first starts because an operation
/// arrived, because a real deadline passed and the turn then makes a write
/// attempt, or because the queue closed. So the turns can never exceed one,
/// plus the operations handed over, plus the attempts the writer reported,
/// plus the closing one. The writer's whole life is the scenario: the seal's
/// first attempt, the snapshot, and the seal's first retry `SEAL_RETRY_DELAY`
/// later, which the test waits for as an event. Measured before the fix: 5.4
/// to 6.6 million turns against a bound of 6, and 3.7 to 4.4 s of the
/// writer's CPU over those 5 s. After: 24 turns against a bound of 26 or 27,
/// with the snapshot attempted every `SNAPSHOT_INTERVAL`, and 0.01 to 0.02 s
/// of CPU.
#[test]
fn a_parked_seal_and_a_pending_snapshot_leave_the_writer_asleep() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let paths = crate::config::Paths::resolve(Some(directory.path().to_owned()))?;
    let store = crate::daemon::test_store::open(&paths.store, [0x42; 32])?;
    // Room for every report of the longest run the stall guards allow: four
    // snapshot attempts a second for 470 s is 1 880, and the seals' few.
    let (events, mut reports) = mpsc::channel(8192);
    let turns = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let mut actor =
        store_actor::StoreActor::spawn_counting(store, events, std::sync::Arc::clone(&turns))?;
    let fault = rusqlite::Connection::open(paths.store.join("orbit.sqlite3"))?;
    fail_every_write(&fault)?;
    let mut failed = 0;

    actor.seal(std::sync::Arc::new(pending_segment("parked-asleep", 0)))?;
    await_seal_attempts(&actor, &mut failed, 1)?;
    actor.defer_snapshot(&State::default())?;
    await_seal_attempts(&actor, &mut failed, 2)?;
    // Closing the queue joins the writer, so its last turn is counted and
    // every report it made is already in the channel.
    drop(actor);
    let mut attempts = Attempts::default();
    attempts.read(&mut reports);
    let turns = turns.load(std::sync::atomic::Ordering::Relaxed);

    let operations = 2;
    let bound = 1 + operations + attempts.seals + attempts.snapshots + 1;
    assert!(
        attempts.other.is_empty(),
        "unexpected reports {:?}",
        attempts.other
    );
    assert!(
        turns <= bound,
        "the writer turned {turns} times for {operations} operations, {} seal \
         attempt(s) and {} snapshot attempt(s); a writer that sleeps between real deadlines \
         turns at most {bound}",
        attempts.seals,
        attempts.snapshots
    );
    Ok(())
}

/// A snapshot the daemon waits for is not held behind a seal
/// parked for retry. With a disk that never comes back, the waiter is refused,
/// and the refusal names the store's own cause and the audio still in memory:
/// how many fragments, and for how long.
#[tokio::test]
async fn a_snapshot_waiter_is_refused_with_the_parked_seals_counted() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _receiver) = fixture(directory.path())?;
    let fault = rusqlite::Connection::open(runtime.paths.store.join("orbit.sqlite3"))?;
    fail_every_write(&fault)?;
    let mut failed = 0;
    runtime
        .store_actor
        .seal(std::sync::Arc::new(pending_segment("parked-refusal", 0)))?;
    await_seal_attempts(&runtime.store_actor, &mut failed, 1)?;

    let answer = answer_within(
        "a snapshot requested while a seal is parked",
        runtime.store_actor.snapshot(&runtime.state),
    )
    .await?;
    let refusal = answer
        .err()
        .context("the disk refuses every write, yet the snapshot was reported saved")?
        .to_string();
    assert!(
        refusal.starts_with("No se pudo guardar el estado local"),
        "the refusal must say what could not be saved: {refusal}"
    );
    assert!(
        refusal.contains(SYNTHETIC_DISK_FAILURE),
        "the refusal must carry the store's own cause: {refusal}"
    );
    let (count, _since_ms) = parked_numbers(&refusal).with_context(|| {
        format!("the refusal must count the parked seals and say since when: {refusal}")
    })?;
    assert_eq!(
        count, 1,
        "one seal is parked, and the refusal must say so: {refusal}"
    );
    Ok(())
}

/// With a disk that never comes back and a second of audio that
/// can never be sealed, the daemon still serves the privacy stop and the
/// close. Before the fix the privacy stop's own snapshot waited behind the
/// parked seal for as long as the disk stayed broken, so the helper was never
/// told to stop and the closure's budget was never even checked.
#[tokio::test]
async fn a_disk_that_never_returns_still_lets_the_daemon_stop_and_close() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    let stream = start_recording(&mut runtime, &mut receiver, "parked-daemon").await?;
    let fault = rusqlite::Connection::open(runtime.paths.store.join("orbit.sqlite3"))?;
    fail_every_write(&fault)?;
    // One hundred frames of 10 ms: the hundredth completes one second of PCM,
    // which reaches the store actor and can never be sealed.
    for sequence in 0..100u64 {
        answer_within(
            "an audio frame",
            runtime.native_event(audio_frame(&stream, sequence, 80)),
        )
        .await??;
    }
    assert_eq!(
        runtime.capture.pending_seals.len(),
        1,
        "one second of audio is waiting for a seal the disk refuses"
    );

    answer_within("the privacy stop", runtime.command(Command::PrivacyStop)).await??;
    assert!(
        runtime.state.privacy_stopped(),
        "the privacy stop is latched even though the disk refuses it"
    );
    let mut told_to_stop = false;
    while let Ok(command) = receiver.try_recv() {
        told_to_stop |= command["command"] == "privacy_stop";
    }
    assert!(
        told_to_stop,
        "the helper must be told to stop while the disk refuses every write"
    );

    let mut closing = shutdown::Shutdown::default();
    let closed = answer_within(
        "the shutdown order",
        closing.command(Command::Shutdown, &mut runtime),
    )
    .await?;
    assert!(
        closed.is_err() && closing.requested,
        "the close is answered, latched, and refuses to discard the audio: {closed:?}"
    );
    // What the daemon does when the closure's budget runs out: declare what
    // stayed in memory and write the last snapshot before leaving.
    runtime.declare_bounded_exit().await?;
    let last = answer_within(
        "the closing snapshot",
        runtime.store_actor.snapshot(&runtime.state),
    )
    .await?;
    let refusal = last
        .err()
        .context("the disk refuses every write, yet the last snapshot was saved")?
        .to_string();
    assert!(
        parked_numbers(&refusal).is_some_and(|(count, _)| count >= 1),
        "the last refusal must count the audio still in memory: {refusal}"
    );
    Ok(())
}

/// `snapshot_due` was rewritten to stop the busy loop; what it must keep is
/// that audio admitted in the same turn is written before the coalesced
/// snapshot. The order is read
/// from the database itself, through triggers, not from the actor's counters.
#[tokio::test]
async fn audio_admitted_with_a_snapshot_is_written_before_it() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _receiver) = fixture(directory.path())?;
    let probe = rusqlite::Connection::open(runtime.paths.store.join("orbit.sqlite3"))?;
    probe.execute_batch(
        "CREATE TABLE write_order(n INTEGER PRIMARY KEY AUTOINCREMENT, what TEXT NOT NULL);
         CREATE TRIGGER order_chunk AFTER INSERT ON chunks
         BEGIN INSERT INTO write_order(what) VALUES ('chunk'); END;
         CREATE TRIGGER order_snapshot_insert AFTER INSERT ON settings WHEN NEW.key = 'snapshot'
         BEGIN INSERT INTO write_order(what) VALUES ('snapshot'); END;
         CREATE TRIGGER order_snapshot_update AFTER UPDATE ON settings WHEN NEW.key = 'snapshot'
         BEGIN INSERT INTO write_order(what) VALUES ('snapshot'); END;",
    )?;
    let release = hold_the_actor(&runtime)?;
    runtime
        .store_actor
        .seal(std::sync::Arc::new(pending_segment("audio-first", 0)))?;
    runtime
        .store_actor
        .seal(std::sync::Arc::new(pending_segment("audio-second", 1)))?;
    runtime.state.set_notice(NoticeCode::SessionOpened);
    let state = runtime.state.clone();
    let mut saving = std::pin::pin!(runtime.store_actor.snapshot(&state));
    hand_over(&mut saving);
    release.send(())?;
    saving.await?;
    let mut statement = probe.prepare("SELECT what FROM write_order ORDER BY n")?;
    let order: Vec<String> = statement
        .query_map([], |row| row.get(0))?
        .collect::<std::result::Result<_, _>>()?;
    assert_eq!(
        order,
        vec!["chunk", "chunk", "snapshot"],
        "audio admitted in the same turn must be written before the coalesced snapshot"
    );
    Ok(())
}
