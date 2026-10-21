//! Real daemon/Unix IPC with an explicitly synthetic NDJSON native peer
//! behind the production helper launch (`--helper`).
//! PCM timestamps and permissions are fixture data, not macOS or realtime evidence.
use anyhow::{Context, Result, bail, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use orbit_core::protocol::{Command as IpcCommand, MeetingAudio};
use orbit_store::{ChunkReceipt, Store};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    path::Path,
    process::Stdio,
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};
use tokio::{
    fs::File,
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    process::{Child, Command},
    sync::{mpsc, oneshot},
    task::JoinHandle,
};

#[path = "support/helper_relay.rs"]
mod helper_relay;
use helper_relay::{Relay, kill_group, tokio_daemon};
#[path = "support/ipc.rs"]
mod ipc;

const RATE: u64 = 8_000;
const ORIGINAL_SECONDS: u64 = 10;
const TAIL_FRAMES: u64 = 100;
const MAX_OUTPUT: u64 = 1_048_576;
const REMOTE_PID: i32 = 321;

// The budgets in this file, and where their numbers come from.
//
// `real_daemon_observes_synthetic_tracks_without_discarding_originals` failed
// on the Mac at 60.01 s against a flat `Duration::from_secs(60)` wrapped round
// the whole lifecycle. Nothing about that number came from the test: it is a
// round one, and it was the only bound in the file, so when it expired the
// build log said the lifecycle `exceeded its 60-second deadline` and not which
// of its ten waits had still been waiting. The same defect had already been
// found in another test file.
//
// What this test cost was not arithmetic. `wait_state` asked the daemon by
// running a fresh `orbit status` process every 25 ms (since 2026-10-15 it asks
// over the socket, in process), so a run of the lifecycle was a few hundred
// `fork`/`exec`s end to end and it was a machine's process creation cost, not
// its clock speed, that decided how long it took. That is
// why it passes here and expired there: spawning is several times dearer on
// macOS, and on the Mac the whole suite is spawning at once.
//
// So it was measured under a load built to stretch exactly that: 32 busy loops
// and 64 `fork`/`exec` loops on 32 cores, with every step of the test reporting
// its own time, over 25 runs. In milliseconds:
//
//     step                                 n     p50    p95    p99   worst
//     the whole lifecycle                 21    7282   7715   7869    7869
//     delivery_delayed (3 000 of floor)   21    2955   3050   3116    3116
//     the native helper after a restart   21     539    927    954     954
//     ten seconds counted on both tracks  21     467    814    895     895
//     back to idle after finish           21     551    796    812     812
//     one `orbit status`                 740     101    237    517     787
//     `orbit export`                      42     374    610    699     699
//     `orbit shutdown`                    42     158    569    590     590
//     the microphone declared             21     213    285    299     299
//     the recording started               21     243    268    277     277
//     `orbit meeting …`                   42     102    134    213     213
//     the daemon's exit on parent loss    34    1388   1564   1571    1571
//
// The whole lifecycle is 7.9 s at its worst there, against a 60 s deadline:
// 7.6x, when the rule for test budgets asks for ten times p99 and five times
// the worst. It
// was not far off being a coin toss, and the Mac lost the toss.
//
// Two things follow. Each wait now carries its own budget and says what it was
// waiting for, so the next expiry names the step instead of the test. And the
// one wait that cannot be hurried carries its floor explicitly: the daemon only
// calls delivery delayed after `SAMPLE_DELIVERY_TIMEOUT_MS` of real silence, so
// that wait cannot finish sooner than three seconds on any machine, and a
// budget for it that did not start from there would be measuring the wrong
// thing. It is taken from the constant rather than written out, so the two
// cannot drift apart.

/// Real time no machine can shorten: the daemon will not report a track as
/// `delivery_delayed` until this much has passed without samples.
const DELIVERY_DELAY_FLOOR: Duration =
    Duration::from_millis(orbit_core::audio_signal::SAMPLE_DELIVERY_TIMEOUT_MS);

/// One ordinary wait for the daemon to reach a state.
const STATE_BUDGET: Duration = Duration::from_secs(30);

/// The wait for `delivery_delayed`, which is the floor plus an ordinary wait.
const DELAYED_STATE_BUDGET: Duration = DELIVERY_DELAY_FLOOR.saturating_add(STATE_BUDGET);

/// The whole lifecycle, as a last bound on the steps that are not state waits
/// -- a request, a daemon exit -- so a hang in one of those still fails the
/// suite. Every wait inside it expires first and names itself, so this one
/// being reached at all now means the hang was somewhere else.
const LIFECYCLE_BUDGET: Duration = Duration::from_secs(120);

