//! The whole daemon under supervision, on a thread of its own with its own
//! runtime, as Orbit.app runs it:
//!
//! - an actor past its budget ends the daemon with a failure, which the
//!   app answers with a whole-daemon relaunch (`daemon_thread::relaunches`);
//! - a panic of the loop itself ends the daemon's thread with a panic, as
//!   before, and the next daemon resumes the meeting that was live, as after
//!   any crash.
//!
//! The helper is a relay of two `cat`s over FIFOs, and this test plays its
//! side of the protocol, as the integration tests do (`helper_relay.rs`).
//! Synthetic audio and permissions; nothing of macOS.
use crate::daemon::run::run_with_helper_inner;
use anyhow::{Context, Result, bail};
use base64::{Engine, engine::general_purpose::STANDARD};
use orbit_core::protocol::{Command, MeetingAudio};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

#[path = "../../../../../orbit-ipc/tests/support/status.rs"]
mod status;

/// A cold daemon's start and its first meeting under the whole suite's load;
/// it bounds a hang, it decides nothing (`helper_relay::CONNECT_BUDGET`).
const BUDGET: Duration = Duration::from_secs(60);
const RATE: u64 = 8_000;
const FRAMES: u64 = RATE / 4;

/// The daemon's stack, as the app gives its thread (`DAEMON_STACK_BYTES`).
const STACK: usize = 8 * 1024 * 1024;

type Daemon = std::thread::JoinHandle<Result<()>>;

fn daemon(paths: crate::config::Paths, helper: Option<PathBuf>) -> Result<Daemon> {
    Ok(std::thread::Builder::new()
        .name("orbit-daemon".into())
        .stack_size(STACK)
        .spawn(move || {
            tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()?
                .block_on(run_with_helper_inner(paths, true, helper, None))
        })?)
}

fn paths(root: &Path) -> Result<crate::config::Paths> {
    let paths = crate::config::Paths::resolve(Some(root.to_owned()))?;
    paths.save_settings(&crate::config::Settings::default())?;
    Ok(paths)
}

