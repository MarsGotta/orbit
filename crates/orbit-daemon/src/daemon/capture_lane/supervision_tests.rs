//! A panic of the capture task while a meeting records: the task is
//! relaunched and armed again, the frame it had in hand is declared a gap,
//! and the meeting goes on. Past the budget the task stays down, the loop
//! serves every frame and the daemon is told to end. A panicking chunk
//! transfer is counted and the chunks still go up. The loop here is the
//! test, serving its channel as `MainLoop` does; frames go through the
//! production route. Synthetic audio, store and hub.
use super::task::CaptureTask;
use super::test_support::{self, TASK_JAM};
use crate::daemon::audio::tests::fixture;
use crate::daemon::meeting_continuity_tests::{frame, start};
use crate::daemon::*;
use crate::native::{NativeRoute, SEQ_FIELD};
use crate::supervision::{Actor, Exhausted, RESTART_BUDGET, Supervision};
use std::time::Instant;

const RECORDING: &str = "reunion-sintetica";
/// A tenth of a second at 8 kHz: a chunk seals every ten frames.
const FRAMES: usize = 800;

struct Meeting {
    runtime: Runtime,
    stream: String,
    route: NativeRoute,
    events: mpsc::Receiver<Value>,
    task: CaptureTask,
    next: u64,
    _commands: mpsc::Receiver<Value>,
}

async fn meeting(directory: &std::path::Path) -> Result<Meeting> {
    let (mut runtime, mut commands) = fixture(directory)?;
    runtime.supervision = Supervision::new(Some(crate::diagnostics::DiagnosticLog::in_root(
        &runtime.paths.root,
    )));
    let stream = start(&mut runtime, &mut commands).await?;
    let (task, audio) = test_support::spawn_task(&runtime)?;
    let (events_tx, events) = mpsc::channel(crate::native::NATIVE_EVENTS_CAPACITY);
    runtime.settle_capture_lane().await;
    anyhow::ensure!(
        runtime.capture.lane.lock().gate.armed,
        "the lane was not armed"
    );
    Ok(Meeting {
        runtime,
        stream,
        route: NativeRoute::new(events_tx, audio),
        events,
        task,
        next: 0,
        _commands: commands,
    })
}

impl Meeting {
    /// One turn of the loop, as `MainLoop` serves its channel and the items
    /// the task handed over, and the end of that turn.
    async fn turn(&mut self) {
        while let Ok(mut event) = self.events.try_recv() {
            let seq = event
                .as_object_mut()
                .and_then(|object| object.remove(SEQ_FIELD))
                .and_then(|seq| seq.as_u64());
            self.runtime.capture_barrier(seq).await;
            self.runtime.handle_native_event(event).await;
            if let Some(seq) = seq {
                self.runtime.capture.served_control = seq;
            }
        }
        self.runtime.serve_forwarded(None).await;
        self.runtime.settle_capture_lane().await;
    }

    /// The helper's next `count` frames, through the bridge's route, with a
    /// loop turn after each when `turns`.
    async fn feed(&mut self, count: u64, turns: bool) -> Result<()> {
        for _ in 0..count {
            let audio = frame(&self.stream, self.next, FRAMES);
            self.next += 1;
            self.route
                .send(audio)
                .await
                .map_err(|_| anyhow::anyhow!("the loop's channel closed"))?;
            tokio::time::sleep(Duration::from_millis(5)).await;
            if turns {
                self.turn().await;
            }
        }
        Ok(())
    }

    fn sealed(&self) -> Result<usize> {
        Ok(self
            .runtime
            .fixture_store
            .list_chunks(RECORDING, None, 500)?
            .len())
    }