/// A managed daemon has to notice its parent is gone and drain both tracks.
const PARENT_LOSS_EXIT_BUDGET: Duration = Duration::from_secs(30);

#[derive(Default)]
struct Effects {
    starts: usize,
    stops: usize,
    heartbeats: usize,
    heartbeats_after_originals: usize,
}

enum Control {
    SendOriginals(oneshot::Sender<()>),
}

struct NativeDaemon {
    child: Child,
    group: u32,
    peer: Option<JoinHandle<Result<Effects>>>,
    control: mpsc::Sender<Control>,
}

impl NativeDaemon {
    async fn start(path: &Path) -> Result<Self> {
        Self::start_with_parent(path, None).await
    }

    async fn start_with_parent(path: &Path, parent_pid: Option<u32>) -> Result<Self> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        // A fresh relay per daemon: a restarted daemon launches its own helper.
        let relay = Relay::create(
            path,
            &format!("helper-{}", NEXT.fetch_add(1, Ordering::SeqCst)),
        )?;
        let child = tokio_daemon(path, &relay.script, parent_pid)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;
        let group = child.id().context("the daemon has no pid")?;
        // Opening a FIFO blocks until its other end is open: off the runtime.
        let connected = tokio::task::spawn_blocking(move || relay.connect()).await?;
        let (reader, writer) = match connected {
            Ok(ends) => ends,
            Err(error) => {
                kill_group(group);
                return Err(error);
            }
        };
        let (control, incoming) = mpsc::channel(1);
        let peer = tokio::spawn(native_peer(
            File::from_std(writer),
            File::from_std(reader),
            incoming,
        ));
        Ok(Self {
            child,
            group,
            peer: Some(peer),
            control,
        })
    }

    async fn send_originals(&self) -> Result<()> {
        let (done, received) = oneshot::channel();
        self.control.send(Control::SendOriginals(done)).await?;
        received.await.context("synthetic PCM delivery failed")
    }

    async fn finish(&mut self) -> Result<Effects> {
        ensure!(
            self.child.wait().await?.success(),
            "daemon exit was not graceful"
        );
        self.peer.take().context("peer already joined")?.await?
    }
}

impl Drop for NativeDaemon {
    fn drop(&mut self) {
        // Failure/timeout cleanup only. Passing assertions require normal exit.
        // The group is this test's daemon and the helpers it started.
        kill_group(self.group);
        if let Some(peer) = &self.peer {
            peer.abort();
        }
        let _ = self.child.start_kill();
    }
}

async fn status(path: &Path) -> Result<Value> {
    ipc::status_async(path).await
}

async fn start_meeting(path: &Path, title: &str) -> Result<()> {
    ipc::request_async(
        path,
        IpcCommand::MeetingStart {
            title: title.into(),
            audio: MeetingAudio::Application {
                application_pid: REMOTE_PID,
            },
            duration_minutes: 1,
            event: None,
        },
    )
    .await?;
    Ok(())
}