async fn wait_until(root: &Path, what: &str, ready: impl Fn(&Value) -> bool) -> Result<Value> {
    let started = Instant::now();
    loop {
        let last = match status::status_async(root).await {
            Ok(state) if ready(&state) => return Ok(state),
            Ok(state) => state.to_string(),
            Err(error) => error.to_string(),
        };
        if started.elapsed() > BUDGET {
            bail!("the daemon never got to {what} within {BUDGET:?}: {last}");
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

/// How the daemon's thread ended, once it did: a thread that is still
/// running past the budget fails the test instead of being joined.
async fn ended(daemon: Daemon) -> Result<std::thread::Result<Result<()>>> {
    let started = Instant::now();
    while !daemon.is_finished() {
        anyhow::ensure!(
            started.elapsed() < BUDGET,
            "the daemon's thread did not end within {BUDGET:?}"
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    Ok(daemon.join())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_actor_past_its_budget_ends_the_daemon_with_its_failure() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let paths = paths(&directory.path().join("orbit"))?;
    let root = paths.root.clone();
    let daemon = daemon(paths, None)?;
    wait_until(&root, "be ready", |state| {
        state["recovery"]["state"] == "ready"
    })
    .await?;
    std::fs::create_dir_all(root.join("test-faults"))?;
    std::fs::write(root.join("test-faults/store_writer"), "")?;
    let started = Instant::now();
    let outcome = ended(daemon).await?;
    let Ok(Err(failure)) = outcome else {
        bail!(
            "the daemon did not end with a failure {:?} after the writer spent its budget",
            started.elapsed()
        );
    };
    assert!(
        failure.to_string().contains("store_writer"),
        "the failure does not name the actor: {failure:#}"
    );
    let log = std::fs::read_to_string(root.join("logs/daemon.log"))?;
    assert!(log.contains("\"relaunch\":\"daemon\""), "{log}");
    assert!(
        !log.contains("injected"),
        "the panic's message reached the log"
    );
    Ok(())
}

/// The helper stand-in: two `cat`s relaying FIFOs this test reads and writes.
struct Relay {
    script: PathBuf,
    commands: PathBuf,
    events: PathBuf,
}

impl Relay {
    fn create(directory: &Path, name: &str) -> Result<Self> {
        use std::os::unix::fs::PermissionsExt;
        let relay = Self {
            script: directory.join(name),
            commands: directory.join(format!("{name}.commands")),
            events: directory.join(format!("{name}.events")),
        };
        for fifo in [&relay.commands, &relay.events] {
            let made = std::process::Command::new("mkfifo")
                .args(["-m", "600"])
                .arg(fifo)
                .status()?;
            anyhow::ensure!(made.success(), "mkfifo failed");
        }
        let source = directory.join(format!("{name}.txt"));
        std::fs::write(
            &source,
            format!(
                "#!/bin/sh\n/bin/cat '{}' &\nexec /bin/cat > '{}'\n",
                relay.events.display(),
                relay.commands.display()
            ),
        )?;
        // Written by `cp`, never held open for writing here (`ETXTBSY`).
        let copied = std::process::Command::new("/bin/cp")
            .arg(&source)
            .arg(&relay.script)
            .status()?;
        anyhow::ensure!(copied.success(), "cp of the relay failed");
        std::fs::set_permissions(&relay.script, std::fs::Permissions::from_mode(0o700))?;
        Ok(relay)
    }

    /// The test's two ends, once the daemon launched the relay.
    async fn connect(&self) -> Result<(tokio::fs::File, tokio::fs::File)> {
        let (commands, events) = (self.commands.clone(), self.events.clone());
        let opened = tokio::task::spawn_blocking(move || -> std::io::Result<_> {
            let events = std::fs::OpenOptions::new().write(true).open(events)?;
            let commands = std::fs::File::open(commands)?;
            Ok((commands, events))
        });
        let (commands, events) = tokio::time::timeout(BUDGET, opened)
            .await
            .context("the daemon never launched its helper")???;
        Ok((
            tokio::fs::File::from_std(commands),
            tokio::fs::File::from_std(events),
        ))
    }
}

/// A state reply with the microphone authorized and no calendar.
fn state() -> Result<Value> {
    let mut state: Value = serde_json::from_str(include_str!(
        "../../../../../../contracts/fixtures/native/results/status.json"
    ))?;
    state["permissions"] = json!({"microphone":"authorized","screen":true,
        "accessibility":false,"calendars":"not_determined"});
    state["calendar"]["authorization"] = json!("not_determined");
    Ok(state)
}

fn audio(stream: &str, sequence: u64) -> Value {
    json!({"v":1,"event":"audio","source":"mic_local","stream_id":stream,
        "sequence":sequence,"frames":FRAMES,"sample_rate":RATE,"channels":1,
        "clock_domain":"mach_absolute_time",
        "monotonic_ns":1_000_000_000 + sequence * 250_000_000,
        "pcm_s16le_base64":STANDARD.encode(256_i16.to_le_bytes().repeat(FRAMES as usize)),
        "levels":{"rms_dbfs":-30.1,"peak_dbfs":-30.1,"clipped_frames":0},
        "processing":"unprocessed"})
}

async fn send(events: &mut tokio::fs::File, event: Value) -> Result<()> {
    let mut bytes = serde_json::to_vec(&event)?;
    bytes.push(b'\n');
    events.write_all(&bytes).await?;
    events.flush().await?;
    Ok(())
}

/// What the helper's side saw: the streams the daemon opened, in order.
type Starts = tokio::sync::mpsc::UnboundedReceiver<String>;

/// The helper's side: `ready`, every order answered, and a quarter second
/// of microphone audio every quarter second on each open stream.
fn peer(
    commands: tokio::fs::File,
    mut events: tokio::fs::File,
) -> (Starts, tokio::task::JoinHandle<Result<()>>) {
    let (started, starts) = tokio::sync::mpsc::unbounded_channel();
    let task = tokio::spawn(async move {
        let mut ready: Value = serde_json::from_str(include_str!(
            "../../../../../../contracts/fixtures/native/events/ready.json"
        ))?;
        ready["selected_text_available"] = json!(false);
        send(&mut events, ready).await?;
        let mut lines = BufReader::new(commands).lines();
        let mut stream: Option<(String, u64)> = None;
        let mut ticks = tokio::time::interval(Duration::from_millis(250));
        loop {
            tokio::select! {
                _ = ticks.tick(), if stream.is_some() => {
                    if let Some((id, sequence)) = stream.as_mut() {
                        send(&mut events, audio(id, *sequence)).await?;
                        *sequence += 1;
                    }
                }
                line = lines.next_line() => {
                    let Some(line) = line? else { return Ok(()) };
                    let request: Value = serde_json::from_str(&line)?;
                    let result = match request["command"].as_str() {
                        Some("hello" | "heartbeat" | "status") => state()?,
                        Some("audio_start") => {
                            let id = request["stream_id"].as_str().context("stream")?.to_owned();
                            let _ = started.send(id.clone());
                            stream = Some((id.clone(), 0));
                            json!({"stream_id":id,"source":"mic_local","sample_rate":RATE,"channels":1})
                        }
                        Some("audio_stop") => {
                            stream = None;
                            json!({"stream_id":request["stream_id"],"drain_complete":true})
                        }
                        Some("capture_stop" | "privacy_stop") => {
                            stream = None;
                            let mut result = state()?;
                            result["privacy_stopped"] = json!(request["command"] == "privacy_stop");
                            result["drain_complete"] = json!(true);
                            result
                        }
                        Some("shutdown") => json!({"drain_complete":true}),
                        _ => json!({}),
                    };
                    send(&mut events, json!({"v":1,"event":"response","id":request["id"],
                        "ok":true,"result":result})).await?;
                }
            }
        }
    });
    (starts, task)
}

fn sealed(state: &Value) -> u64 {
    state["recording"]["sealed_chunks"].as_u64().unwrap_or(0)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_loop_that_panics_ends_the_daemon_and_the_next_one_resumes_the_meeting() -> Result<()> {
    let directory = tempfile::Builder::new()
        .prefix("orbit-loop-")
        .tempdir_in("/tmp")?;
    let paths = paths(&directory.path().join("orbit"))?;
    let root = paths.root.clone();
    let first = Relay::create(directory.path(), "first")?;
    let daemon_one = daemon(paths.clone(), Some(first.script.clone()))?;
    let (commands, events) = first.connect().await?;
    let (mut starts, peer_one) = peer(commands, events);
    wait_until(&root, "know the microphone", |state| {
        state["recovery"]["state"] == "ready" && state["capabilities"]["microphone"] == true
    })
    .await?;
    crate::ipc::request(
        &paths,
        Command::MeetingStart {
            title: "Reunión sintética del pánico".into(),
            audio: MeetingAudio::MicrophoneOnly,
            duration_minutes: None,
            event: None,
        },
    )
    .await?;
    let opened = tokio::time::timeout(BUDGET, starts.recv())
        .await?
        .context("the meeting never opened its stream")?;
    let live = wait_until(&root, "seal two chunks", |state| {
        state["recording"]["phase"] == "recording" && sealed(state) >= 2
    })
    .await?;
    let recording = live["recording"]["id"].as_str().context("id")?.to_owned();

    std::fs::create_dir_all(root.join("test-faults"))?;
    std::fs::write(root.join("test-faults/loop"), "")?;
    let outcome = ended(daemon_one).await?;
    assert!(
        outcome.is_err(),
        "a loop panic must end the daemon's thread without a result (`Vanished`)"
    );
    peer_one.abort();

    let second = Relay::create(directory.path(), "second")?;
    let daemon_two = daemon(paths.clone(), Some(second.script.clone()))?;
    let (commands, events) = second.connect().await?;
    let (mut starts, peer_two) = peer(commands, events);
    let resumed = tokio::time::timeout(BUDGET, starts.recv())
        .await?
        .context("the next daemon did not resume the meeting")?;
    assert_ne!(resumed, opened, "a resumed meeting opens a new stream");
    let after = wait_until(&root, "record the same meeting again", |state| {
        state["recording"]["phase"] == "recording" && sealed(state) > sealed(&live)
    })
    .await?;
    assert_eq!(after["recording"]["id"], recording.as_str());
    crate::ipc::request(&paths, Command::MeetingFinish).await?;
    wait_until(&root, "close the meeting", |state| {
        state["recording"]["phase"] == "idle"
    })
    .await?;
    crate::ipc::request(&paths, Command::Shutdown).await?;
    let closed = ended(daemon_two).await?;
    assert!(
        matches!(closed, Ok(Ok(()))),
        "the second daemon did not close in order"
    );
    peer_two.abort();
    Ok(())
}
