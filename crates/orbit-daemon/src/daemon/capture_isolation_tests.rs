//! A stall anywhere else in the daemon never delays capture: with the loop
//! held for fifteen seconds by an order, the capture task keeps sequencing,
//! sealing and uploading a two-track meeting at the pace it is recorded.
//! The loop here is the test itself: it starts the meeting, ends its turn as
//! the loop does, and then holds a turn of its own while the helper's frames
//! keep coming through the bridge's channel. Synthetic audio, store and hub.
use super::*;
use crate::daemon::audio::tests::fixture;
use crate::daemon::capture_lane::test_support;
use base64::{Engine, engine::general_purpose::STANDARD};
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// The hold: the test the brief asks for, fifteen seconds of a loop busy
/// with something that is not the capture.
const HOLD: Duration = Duration::from_secs(15);
/// One frame per track every 100 ms, as the helper sends them (~10 per
/// second per track at this size): a chunk seals every ten frames.
const FRAME: Duration = Duration::from_millis(100);
const FRAMES_PER_FRAME: usize = 800;

/// What the synthetic hub received, by when.
type Received = Arc<Mutex<Vec<(Instant, String)>>>;

/// Acknowledges every chunk PUT with a durable receipt, and records it.
fn synthetic_hub(listener: tokio::net::TcpListener) -> (Received, tokio::task::JoinHandle<()>) {
    let received: Received = Arc::default();
    let record = Arc::clone(&received);
    let task = tokio::spawn(async move {
        while let Ok((mut stream, request)) = test_support::accept_put(&listener).await {
            let text = String::from_utf8_lossy(&request).into_owned();
            let header = |name: &str| {
                text.lines()
                    .find_map(|line| {
                        line.to_ascii_lowercase()
                            .strip_prefix(&format!("{name}: "))
                            .map(str::to_owned)
                    })
                    .unwrap_or_default()
            };
            let id = text
                .split_whitespace()
                .nth(1)
                .and_then(|path| path.strip_prefix("/v1/chunks/"))
                .unwrap_or_default()
                .to_owned();
            let sha = header("x-orbit-sha256");
            if let Ok(mut received) = record.lock() {
                received.push((Instant::now(), id.clone()));
            }
            let _ = test_support::acknowledge(&mut stream, &id, &sha).await;
        }
    });
    (received, task)
}

fn audio(source: &str, stream: &str, sequence: u64) -> Value {
    json!({"event":"audio", "source":source, "stream_id":stream,
        "sample_rate":8000, "channels":1, "frames":FRAMES_PER_FRAME, "sequence":sequence,
        "monotonic_ns":sequence.saturating_mul(100_000_000),
        "pcm_s16le_base64":STANDARD.encode(vec![1; FRAMES_PER_FRAME * 2])})
}

/// A two-track meeting started as the loop starts one: its streams are the
/// ones the helper's frames carry.
async fn two_track_meeting(
    runtime: &mut Runtime,
    receiver: &mut mpsc::Receiver<Value>,
) -> Result<(String, String)> {
    runtime
        .apply(Event::StartMeeting {
            id: "reunion-retenida".into(),
            title: "Reunión sintética".into(),
            audio: orbit_core::protocol::MeetingAudio::System,
        })
        .await?;
    let mut streams = (String::new(), String::new());
    for _ in 0..2 {
        let command = receiver.recv().await.context("start command absent")?;
        runtime
            .native_event(json!({"event":"response","id":command["id"],"ok":true,
                "result":{"stream_id":command["stream_id"]}}))
            .await?;
        let stream = command["stream_id"].as_str().context("stream")?.to_owned();
        if command["source"] == "remote_app" {
            streams.1 = stream;
        } else {
            streams.0 = stream;
        }
    }
    Ok(streams)
}

/// Sends both tracks' frames in real time for `duration`, through the
/// bridge's channel, numbered as the bridge numbers them.
fn helper(
    bridge: mpsc::Sender<crate::native::RoutedAudio>,
    streams: (String, String),
    duration: Duration,
) -> tokio::task::JoinHandle<Result<u64>> {
    tokio::spawn(async move {
        let started = Instant::now();
        let (mut sequence, mut seq) = (0_u64, 0_u64);
        while started.elapsed() < duration {
            for (source, stream) in [("mic_local", &streams.0), ("remote_app", &streams.1)] {
                seq += 1;
                bridge
                    .send(test_support::frame(seq, audio(source, stream, sequence)))
                    .await
                    .map_err(|_| anyhow::anyhow!("the task closed its channel"))?;
            }
            sequence += 1;
            tokio::time::sleep(FRAME).await;
        }
        Ok(sequence)
    })
}

