//! A calendar read that takes 30 s holds neither the capture nor the app's
//! orders.
//!
//! On 2026-10-22 a meeting was cut in the same second as a calendar
//! invalidation. This runs the real daemon, its real socket and the production
//! helper launch (`--helper`) against a synthetic NDJSON peer that plays the
//! helper's one EventKit slot: it holds the first calendar read it is asked
//! for [`HOLD`] and refuses every other one with `calendar_busy`, as the helper
//! does while its EventKit call has not returned. Meanwhile a meeting records
//! two tracks at their real pace, and the app asks the daemon for its state.
//!
//! What it reads is the daemon's own turn instrumentation (`daemon_turn_slow`,
//! `daemon_loop_stalled`), each order's answer time, and the sealed chunks.
//! PCM, timestamps and permissions are fixture data, not macOS evidence:
//! EventKit's own slowness on the Mac is the helper's (`CalendarController`).
use anyhow::{Context, Result, bail, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use orbit_core::{
    calendar::CalendarWindow,
    calendar_control::CalendarCommand,
    protocol::{Command as IpcCommand, MeetingAudio},
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    path::Path,
    process::Stdio,
    time::{Duration, Instant},
};
use tokio::{
    fs::File,
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    sync::{mpsc, oneshot},
};

#[path = "support/helper_relay.rs"]
mod helper_relay;
use helper_relay::{Relay, STRICT_TURN_BOUND, kill_group, tokio_daemon};
#[path = "support/ipc.rs"]
mod ipc;

/// How long the helper's EventKit call takes: the read the brief names.
const HOLD: Duration = Duration::from_secs(30);

/// One audio event per track: a quarter of a second, sent every quarter of a
/// second, the pace of a live capture.
const CHUNK: Duration = Duration::from_millis(250);
const RATE: u64 = 8_000;
const CHUNK_FRAMES: u64 = RATE / 4;

/// How long before the held read is answered the audio stops, so that its
/// sealing can be seen to finish while the read is still held.
const SETTLE: Duration = Duration::from_secs(6);

/// A state the daemon reaches as part of starting or of an order: the suite
/// gives a cold daemon this long under load elsewhere (`track_signal_lifecycle.rs`).
const STATE_BUDGET: Duration = Duration::from_secs(30);

/// The fixture's helper identity, from `contracts/fixtures/native/results/hello.json`.
const INSTANCE: &str = "00000000-0000-4000-8000-000000000911";
const REMOTE_PID: i32 = 321;

#[derive(Default, Debug)]
struct Peer {
    held_for: Option<Duration>,
    busy_refusals: usize,
    chunks_per_track: u64,
    tails: usize,
}

enum Control {
    Stream(bool, oneshot::Sender<u64>),
}

async fn send(writer: &mut File, event: Value) -> Result<()> {
    let mut bytes = serde_json::to_vec(&event)?;
    ensure!(bytes.len() < 32_768, "synthetic event budget exceeded");
    bytes.push(b'\n');
    writer.write_all(&bytes).await?;
    writer.flush().await?;
    Ok(())
}

fn audio(source: &str, stream: &str, sequence: u64, frames: u64) -> Value {
    let sample: i16 = if source == "mic_local" { 0 } else { 1_024 };
    let level = if source == "mic_local" { -120.0 } else { -30.1 };
    json!({"v":1,"event":"audio","source":source,"stream_id":stream,
        "sequence":sequence,"frames":frames,"sample_rate":RATE,"channels":1,
        "clock_domain":"mach_absolute_time",
        "monotonic_ns":1_000_000_000 + sequence * 250_000_000,
        "pcm_s16le_base64":STANDARD.encode(sample.to_le_bytes().repeat(frames as usize)),
        "levels":{"rms_dbfs":level,"peak_dbfs":level,"clipped_frames":0},
        "processing":"unprocessed"})
}

fn state(pending: bool) -> Result<Value> {
    let mut result = helper_relay::state_result(json!({
        "microphone":"authorized","screen":true,"accessibility":false,
        "calendars":"full_access"}))?;
    result["calendar"]["physical_read_pending"] = json!(pending);
    Ok(result)
}

/// The answer a calendar read gets when EventKit finally returns: well formed
/// and empty. The daemon gave up on it long before, so it is an obsolete reply.
fn calendar_answer(request: &Value) -> Value {
    if request["command"] == "calendar_sources" {
        return json!({"inventory_id":"11111111-1111-4111-8111-111111111111",
            "native_instance_id":INSTANCE,"privacy_generation":1,
            "clock_domain":"mach_absolute_time","observed_monotonic_ns":100,
            "completed_monotonic_ns":200,"calendars":[],"after_index":0,"total":0});
    }
    let query = &request["calendar_query"];
    json!({"read_id":"44444444-4444-4444-8444-444444444444","native_instance_id":INSTANCE,
        "privacy_generation":1,
        "window":{"start_unix_ms":query["start_unix_ms"],"end_unix_ms":query["end_unix_ms"]},
        "clock_domain":"mach_absolute_time","capture_started_monotonic_ns":100,
        "capture_completed_monotonic_ns":200,"observed_at_unix_ms":1_800_000_000_000_i64,
        "system_timezone":"UTC","provider_sync_status":"unknown",
        "enumeration":"bounded_local_read","after_index":0,"total":0,"events":[]})
}

/// The helper's side of the pipe. Its one EventKit slot is held by the first
/// calendar read for [`HOLD`]; `held` hears when that read arrived.
async fn native_peer(
    mut writer: File,
    reader: File,
    mut control: mpsc::Receiver<Control>,
    held: oneshot::Sender<Instant>,
) -> Result<Peer> {
    send(&mut writer, helper_relay::ready_event()?).await?;
    let mut lines = BufReader::new(reader).lines();
    let mut peer = Peer::default();
    let mut held = Some(held);
    let mut hold: Option<(Value, Instant)> = None;
    let mut streams = BTreeMap::<String, String>::new();
    let mut streaming = false;
    let mut ticks = tokio::time::interval(CHUNK);
    loop {
        let answer_at = hold.as_ref().map(|(_, since)| *since + HOLD);
        tokio::select! {
            Some(Control::Stream(on, done)) = control.recv() => {
                streaming = on;
                done.send(peer.chunks_per_track).map_err(|_| anyhow::anyhow!("test gone"))?;
            }
            _ = ticks.tick(), if streaming => {
                for (source, stream) in &streams {
                    send(&mut writer, audio(source, stream, peer.chunks_per_track, CHUNK_FRAMES)).await?;
                }
                peer.chunks_per_track += 1;
            }
            () = tokio::time::sleep_until(answer_at.unwrap_or_else(Instant::now).into()),
                if answer_at.is_some() => {
                let Some((request, since)) = hold.take() else { continue };
                peer.held_for = Some(since.elapsed());
                send(&mut writer, json!({"v":1,"event":"response","id":request["id"],
                    "ok":true,"result":calendar_answer(&request)})).await?;
            }
            line = lines.next_line() => {
                let Some(line) = line? else { break };
                let request: Value = serde_json::from_str(&line)?;
                let result = match request["command"].as_str() {
                    Some("hello" | "heartbeat" | "status") => state(hold.is_some())?,
                    Some("calendar_sources" | "calendar_snapshot") => {
                        if hold.is_some() {
                            peer.busy_refusals += 1;
                            send(&mut writer, json!({"v":1,"event":"response","id":request["id"],
                                "ok":false,"error":{"code":"calendar_busy",
                                "message":"La operación anterior de EventKit aún no ha terminado."}})).await?;
                            continue;
                        }
                        if peer.held_for.is_some() {
                            calendar_answer(&request)
                        } else {
                            let since = Instant::now();
                            if let Some(held) = held.take() {
                                let _ = held.send(since);
                            }
                            hold = Some((request, since));
                            continue;
                        }
                    }
                    Some("calendar_disconnect") => {
                        json!({"authorization":"full_access","physical_read_pending":hold.is_some()})
                    }
                    Some("audio_start") => {
                        let source = request["source"].as_str().context("start source")?;
                        let stream = request["stream_id"].as_str().context("start stream")?;
                        ensure!(!streams.contains_key(source), "duplicate stream");
                        streams.insert(source.to_owned(), stream.to_owned());
                        json!({"stream_id":stream,"source":source,"sample_rate":RATE,"channels":1})
                    }
                    Some("audio_wake_phrase") => {
                        json!({"stream_id":request["stream_id"],"enabled":request["enabled"]})
                    }
                    Some("audio_stop") => {
                        let stream = request["stream_id"].as_str().context("stop stream")?;
                        let source = streams.iter().find(|(_, id)| id.as_str() == stream)
                            .map(|(source, _)| source.clone()).context("stop not bound")?;
                        // The tail precedes the drain answer on the same pipe.
                        send(&mut writer, audio(&source, stream, peer.chunks_per_track, CHUNK_FRAMES)).await?;
                        streams.remove(&source);
                        peer.tails += 1;
                        json!({"stream_id":stream,"drain_complete":true})
                    }
                    // A closing cuts without the latch; nothing is live by then.
                    Some(name @ ("privacy_stop" | "capture_stop")) => {
                        ensure!(streams.is_empty(), "{name} with live streams");
                        let mut result = state(hold.is_some())?;
                        result["privacy_stopped"] = json!(name == "privacy_stop");
                        result["drain_complete"] = json!(true);
                        result
                    }
                    Some("playback_stop") => json!({}),
                    Some("shutdown") => json!({"drain_complete":true}),
                    other => bail!("native action outside this fixture: {other:?}"),
                };
                send(&mut writer, json!({"v":1,"event":"response","id":request["id"],
                    "ok":true,"result":result})).await?;
            }
        }
    }
    Ok(peer)
}

/// One order to the daemon and how long its answer took.
async fn timed(path: &Path, command: IpcCommand) -> Result<Duration> {
    let started = Instant::now();
    ipc::request_async(path, command).await?;
    Ok(started.elapsed())
}

async fn wait_state(
    path: &Path,
    waiting_for: &str,
    budget: Duration,
    predicate: impl Fn(&Value) -> bool,
) -> Result<Value> {
    let started = Instant::now();
    loop {
        let state = ipc::status_async(path).await?;
        if predicate(&state) {
            return Ok(state);
        }
        if started.elapsed() >= budget {
            bail!("the daemon never got to {waiting_for} within {budget:?}: {state}");
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

fn sealed(state: &Value) -> u64 {
    state["recording"]["sealed_chunks"].as_u64().unwrap_or(0)
}

/// The capture seals one chunk per second of audio of each track. Caught up
/// means both tracks received every frame sent and sealed every whole second.
fn caught_up(state: &Value, chunks: u64) -> bool {
    let frames = chunks * CHUNK_FRAMES;
    ["mic_local", "remote_app"]
        .iter()
        .all(|track| state["recording_signal"][track]["received_frames"] == frames)
        && state["recording"]["tracks"]
            .as_array()
            .is_some_and(|tracks| {
                tracks.len() == 2
                    && tracks
                        .iter()
                        .all(|track| track["sealed_chunks"] == frames / RATE)
            })
}

async fn stream(control: &mpsc::Sender<Control>, on: bool) -> Result<u64> {
    let (done, sent) = oneshot::channel();
    control.send(Control::Stream(on, done)).await?;
    sent.await.context("the peer stopped")
}

/// What the daemon logged of its loop while this ran.
fn loop_lines(path: &Path) -> (Vec<Value>, Vec<Value>) {
    let lines = helper_relay::daemon_log(path);
    let pick = |event: &str| {
        lines
            .iter()
            .filter(|line| line["event"] == event)
            .map(|line| line["fields"].clone())
            .collect::<Vec<_>>()
    };
    (pick("daemon_turn_slow"), pick("daemon_loop_stalled"))
}

async fn scenario(path: &Path) -> Result<()> {
    let relay = Relay::create(path, "helper")?;
    let child = tokio_daemon(path, &relay.script, None)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    let group = child.id().context("the daemon has no pid")?;
    let mut cleanup = Cleanup(Some(group));
    let mut child = child;
    let (reader, writer) = tokio::task::spawn_blocking(move || relay.connect()).await??;
    let (control, incoming) = mpsc::channel(1);
    let (held, hold_started) = oneshot::channel();
    let peer = tokio::spawn(native_peer(
        File::from_std(writer),
        File::from_std(reader),
        incoming,
        held,
    ));

    // With the permission known, the daemon lists the calendars by itself:
    // that first read is the one EventKit holds.
    let since = tokio::time::timeout(STATE_BUDGET, hold_started)
        .await
        .context("the daemon never asked for a calendar read")??;
    let answered_at = since + HOLD;
    let mut orders = Vec::new();
    let week = CalendarWindow {
        start_unix_ms: 1_790_812_800_000,
        end_unix_ms: 1_790_812_800_000 + 7 * 86_400_000,
    };
    orders.push(
        timed(
            path,
            IpcCommand::Calendar {
                request: CalendarCommand::Read { window: week },
            },
        )
        .await?,
    );
    orders.push(
        timed(
            path,
            IpcCommand::MeetingStart {
                title: "Reunión sintética con la agenda retenida".into(),
                audio: MeetingAudio::Application {
                    application_pid: REMOTE_PID,
                },
                duration_minutes: None,
                event: None,
            },
        )
        .await?,
    );
    wait_state(path, "start the recording", STATE_BUDGET, |state| {
        state["recording"]["phase"] == "recording"
    })
    .await?;
    stream(&control, true).await?;
    while Instant::now() + SETTLE < answered_at {
        orders.push(timed(path, IpcCommand::Status).await?);
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    let chunks = stream(&control, false).await?;
    let stopped_at = Instant::now();
    let caught_up = wait_state(path, "seal every second of audio sent", SETTLE, |state| {
        caught_up(state, chunks)
    })
    .await?;
    let seal_lag = stopped_at.elapsed();
    // Everything recorded while EventKit held the read was sealed before the
    // read was answered.
    ensure!(
        Instant::now() < answered_at,
        "the chunks were sealed only after the calendar read was answered"
    );
    ensure!(
        chunks >= 60,
        "too little audio to tell: {chunks} chunks per track"
    );
    assert_eq!(caught_up["recording"]["gaps"], 0, "{caught_up}");

    tokio::time::sleep_until((answered_at + Duration::from_millis(500)).into()).await;
    orders.push(timed(path, IpcCommand::MeetingFinish).await?);
    let closed = wait_state(path, "close the meeting", STATE_BUDGET, |state| {
        state["recording"]["phase"] == "idle"
    })
    .await?;
    // The closing seals what was left of each track and its tail.
    let left = ((chunks + 1) * CHUNK_FRAMES).div_ceil(RATE);
    assert_eq!(sealed(&closed), 2 * left, "{closed}");
    assert_eq!(closed["recording"]["gaps"], 0, "{closed}");
    ipc::request_async(path, IpcCommand::Shutdown).await?;
    let exit = child.wait().await?;
    // Reaped: its group id may be handed on, so it is never signalled again.
    cleanup.0 = None;
    ensure!(exit.success(), "the daemon did not close cleanly");
    let peer = peer.await??;

    assert_eq!(peer.tails, 2, "both tracks must drain their tail");
    let held_for = peer.held_for.context("the held read was never answered")?;
    ensure!(held_for >= HOLD, "the read was held only {held_for:?}");
    let slowest = orders.iter().max().copied().unwrap_or_default();
    let (slow, stalled) = loop_lines(path);
    eprintln!(
        "calendar_isolation held_ms={} chunks_per_track={chunks} orders={} \
         slowest_order_ms={} seal_lag_ms={} busy_refusals={} slow_turns={} stalls={}",
        held_for.as_millis(),
        orders.len(),
        slowest.as_millis(),
        seal_lag.as_millis(),
        peer.busy_refusals,
        serde_json::to_string(&slow)?,
        serde_json::to_string(&stalled)?,
    );
    ensure!(
        slowest < STRICT_TURN_BOUND,
        "an order waited {slowest:?} while the calendar read was held"
    );
    helper_relay::assert_no_held_turn(path)?;
    let calendar_named = |fields: &&Value| fields.to_string().contains("calendar");
    ensure!(
        !slow
            .iter()
            .chain(&stalled)
            .any(|fields| calendar_named(&fields)),
        "a loop turn was held by the calendar: {slow:?} {stalled:?}"
    );
    Ok(())
}

/// Failure cleanup only: the group is this test's daemon and its helper,
/// signalled only while the daemon is unreaped and its id still reserved.
struct Cleanup(Option<u32>);

impl Drop for Cleanup {
    fn drop(&mut self) {
        if let Some(group) = self.0 {
            kill_group(group);
        }
    }
}

#[tokio::test]
async fn a_calendar_read_held_thirty_seconds_delays_neither_capture_nor_orders() -> Result<()> {
    let directory = tempfile::Builder::new()
        .prefix("orbit-calendar-")
        .tempdir_in("/tmp")?;
    tokio::time::timeout(HOLD + Duration::from_secs(90), scenario(directory.path()))
        .await
        .context("the scenario outlived its bound without one of its waits expiring first")??;
    Ok(())
}
