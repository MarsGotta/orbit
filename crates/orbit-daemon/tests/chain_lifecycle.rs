//! The chain app → daemon → helper closes whole.
//!
//! A real daemon with a real helper child — the relay or the stubborn script
//! of `support/helper_relay.rs` — launched the way the app launches it: with
//! `--helper`, `--parent-pid` and a process group of its own. No macOS, no
//! permission, no capture: what is proved is the daemon's side of the
//! lifecycle, never how `OrbitNative` behaves (that is XCTest and the Mac).
use anyhow::{Context, Result, bail, ensure};
use orbit_core::protocol::Command as IpcCommand;
use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader, Write},
    path::Path,
    process::{Child, Command, Stdio},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

#[path = "support/helper_relay.rs"]
mod helper_relay;
#[path = "support/ipc.rs"]
mod ipc;
use helper_relay::{
    Relay, kill_group, last_logged, signal_own_daemon, std_daemon, stubborn_helper,
    wait_group_empty,
};

/// What the daemon allows itself to close, from the latch to the helper's
/// grace running out: the two constants the daemon itself runs on, read from
/// its crate rather than copied, so this file cannot drift from them.
fn closing_ceiling() -> Duration {
    orbit_daemon::daemon::SHUTDOWN_BUDGET + orbit_daemon::native::HELPER_EXIT_GRACE
}

/// A cold daemon's first answer. The suite already bounds it at sixty seconds
/// under full load; this bounds a hang, it does not decide a result.
const READY_BUDGET: Duration = Duration::from_secs(60);

/// Room over [`closing_ceiling`] for the daemon to exit once it has decided to:
/// the last snapshot, removing the socket, the process exit. A shutdown request
/// was measured at 590 ms at its worst under load (`track_signal_lifecycle`),
/// and five times that is under three seconds; this is ten.
const EXIT_ROOM: Duration = Duration::from_secs(10);

struct Chain {
    child: Child,
    group: u32,
    peer: Option<JoinHandle<Result<Vec<String>>>>,
}

impl Chain {
    /// A daemon whose helper is the relay, and a peer thread that answers the
    /// protocol like a helper and records every command it was sent.
    fn with_relay(path: &Path) -> Result<Self> {
        let relay = Relay::create(path, "helper")?;
        let child = std_daemon(path, &relay.script)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()?;
        let group = child.id();
        let mut chain = Self {
            child,
            group,
            peer: None,
        };
        let (commands, mut events) = relay.connect()?;
        chain.peer = Some(thread::spawn(move || -> Result<Vec<String>> {
            send(&mut events, &helper_relay::ready_event()?)?;
            let mut seen = Vec::new();
            for line in BufReader::new(commands).lines() {
                let command: Value = serde_json::from_str(&line?)?;
                let name = command["command"]
                    .as_str()
                    .context("native command without a name")?
                    .to_owned();
                let result = match name.as_str() {
                    "heartbeat" | "hello" | "status" => helper_relay::state_result(json!({
                        "microphone":"not_determined","screen":false,"accessibility":false,
                        "calendars":"not_determined"}))?,
                    "playback_stop" => json!({}),
                    "privacy_stop" => {
                        let mut result = helper_relay::state_result(json!({
                            "microphone":"not_determined","screen":false,"accessibility":false,
                            "calendars":"not_determined"}))?;
                        result["privacy_stopped"] = json!(true);
                        result["drain_complete"] = json!(true);
                        result
                    }
                    // Closing cuts the capture without the latch.
                    "capture_stop" => {
                        let mut result = helper_relay::state_result(json!({
                            "microphone":"not_determined","screen":false,"accessibility":false,
                            "calendars":"not_determined"}))?;
                        result["drain_complete"] = json!(true);
                        result
                    }
                    "shutdown" => json!({"drain_complete":true}),
                    other => bail!("native order outside this fixture: {other}"),
                };
                seen.push(name);
                send(
                    &mut events,
                    &json!({"v":1,"event":"response","id":command["id"],"ok":true,"result":result}),
                )?;
            }
            // The daemon closed the helper's input: the relay's `cat` ended and
            // this read reached the end of the FIFO.
            Ok(seen)
        }));
        wait_helper_ready(path, &mut chain.child)?;
        Ok(chain)
    }

    /// A daemon whose helper ignores `shutdown`, EOF and `SIGTERM`.
    fn with_stubborn_helper(path: &Path) -> Result<Self> {
        let helper = stubborn_helper(path, "stubborn")?;
        let child = std_daemon(path, &helper)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()?;
        let group = child.id();
        let mut chain = Self {
            child,
            group,
            peer: None,
        };
        wait_helper_ready(path, &mut chain.child)?;
        Ok(chain)
    }