fn sealed_chunks(runtime: &Runtime) -> Result<usize> {
    Ok(runtime
        .fixture_store
        .list_chunks("reunion-retenida", None, 500)?
        .len())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_loop_held_fifteen_seconds_keeps_sealing_and_uploading_on_schedule() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    runtime.settings.hub_url = Some(format!("http://{}", listener.local_addr()?));
    runtime.link.token = Some("synthetic-not-a-credential".into());
    let (received, hub) = synthetic_hub(listener);
    let streams = two_track_meeting(&mut runtime, &mut receiver).await?;
    let (mut task, bridge) = test_support::spawn_task(&runtime)?;
    // The loop's end of turn: the recording is steady, the lane is armed.
    runtime.settle_capture_lane().await;
    assert!(runtime.capture.lane.lock().gate.armed);

    // An order holds the loop: nothing of the runtime runs for HOLD.
    runtime
        .turn_watch
        .begin(turn_watch::TurnSource::command(&Command::Status));
    let held = Instant::now();
    let feeding = helper(bridge, streams, HOLD);
    let mut seals = Vec::new();
    while held.elapsed() < HOLD {
        tokio::time::sleep(Duration::from_millis(250)).await;
        seals.push((held.elapsed(), sealed_chunks(&runtime)?));
    }
    let uploads: Vec<_> = received
        .lock()
        .map_err(|_| anyhow::anyhow!("hub record poisoned"))?
        .iter()
        .map(|(at, id)| (at.duration_since(held), id.clone()))
        .collect();
    let slow = runtime
        .turn_watch
        .end()
        .context("the hold is a slow turn")?;
    let frames = feeding.await??;
    // Fifteen seconds of two tracks: a chunk per track per second.
    let expected = (frames / 10) as usize * 2;
    let sealed = seals.last().map_or(0, |(_, sealed)| *sealed);
    eprintln!(
        "capture_isolation held_ms={} frames_per_track={frames} sealed={sealed} uploaded={} \
         first_seal_ms={:?} last_upload_ms={:?}",
        slow.took.as_millis(),
        uploads.len(),
        seals
            .iter()
            .find(|(_, sealed)| *sealed > 0)
            .map(|(at, _)| at.as_millis()),
        uploads.last().map(|(at, _)| at.as_millis()),
    );
    assert!(slow.took >= HOLD, "the loop was not held: {slow:?}");
    assert!(
        sealed + 2 >= expected,
        "sealed {sealed} of {expected} while held: {seals:?}"
    );
    assert!(
        uploads.len() + 4 >= expected,
        "uploaded {} of {expected}: {uploads:?}",
        uploads.len()
    );
    // On schedule: chunks keep being sealed and sent all through the hold,
    // never in a burst after it.
    for second in 3..14 {
        let by = Duration::from_secs(second);
        let sealed_by = seals
            .iter()
            .filter(|(at, _)| *at <= by)
            .map(|(_, n)| *n)
            .max();
        let sent_by = uploads.iter().filter(|(at, _)| *at <= by).count();
        assert!(
            sealed_by.unwrap_or(0) + 4 >= second as usize * 2,
            "{second} s: {seals:?}"
        );
        assert!(
            sent_by + 6 >= second as usize * 2,
            "{second} s: {uploads:?}"
        );
    }
    assert!(runtime.capture.lane.lock().forwarded.is_empty());

    // The loop comes back and counts what was sealed meanwhile.
    runtime.settle_capture_lane().await;
    let recording = runtime
        .state
        .durable
        .capture
        .recording
        .as_ref()
        .context("recording")?;
    assert_eq!(recording.sealed_chunks as usize, sealed_chunks(&runtime)?);
    task.stop();
    hub.abort();
    let log =
        std::fs::read_to_string(runtime.paths.root.join("logs/daemon.log")).unwrap_or_default();
    assert!(!log.contains("capture_turn_slow"), "{log}");
    Ok(())
}

/// The contrast: a lane the loop has not armed (a voice note, a drain, a
/// failed seal) hands every frame to the loop, in order. A held loop then
/// seals nothing until it comes back, as every frame waited before this
/// change, and the helper's backlog is kept, not lost.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_disarmed_lane_hands_frames_to_the_loop_and_loses_none() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    let streams = two_track_meeting(&mut runtime, &mut receiver).await?;
    let (mut task, bridge) = test_support::spawn_task(&runtime)?;
    runtime.capture.lane.disarm();
    let held = Duration::from_secs(3);
    let frames = helper(bridge, streams, held).await??;
    assert_eq!(
        sealed_chunks(&runtime)?,
        0,
        "the loop was held: nothing sealed"
    );
    let forwarded = runtime.capture.lane.lock().forwarded.len();
    assert_eq!(
        forwarded as u64,
        frames * 2,
        "every frame waits for the loop"
    );
    runtime.serve_forwarded(None).await;
    crate::daemon::audio::tests::settle_seal_receipts(&mut runtime).await?;
    assert_eq!(sealed_chunks(&runtime)?, (frames / 10) as usize * 2);
    task.stop();
    Ok(())
}