/// Wait for the daemon to reach a state, and say what was being waited for if
/// it never does.
///
/// `waiting_for` completes the sentence "the daemon never got to ...",
/// because a wait that expires without naming its subject sends the next
/// reader to the wrong place -- which is what the single `exceeded its
/// 60-second deadline` over the whole lifecycle did on the Mac.
///
/// The budget is a parameter rather than a constant read here so that the guard
/// at the foot of the file can watch one expire without standing there for the
/// real budget, and so that the one wait with a real-time floor under it can
/// say so at its call site.
async fn wait_state(
    path: &Path,
    waiting_for: &str,
    budget: Duration,
    predicate: impl Fn(&Value) -> bool,
) -> Result<Value> {
    let started = std::time::Instant::now();
    let mut polls = 0_u64;
    let mut last: Option<Value> = None;
    let mut last_error: Option<String> = None;
    loop {
        polls += 1;
        match status(path).await {
            Ok(state) => {
                if predicate(&state) {
                    return Ok(state);
                }
                last = Some(state);
            }
            // Kept rather than swallowed: "the daemon answered something else"
            // and "the daemon did not answer" are different failures and used
            // to arrive as the same silence.
            Err(error) => last_error = Some(error.to_string()),
        }
        if started.elapsed() >= budget {
            let seen = match &last {
                Some(state) => {
                    let mut text = state.to_string();
                    // A status is small, but a budget is not a licence to put
                    // an unbounded blob in a build log.
                    text.truncate(2_000);
                    text
                }
                None => "none: the daemon answered no poll".to_owned(),
            };
            let trouble = match &last_error {
                Some(error) => format!("; the last poll failed with: {error}"),
                None => String::new(),
            };
            bail!(
                "the daemon never got to {waiting_for} within {budget:?} \
                 ({polls} status polls){trouble}; the last state was {seen}"
            );
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

async fn send(writer: &mut File, event: Value) -> Result<()> {
    let mut bytes = serde_json::to_vec(&event)?;
    ensure!(bytes.len() < 32_768, "synthetic event budget exceeded");
    bytes.push(b'\n');
    writer.write_all(&bytes).await?;
    // A file hands its writes to a blocking thread; flushing waits for them,
    // which a child's stdin never needed.
    writer.flush().await?;
    Ok(())
}

fn pcm(source: &str, frames: u64) -> Vec<u8> {
    let sample: i16 = if source == "mic_local" { 0 } else { 1_024 };
    sample.to_le_bytes().repeat(frames as usize)
}

fn audio(source: &str, stream: &str, sequence: u64, frames: u64) -> Value {
    let level = if source == "mic_local" {
        -120.0
    } else {
        20.0 * (1_024.0_f64 / 32_768.0).log10()
    };
    json!({"v":1,"event":"audio","source":source,"stream_id":stream,
        "sequence":sequence,"frames":frames,"sample_rate":RATE,"channels":1,
        "clock_domain":"mach_absolute_time",
        "monotonic_ns":1_000_000_000+sequence*1_000_000_000,
        "pcm_s16le_base64":STANDARD.encode(pcm(source,frames)),
        "levels":{"rms_dbfs":level,"peak_dbfs":level,"clipped_frames":0},
        "processing":"unprocessed"})
}

async fn native_peer(
    mut writer: File,
    reader: File,
    mut control: mpsc::Receiver<Control>,
) -> Result<Effects> {
    send(&mut writer, helper_relay::ready_event()?).await?;
    // Bound total native command bytes, therefore also each parsed JSON line.
    let mut lines = BufReader::new(reader.take(MAX_OUTPUT + 1)).lines();
    let mut read_bytes = 0;
    let mut effects = Effects::default();
    let mut streams = BTreeMap::<String, String>::new();
    let mut sent_originals = false;
    loop {
        tokio::select! {
            Some(Control::SendOriginals(done)) = control.recv() => {
                ensure!(!sent_originals && streams.len() == 2, "unexpected synthetic PCM request");
                // Deliver ten seconds of samples quickly. This explicitly does
                // NOT simulate ten seconds of live capture or acoustic timing.
                for sequence in 0..ORIGINAL_SECONDS {
                    for (source, stream) in &streams {
                        send(&mut writer, audio(source, stream, sequence, RATE)).await?;
                    }
                }
                sent_originals = true;
                done.send(()).map_err(|_| anyhow::anyhow!("PCM consumer dropped"))?;
            }
            line = lines.next_line() => {
                let Some(line) = line? else { break };
                read_bytes += line.len() + 1;
                ensure!(read_bytes as u64 <= MAX_OUTPUT, "native command budget exceeded");
                let request: Value = serde_json::from_str(&line)?;
                ensure!(request["v"] == 1 && request["id"].is_string(), "invalid native request");
                let result = match request["command"].as_str() {
                    Some("hello") => helper_relay::state_result(json!({
                        "microphone":"authorized","screen":true,"accessibility":false,
                        "calendars":"not_determined"}))?,
                    Some("heartbeat") => {
                        effects.heartbeats += 1;
                        // A heartbeat during export/shutdown cannot establish
                        // liveness of the preceding no-PCM interval.
                        if sent_originals && effects.stops == 0 {
                            effects.heartbeats_after_originals += 1;
                        }
                        helper_relay::state_result(json!({
                            "microphone":"authorized","screen":true,"accessibility":false,
                            "calendars":"not_determined"}))?
                    }
                    Some("status") => helper_relay::state_result(json!({
                        "microphone":"authorized","screen":true,"accessibility":false,
                        "calendars":"not_determined"}))?,
                    Some("audio_start") => {
                        let source = request["source"].as_str().context("start source absent")?;
                        ensure!(matches!(source,"mic_local"|"remote_app"), "unrequested source");
                        ensure!(if source == "remote_app" { request["application_pid"] == REMOTE_PID }
                            else { request["application_pid"].is_null() }, "wrong application scope");
                        let stream = request["stream_id"].as_str().context("start stream absent")?;
                        ensure!(!stream.is_empty() && stream.len() <= 128 && !streams.contains_key(source),
                            "invalid or duplicate stream");
                        streams.insert(source.to_owned(),stream.to_owned());
                        effects.starts += 1;
                        json!({"stream_id":stream,"source":source,"sample_rate":RATE,"channels":1})
                    }
                    Some("audio_wake_phrase") => {
                        let stream = request["stream_id"].as_str().context("wake phrase stream absent")?;
                        let enabled = request["enabled"].as_bool().context("wake phrase enabled absent")?;
                        ensure!(!enabled || streams.get("mic_local").is_some_and(|id| id == stream),
                            "wake phrase not bound to microphone");
                        json!({"stream_id":request["stream_id"],"enabled":request["enabled"]})
                    }
                    Some("audio_stop") => {
                        let stream = request["stream_id"].as_str().context("stop stream absent")?;
                        let source = streams.iter().find(|(_,id)| id.as_str() == stream)
                            .map(|(source,_)| source.clone()).context("stop not bound to a stream")?;
                        ensure!(sent_originals, "unexpected stop before original PCM");
                        // The tail precedes the exact drain ACK on the same pipe.
                        send(&mut writer,audio(&source,stream,ORIGINAL_SECONDS,TAIL_FRAMES)).await?;
                        streams.remove(&source);
                        effects.stops += 1;
                        json!({"stream_id":stream,"drain_complete":true})
                    }
                    // The production helper stops every live source, emits its
                    // final PCM before the drain ACK and then closes the
                    // managed stdio pipe. The user's stop latches
                    // (`privacy_stop`); a closing cuts without the latch
                    // (`capture_stop`), which is what parent loss exercises now.
                    Some(name @ ("privacy_stop" | "capture_stop")) => {
                        for (source, stream) in &streams {
                            send(&mut writer,audio(source,stream,ORIGINAL_SECONDS,TAIL_FRAMES)).await?;
                        }
                        effects.stops += streams.len();
                        streams.clear();
                        let mut result = helper_relay::state_result(json!({
                            "microphone":"authorized","screen":true,"accessibility":false,
                            "calendars":"not_determined"}))?;
                        result["privacy_stopped"] = json!(name == "privacy_stop");
                        result["drain_complete"] = json!(true);
                        result
                    }
                    Some("playback_stop") => json!({}),
                    // The daemon's last word to its helper before closing its
                    // input; nothing is live by then.
                    Some("shutdown") => json!({"drain_complete":true}),
                    _ => bail!("native action outside the synthetic fixture contract"),
                };
                send(&mut writer,json!({"v":1,"event":"response","id":request["id"],
                    "ok":true,"result":result})).await?;
            }
        }
    }
    ensure!(streams.is_empty(), "peer ended with active streams");
    Ok(effects)
}

/// The originals as the store keeps them: every chunk of both tracks, read
/// back through the store's reader with this fixture's development key.
fn check_originals(path: &Path, recording: &str, epoch: &str) -> Result<()> {
    let key: [u8; 32] = STANDARD
        .decode(std::fs::read(path.join("development-storage.key"))?)?
        .try_into()
        .map_err(|_| anyhow::anyhow!("invalid fixture key"))?;
    let store = Store::open_reader(path.join("store"), key)?;
    let mut receipts: Vec<ChunkReceipt> = Vec::new();
    loop {
        let after = receipts.last().map(|receipt| receipt.metadata.id.clone());
        let page = store.list_chunks(recording, after.as_deref(), 100)?;
        if page.is_empty() {
            break;
        }
        receipts.extend(page);
    }
    ensure!(
        receipts.len() == 22,
        "both original tracks and final tails must be kept"
    );
    for source in ["mic_local", "remote_app"] {
        let mut track: Vec<_> = receipts
            .iter()
            .filter(|receipt| receipt.metadata.track == source)
            .collect();
        track.sort_by_key(|receipt| receipt.metadata.sequence);
        ensure!(track.len() == 11, "track chunks missing or duplicated");
        for (sequence, receipt) in track.into_iter().enumerate() {
            let metadata = &receipt.metadata;
            let frames = if sequence < 10 { RATE } else { TAIL_FRAMES };
            assert_eq!(metadata.recording_id, recording);
            assert_eq!(metadata.capture_epoch_id.as_deref(), Some(epoch));
            assert_eq!(metadata.sequence, sequence as u64);
            assert_eq!(metadata.first_sample, sequence as u64 * RATE);
            assert_eq!(metadata.frames, frames);
            assert_eq!(metadata.sample_rate, RATE as u32);
            assert_eq!(metadata.channels, 1);
            assert_eq!(
                metadata.monotonic_ns,
                1_000_000_000 + sequence as u64 * 1_000_000_000
            );
            let wav = store.read_chunk(&metadata.id)?;
            assert_eq!(wav.len() as u64, receipt.bytes);
            assert_eq!(wav.len() as u64, 44 + frames * 2);
            assert_eq!(&wav[..4], b"RIFF");
            assert_eq!(&wav[8..12], b"WAVE");
            assert_eq!(&wav[44..], pcm(source, frames));
            assert_eq!(format!("{:x}", Sha256::digest(&wav)), receipt.sha256);
        }
    }
    Ok(())
}

async fn lifecycle(path: &Path) -> Result<()> {
    let mut daemon = NativeDaemon::start(path).await?;
    let initial = wait_state(
        path,
        "declare the microphone available",
        STATE_BUDGET,
        |state| state["capabilities"]["microphone"] == true,
    )
    .await?;
    assert_eq!(initial["connection"], "offline");
    assert!(initial["recording_signal"].is_null());
    start_meeting(path, "Synthetic two-track PCM").await?;
    wait_state(path, "start the recording", STATE_BUDGET, |state| {
        state["recording"]["phase"] == "recording"
    })
    .await?;
    daemon.send_originals().await?;
    let active = wait_state(
        path,
        "count the ten seconds of samples on both tracks",
        STATE_BUDGET,
        |state| {
            state["recording_signal"]["mic_local"]["received_frames"] == RATE * ORIGINAL_SECONDS
                && state["recording_signal"]["remote_app"]["received_frames"]
                    == RATE * ORIGINAL_SECONDS
        },
    )
    .await?;
    assert_eq!(
        active["recording_signal"]["mic_local"]["condition"],
        "low_signal"
    );
    assert_eq!(
        active["recording_signal"]["mic_local"]["low_signal_ms"],
        10_000
    );
    assert_eq!(
        active["recording_signal"]["remote_app"]["condition"],
        "receiving"
    );
    assert_eq!(active["recording"]["sealed_chunks"], 20);
    assert_eq!(active["recording"]["gaps"], 0);
    let recording = active["recording"]["id"]
        .as_str()
        .context("recording id absent")?
        .to_owned();
    let epoch = active["recording_signal"]["capture_epoch_id"]
        .as_str()
        .context("epoch absent")?
        .to_owned();

    // Real receive-time delay, with daemon heartbeat commands still answered.
    // This peer deliberately sends no native gap: the test isolates the new
    // observational projection, not Swift's separate no_capture_frames guard.
    // The one wait with a real-time floor under it: the daemon will not call a
    // track delayed until `SAMPLE_DELIVERY_TIMEOUT_MS` of silence has actually
    // passed, so its budget starts from that and not from zero.
    let delayed = wait_state(
        path,
        "mark both tracks `delivery_delayed`",
        DELAYED_STATE_BUDGET,
        |state| {
            state["recording_signal"]["mic_local"]["condition"] == "delivery_delayed"
                && state["recording_signal"]["remote_app"]["condition"] == "delivery_delayed"
        },
    )
    .await?;
    assert_eq!(
        delayed["recording_signal"]["remote_app"]["condition"],
        "delivery_delayed"
    );
    assert_eq!(delayed["recording_signal"]["mic_local"]["low_signal_ms"], 0);
    assert_eq!(
        delayed["recording_signal"]["mic_local"]["received_frames"],
        80_000
    );
    assert_eq!(delayed["recording"]["phase"], "recording");
    assert_eq!(delayed["privacy_stopped"], false);
    assert_eq!(delayed["recording"]["gaps"], 0);
    assert_eq!(delayed["recording"]["sealed_chunks"], 20);
    ipc::request_async(path, IpcCommand::MeetingFinish).await?;
    let closed = wait_state(
        path,
        "return to idle after the meeting closed",
        STATE_BUDGET,
        |state| state["recording"]["phase"] == "idle",
    )
    .await?;
    assert!(closed["recording_signal"].is_null());
    assert_eq!(closed["recording"]["sealed_chunks"], 22);
    assert_eq!(closed["recording"]["gaps"], 0);
    check_originals(path, &recording, &epoch)?;
    ipc::request_async(path, IpcCommand::Shutdown).await?;
    let effects = daemon.finish().await?;
    assert_eq!(effects.starts, 2);
    assert_eq!(effects.stops, 2);
    assert!(
        effects.heartbeats > 0 && effects.heartbeats_after_originals > 0,
        "the silent interval must keep the pipe live"
    );

    let mut recovered = NativeDaemon::start(path).await?;
    let state = wait_state(
        path,
        "recognise the native helper after restarting",
        STATE_BUDGET,
        |state| state["capabilities"]["native_helper"] == true,
    )
    .await?;
    // A shutdown nobody asked to stop comes back working.
    assert_eq!(state["privacy_stopped"], false);
    assert!(state["recording_signal"].is_null());
    assert_eq!(state["recording"]["id"], recording);
    assert_eq!(state["recording"]["phase"], "idle");
    check_originals(path, &recording, &epoch)?;
    ipc::request_async(path, IpcCommand::Shutdown).await?;
    let effects = recovered.finish().await?;
    assert_eq!(
        effects.starts, 0,
        "recovery must not start another producer"
    );
    assert_eq!(effects.stops, 0);
    Ok(())
}

#[tokio::test]
async fn real_daemon_observes_synthetic_tracks_without_discarding_originals() -> Result<()> {
    let directory = tempfile::Builder::new()
        .prefix("orbit-signal-")
        .tempdir_in("/tmp")?;
    tokio::time::timeout(LIFECYCLE_BUDGET, lifecycle(directory.path()))
        .await
        .with_context(|| {
            format!(
                "the synthetic lifecycle ran out of its {LIFECYCLE_BUDGET:?} without any of its \
                 waits running out first, so the jam was not in a state wait but in a \
                 request, a startup or an exit"
            )
        })??;
    Ok(())
}

#[tokio::test]
async fn managed_daemon_seals_audio_and_exits_when_its_parent_disappears() -> Result<()> {
    let directory = tempfile::Builder::new()
        .prefix("orbit-parent-")
        .tempdir_in("/tmp")?;
    let path = directory.path();
    let mut parent = Command::new("/bin/sleep")
        .arg("60")
        .kill_on_drop(true)
        .spawn()?;
    let parent_pid = parent.id().context("synthetic parent PID absent")?;
    let mut daemon = NativeDaemon::start_with_parent(path, Some(parent_pid)).await?;

    wait_state(
        path,
        "declare the microphone available",
        STATE_BUDGET,
        |state| state["capabilities"]["microphone"] == true,
    )
    .await?;
    start_meeting(path, "Synthetic parent loss").await?;
    wait_state(path, "start the recording", STATE_BUDGET, |state| {
        state["recording"]["phase"] == "recording"
    })
    .await?;
    daemon.send_originals().await?;
    let active = wait_state(path, "seal the twenty chunks", STATE_BUDGET, |state| {
        state["recording"]["sealed_chunks"] == 20
    })
    .await?;
    let recording = active["recording"]["id"]
        .as_str()
        .context("recording id absent")?
        .to_owned();
    let epoch = active["recording_signal"]["capture_epoch_id"]
        .as_str()
        .context("epoch absent")?
        .to_owned();

    parent.start_kill()?;
    parent.wait().await?;
    let effects = tokio::time::timeout(PARENT_LOSS_EXIT_BUDGET, daemon.finish())
        .await
        .with_context(|| {
            format!(
                "the managed daemon did not exit within {PARENT_LOSS_EXIT_BUDGET:?} \
                 after its parent vanished"
            )
        })??;
    assert_eq!(effects.stops, 2, "both live tracks must drain before exit");
    assert!(!path.join("orbit.sock").exists());

    let mut recovered = NativeDaemon::start(path).await?;
    let state = wait_state(
        path,
        "recognise the native helper after restarting",
        STATE_BUDGET,
        |state| state["capabilities"]["native_helper"] == true,
    )
    .await?;
    // The lost parent cut the recording; the restart comes back
    // working, with the recording interrupted and its fragments kept.
    assert_eq!(state["privacy_stopped"], false);
    assert_eq!(state["recording"]["id"], recording);
    assert_eq!(state["recording"]["phase"], "interrupted");
    assert_eq!(state["recording"]["sealed_chunks"], 22);
    assert!(state["recording_signal"].is_null());
    check_originals(path, &recording, &epoch)?;
    ipc::request_async(path, IpcCommand::Shutdown).await?;
    recovered.finish().await?;
    Ok(())
}