    /// Waits for the daemon to leave, requires a clean exit, and returns the
    /// commands the relay's peer saw, in order.
    fn wait_clean_exit(&mut self, within: Duration) -> Result<Vec<String>> {
        let deadline = Instant::now() + within;
        let status = loop {
            if let Some(status) = self.child.try_wait()? {
                break status;
            }
            ensure!(
                Instant::now() < deadline,
                "the daemon did not exit within {within:?} after being asked to close"
            );
            thread::sleep(Duration::from_millis(20));
        };
        ensure!(
            status.success(),
            "the orderly close must end successfully and ended with {status}"
        );
        match self.peer.take() {
            Some(peer) => peer
                .join()
                .map_err(|_| anyhow::anyhow!("the test helper thread failed"))?,
            None => Ok(Vec::new()),
        }
    }
}

impl Drop for Chain {
    fn drop(&mut self) {
        // Failure cleanup only: the group holds this test's daemon and the
        // helpers it started, nothing else, and the daemon is not yet reaped.
        kill_group(self.group);
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(peer) = self.peer.take() {
            let _ = peer.join();
        }
    }
}

fn send(writer: &mut impl Write, value: &Value) -> Result<()> {
    serde_json::to_writer(&mut *writer, value)?;
    writer.write_all(b"\n")?;
    writer.flush()?;
    Ok(())
}

fn wait_helper_ready(path: &Path, child: &mut Child) -> Result<()> {
    let deadline = Instant::now() + READY_BUDGET;
    loop {
        if let Some(status) = child.try_wait()? {
            bail!("the daemon exited ({status}) before declaring its helper");
        }
        if ipc::status(path).is_ok_and(|state| state["capabilities"]["native_helper"] == true) {
            return Ok(());
        }
        ensure!(
            Instant::now() < deadline,
            "the test helper was not declared within {READY_BUDGET:?}"
        );
        thread::sleep(Duration::from_millis(20));
    }
}

fn workspace(prefix: &str) -> Result<tempfile::TempDir> {
    // Short and under /tmp: Darwin's Unix socket path limit is 104 bytes.
    Ok(tempfile::Builder::new().prefix(prefix).tempdir_in("/tmp")?)
}

/// An orderly close sends the helper `shutdown`, then closes its
/// input, and the helper that leaves by itself is not killed.
#[test]
fn an_orderly_close_tells_the_helper_to_shut_down_and_closes_its_input() -> Result<()> {
    let directory = workspace("orbit-close-")?;
    let path = directory.path();
    let mut chain = Chain::with_relay(path)?;
    ipc::request(path, IpcCommand::Shutdown).context("the close was not accepted")?;
    let seen = chain.wait_clean_exit(closing_ceiling() + EXIT_ROOM)?;
    assert_eq!(
        seen.last().map(String::as_str),
        Some("shutdown"),
        "the last order to the helper must be `shutdown`, and the helper only stops \
         reading when the daemon closes its input; it saw: {seen:?}"
    );
    let stopped = last_logged(path, "helper_stopped")
        .context("the daemon left no record of how its helper ended")?;
    assert_eq!(
        stopped["how"], "exited",
        "a helper that exits by itself is not killed: {stopped}"
    );
    assert_eq!(stopped["asked"], "true", "{stopped}");
    wait_group_empty(chain.group, "orderly close")?;
    Ok(())
}

/// A helper that ignores `shutdown` and EOF is killed, and only
/// once its grace has run out; nothing of the chain's group is left alive.
#[test]
fn a_helper_that_ignores_shutdown_is_killed_only_after_its_grace() -> Result<()> {
    let directory = workspace("orbit-kill-")?;
    let path = directory.path();
    let mut chain = Chain::with_stubborn_helper(path)?;
    let asked = Instant::now();
    ipc::request(path, IpcCommand::Shutdown).context("the close was not accepted")?;
    chain.wait_clean_exit(closing_ceiling() + EXIT_ROOM)?;
    let took = asked.elapsed();
    let grace = orbit_daemon::native::HELPER_EXIT_GRACE;
    // A lower bound, so no clock can decide it: the daemon cannot have killed
    // the helper before the grace it gives it has passed.
    assert!(
        took >= grace,
        "the helper was killed at {took:?}, before its grace of {grace:?}"
    );
    let stopped = last_logged(path, "helper_stopped")
        .context("the daemon left no record of how its helper ended")?;
    assert_eq!(
        stopped["how"], "killed",
        "a helper that does not exit within its grace is killed: {stopped}"
    );
    assert_eq!(
        stopped["grace_ms"],
        grace.as_millis().to_string(),
        "the record says how long it waited: {stopped}"
    );
    wait_group_empty(chain.group, "stubborn helper")?;
    Ok(())
}