    /// Loop turns until `at_least` chunks are on disk.
    async fn sealed_at_least(&mut self, at_least: usize) -> Result<usize> {
        let started = Instant::now();
        loop {
            self.turn().await;
            let sealed = self.sealed()?;
            if sealed >= at_least {
                return Ok(sealed);
            }
            anyhow::ensure!(
                started.elapsed() < TASK_JAM,
                "{sealed} chunks sealed, never {at_least}; phase {:?}",
                self.phase()
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    fn phase(&self) -> Option<orbit_core::CapturePhase> {
        crate::daemon::meeting_continuity_tests::phase(&self.runtime)
    }

    fn gaps(&self) -> Option<u64> {
        self.runtime
            .state
            .durable
            .capture
            .recording
            .as_ref()
            .map(|recording| recording.gaps)
    }

    fn log(&self) -> String {
        std::fs::read_to_string(self.runtime.paths.root.join("logs/daemon.log")).unwrap_or_default()
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_capture_task_that_panics_is_relaunched_and_the_meeting_goes_on() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let mut meeting = meeting(directory.path()).await?;
    meeting.feed(20, true).await?;
    let before = meeting.sealed_at_least(2).await?;

    meeting.runtime.capture.lane.inject_panics(1, 0);
    meeting.feed(30, true).await?;
    let after = meeting.sealed_at_least(before + 2).await?;
    assert_eq!(
        meeting.phase(),
        Some(orbit_core::CapturePhase::Recording),
        "the meeting did not go on"
    );
    assert_eq!(meeting.gaps(), Some(1), "the frame in hand is one gap");
    assert_eq!(meeting.runtime.supervision.restarts(Actor::CaptureTask), 1);
    assert!(meeting.runtime.capture.lane.lock().task_running);

    // Relaunched and armed again: the task alone seals, with no loop turn.
    assert!(meeting.runtime.capture.lane.lock().gate.armed);
    meeting.feed(20, false).await?;
    let started = Instant::now();
    while meeting.sealed()? < after + 2 {
        anyhow::ensure!(
            started.elapsed() < TASK_JAM,
            "the relaunched task seals nothing"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(meeting.runtime.capture.lane.lock().forwarded.is_empty());
    let log = meeting.log();
    assert!(log.contains("\"actor_panicked\""), "{log}");
    assert!(log.contains("\"capture_task\""), "{log}");
    assert!(
        !log.contains("injected"),
        "the panic's message reached the log: {log}"
    );
    meeting.task.stop();
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_capture_task_past_its_budget_stays_down_and_the_loop_keeps_the_meeting() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let mut meeting = meeting(directory.path()).await?;
    meeting.feed(20, true).await?;
    let before = meeting.sealed_at_least(2).await?;

    let panics = u32::try_from(RESTART_BUDGET + 1)?;
    meeting.runtime.capture.lane.inject_panics(panics, 0);
    meeting.feed(40, true).await?;
    meeting.sealed_at_least(before + 2).await?;
    assert_eq!(
        meeting.runtime.supervision.exhausted(),
        Some(Exhausted {
            actor: Actor::CaptureTask,
            panics: RESTART_BUDGET + 1,
        })
    );
    assert!(!meeting.runtime.capture.lane.lock().task_running);
    assert_eq!(meeting.phase(), Some(orbit_core::CapturePhase::Recording));
    assert_eq!(meeting.gaps(), Some(u64::from(panics)));
    meeting.task.stop();
    Ok(())
}

/// Acknowledges every chunk PUT and records its id.
fn synthetic_hub(
    listener: tokio::net::TcpListener,
) -> (
    std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    tokio::task::JoinHandle<()>,
) {
    let received = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let record = std::sync::Arc::clone(&received);
    let task = tokio::spawn(async move {
        while let Ok((mut stream, request)) = test_support::accept_put(&listener).await {
            let text = String::from_utf8_lossy(&request).into_owned();
            let id = text
                .split_whitespace()
                .nth(1)
                .and_then(|path| path.strip_prefix("/v1/chunks/"))
                .unwrap_or_default()
                .to_owned();
            let sha = text
                .lines()
                .find_map(|line| {
                    line.to_ascii_lowercase()
                        .strip_prefix("x-orbit-sha256: ")
                        .map(str::to_owned)
                })
                .unwrap_or_default();
            if let Ok(mut received) = record.lock() {
                received.push(id.clone());
            }
            let _ = test_support::acknowledge(&mut stream, &id, &sha).await;
        }
    });
    (received, task)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_chunk_transfer_that_panics_is_counted_and_the_chunks_still_go_up() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let (mut runtime, mut commands) = fixture(directory.path())?;
    runtime.settings.hub_url = Some(format!("http://{}", listener.local_addr()?));
    runtime.token = Some("synthetic-not-a-credential".into());
    let (received, hub) = synthetic_hub(listener);
    let stream = start(&mut runtime, &mut commands).await?;
    runtime.capture.lane.inject_panics(0, 1);
    let (task, audio) = test_support::spawn_task(&runtime)?;
    let (events_tx, events) = mpsc::channel(crate::native::NATIVE_EVENTS_CAPACITY);
    runtime.settle_capture_lane().await;
    let mut meeting = Meeting {
        runtime,
        stream,
        route: NativeRoute::new(events_tx, audio),
        events,
        task,
        next: 0,
        _commands: commands,
    };
    meeting.feed(30, true).await?;
    let sealed = meeting.sealed_at_least(3).await?;
    let started = Instant::now();
    loop {
        let sent = received.lock().map(|ids| ids.len()).unwrap_or(0);
        if sent >= sealed {
            break;
        }
        anyhow::ensure!(
            started.elapsed() < TASK_JAM,
            "{sent} of {sealed} chunks reached the hub"
        );
        meeting.turn().await;
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(meeting.runtime.supervision.restarts(Actor::ChunkUpload), 1);
    meeting.task.stop();
    hub.abort();
    Ok(())
}