/// SIGTERM — what launchd sends at logout — and SIGINT run the
/// same durable closure: the helper is told to shut down, the socket goes,
/// and the process exits cleanly instead of dying by the signal.
#[test]
fn sigterm_and_sigint_run_the_same_orderly_closure() -> Result<()> {
    for signal in ["TERM", "INT"] {
        let directory = workspace("orbit-signal-")?;
        let path = directory.path();
        let mut chain = Chain::with_relay(path)?;
        signal_own_daemon(chain.child.id(), signal)?;
        let seen = chain
            .wait_clean_exit(closing_ceiling() + EXIT_ROOM)
            .with_context(|| format!("with SIG{signal}"))?;
        assert_eq!(
            seen.last().map(String::as_str),
            Some("shutdown"),
            "with SIG{signal} the helper also receives `shutdown`; it saw: {seen:?}"
        );
        assert!(
            !path.join("orbit.sock").exists(),
            "with SIG{signal} the socket must be removed"
        );
        let received = last_logged(path, "shutdown_signal")
            .with_context(|| format!("the daemon did not log SIG{signal}"))?;
        assert_eq!(received["signal"], format!("SIG{signal}"));
        let stopped = last_logged(path, "helper_stopped")
            .with_context(|| format!("no record of the helper after SIG{signal}"))?;
        assert_eq!(stopped["how"], "exited", "SIG{signal}: {stopped}");
        wait_group_empty(chain.group, "signal closure")?;
    }
    Ok(())
}

/// How long the startup-error case watches for a helper launched while the
/// daemon is still starting. Not a wait for the result: a daemon that launches
/// no helper before its startup has succeeded passes whatever this is. It only
/// has to be long enough for the defect to show when it is there: the old
/// daemon launched its helper concurrently with the rest of its startup, and
/// that launch takes milliseconds; this is one second.
const EARLY_HELPER_WINDOW: Duration = Duration::from_secs(1);

/// An error while the daemon starts ends through the ordinary
/// return, not `std::process::exit`, and leaves no helper behind.
///
/// The device credential is a FIFO, which stands for the slow read it is on the
/// Mac (a Keychain prompt): the daemon blocks on it, the test watches for a
/// helper launched meanwhile, and then feeds it bytes that are not UTF-8, so
/// the read — and the startup — fail.
///
/// Why two assertions: the helper surviving the failure was a race in the old
/// daemon (the helper task noticed the dropped command channel and killed its
/// child before `exit` in 29 of 30 measured runs), so the survivor alone is a
/// red that shows up once in thirty. What makes it impossible is that no helper
/// exists until the startup has succeeded, and that is asserted directly.
#[test]
fn a_startup_error_leaves_no_helper_behind() -> Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    let directory = workspace("orbit-startup-")?;
    let path = directory.path();
    let credential = path.join("development-device.key");
    let status = Command::new("mkfifo")
        .args(["-m", "600"])
        .arg(&credential)
        .status()?;
    ensure!(status.success(), "mkfifo ended with {status}");
    let helper = stubborn_helper(path, "stubborn")?;
    let mut child = std_daemon(path, &helper)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()?;
    let group = child.id();
    struct Cleanup(u32);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            kill_group(self.0);
        }
    }
    let _cleanup = Cleanup(group);
    // A non-blocking open for writing only succeeds once a reader holds the
    // FIFO: that is the daemon, blocked on its credential.
    let deadline = Instant::now() + READY_BUDGET;
    let mut feed = loop {
        match std::fs::OpenOptions::new()
            .write(true)
            .custom_flags(rustix::fs::OFlags::NONBLOCK.bits() as i32)
            .open(&credential)
        {
            Ok(file) => break file,
            Err(_) if Instant::now() < deadline => thread::sleep(Duration::from_millis(10)),
            Err(error) => bail!("the daemon never got to read its credential: {error}"),
        }
        if let Some(status) = child.try_wait()? {
            bail!("the daemon ended ({status}) before reading its credential");
        }
    };
    let launched = path.join("stubborn.pid");
    let window = Instant::now() + EARLY_HELPER_WINDOW;
    while !launched.exists() && Instant::now() < window {
        thread::sleep(Duration::from_millis(10));
    }
    let early = launched.exists();
    feed.write_all(&[0xff, 0xfe, 0xfd])?;
    drop(feed);
    let deadline = Instant::now() + READY_BUDGET;
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        ensure!(
            Instant::now() < deadline,
            "the daemon did not end within {READY_BUDGET:?} with its credential unreadable"
        );
        thread::sleep(Duration::from_millis(20));
    };
    let mut stderr = String::new();
    if let Some(mut pipe) = child.stderr.take() {
        std::io::Read::read_to_string(&mut pipe, &mut stderr)?;
    }
    assert!(
        !status.success(),
        "an unreadable credential must prevent the startup; stderr: {stderr}"
    );
    assert_eq!(
        status.code(),
        Some(1),
        "the error leaves through the ordinary return, with its code; stderr: {stderr}"
    );
    assert!(
        stderr.contains("UTF-8"),
        "the refusal names its cause; stderr: {stderr}"
    );
    wait_group_empty(group, "the helper survived a startup error")?;
    assert!(
        !early,
        "the daemon launched its helper while still starting; a startup error \
         can leave it orphaned"
    );
    Ok(())
}
