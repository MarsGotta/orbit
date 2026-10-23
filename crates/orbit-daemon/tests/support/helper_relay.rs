//! The production launch path for integration tests.
//!
//! Production starts `orbit daemon --helper <OrbitNative>` and the daemon starts
//! `OrbitNative --stdio --managed` as its own child. These tests do the same
//! with a shell script standing where `OrbitNative` stands, so the daemon
//! launches, supervises, tells to shut down and, if it must, kills a real
//! child exactly as it would the real helper. Nothing here reads
//! `native/macos`, asks macOS for anything or claims to prove a permission.
//!
//! Two scripts:
//!
//! - [`Relay`] is two `cat`s. One copies the daemon's commands (the helper's
//!   stdin) into a FIFO the test reads; the other copies a FIFO the test writes
//!   into the helper's stdout. The test then plays the helper's side of the
//!   protocol over the FIFOs, as the peers of the retired helper-hosted mode
//!   did over the daemon's own stdio.
//! - [`stubborn_helper`] says `ready` and then ignores everything: `shutdown`,
//!   the end of its input and `SIGTERM`. Only `SIGKILL` ends it, which is what
//!   the daemon must fall back to when its helper does not leave in time.
//!
//! Every daemon started through [`std_daemon`] or [`tokio_daemon`] gets a
//! process group of its own, as the app gives it, and `--parent-pid` of this
//! test process, as the app passes its own: a test binary that is interrupted
//! leaves no daemon behind. The group is also what the cleanup signals, and it
//! is safe to: it holds only the daemon this test started and the helpers that
//! daemon started, and its id — the daemon's pid — cannot be handed to another
//! process while any member of the group, the unreaped daemon included, exists.
#![allow(dead_code)]

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use std::{
    ffi::OsString,
    fs::{File, OpenOptions},
    path::{Path, PathBuf},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

/// How long a test waits for the daemon to launch its helper and for the relay
/// to reach its FIFOs. The daemon does this as part of starting, and the suite
/// already gives a cold daemon sixty seconds to answer its first status under
/// full load (`daemon_helper.rs`, `helper_pipe_lifecycle.rs`);
/// this bounds a hang with the same number, it does not decide a result.
pub const CONNECT_BUDGET: Duration = Duration::from_secs(60);

/// How long the group may take to empty once the daemon is gone. The daemon
/// reaps its helper before it exits, so the normal case is zero; what remains
/// is a straggler reparented to init, which reaps it in milliseconds. This
/// bounds a hang for a readable failure; it is not a measured wait.
pub const GROUP_EMPTY_BUDGET: Duration = Duration::from_secs(10);

/// A complete native-v1 ready for the bundled helper. An incomplete ready
/// is correctly rejected by the daemon.
pub fn ready_event() -> Result<Value> {
    let mut event: Value = serde_json::from_str(include_str!(
        "../../../../contracts/fixtures/native/events/ready.json"
    ))?;
    event["selected_text_available"] = json!(false);
    Ok(event)
}

/// Complete state reply from the generated native-v1 fixture. The synthetic
/// permission is explicit for each test; a partial status is invalid on wire.
pub fn state_result(permissions: Value) -> Result<Value> {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../../contracts/fixtures/native/results/hello.json"
    ))?;
    let mut result = fixture;
    result["calendar"]["authorization"] = permissions["calendars"].clone();
    result["permissions"] = permissions;
    Ok(result)
}

pub fn ready_line() -> Result<String> {
    let line = serde_json::to_string(&ready_event()?)?;
    anyhow::ensure!(!line.contains('\''), "ready fixture is not shell quotable");
    Ok(line)
}

/// The helper stand-in and the two FIFOs it relays.
pub struct Relay {
    pub script: PathBuf,
    /// Written by the relay: what the daemon sends to the helper.
    commands: PathBuf,
    /// Read by the relay: what the helper sends to the daemon.
    events: PathBuf,
    arguments: PathBuf,
}

impl Relay {
    /// Writes the relay script and its two FIFOs into `directory`.
    pub fn create(directory: &Path, name: &str) -> Result<Self> {
        let relay = Self {
            script: directory.join(name),
            commands: directory.join(format!("{name}.commands")),
            events: directory.join(format!("{name}.events")),
            arguments: directory.join(format!("{name}.args")),
        };
        for path in [
            &relay.script,
            &relay.commands,
            &relay.events,
            &relay.arguments,
        ] {
            quotable(path)?;
        }
        for fifo in [&relay.commands, &relay.events] {
            make_fifo(fifo)?;
        }
        // The background `cat` gets /dev/null as its stdin (no job control) and
        // keeps the helper's stdout; the foreground one becomes the helper's
        // pid and ends at the end of its input, which is the daemon closing it.
        write_script(
            &relay.script,
            &format!(
                "#!/bin/sh\n\
                 # Stands where OrbitNative stands for a test; no native effect of any kind.\n\
                 printf '%s\\n' \"$*\" >> '{arguments}'\n\
                 /bin/cat '{events}' &\n\
                 exec /bin/cat > '{commands}'\n",
                arguments = relay.arguments.display(),
                events = relay.events.display(),
                commands = relay.commands.display(),
            ),
        )?;
        Ok(relay)
    }

    /// Waits for the daemon to launch this relay and returns the test's two
    /// ends: the daemon's commands to read and the helper's events to write.
    pub fn connect(&self) -> Result<(File, File)> {
        let commands = self.commands.clone();
        let events = self.events.clone();
        let (sender, receiver) = mpsc::channel();
        // Opening a FIFO waits for its other end. Done on a thread so a daemon
        // that never launches its helper fails this test by name instead of
        // hanging it; the thread is left behind only in that failure.
        thread::spawn(move || {
            let opened = (|| -> std::io::Result<(File, File)> {
                let events = OpenOptions::new().write(true).open(&events)?;
                let commands = File::open(&commands)?;
                Ok((commands, events))
            })();
            let _ = sender.send(opened);
        });
        match receiver.recv_timeout(CONNECT_BUDGET) {
            Ok(opened) => Ok(opened?),
            Err(_) => bail!(
                "the daemon did not launch the test helper {} within {CONNECT_BUDGET:?}",
                self.script.display()
            ),
        }
    }

    /// Every launch of this relay, one line of arguments each.
    pub fn launches(&self) -> Vec<String> {
        std::fs::read_to_string(&self.arguments)
            .map(|text| text.lines().map(str::to_owned).collect())
            .unwrap_or_default()
    }
}

/// A helper that announces itself and then ignores `shutdown`, the end of its
/// input and `SIGTERM`. An ignored signal survives `exec`, so the `sleep` that
/// replaces the shell keeps ignoring it; only `SIGKILL` ends it.
///
/// It writes its pid to `<name>.pid` before anything else, so a test can tell
/// that it was launched without asking the machine.
pub fn stubborn_helper(directory: &Path, name: &str) -> Result<PathBuf> {
    let script = directory.join(name);
    let pid = directory.join(format!("{name}.pid"));
    quotable(&script)?;
    quotable(&pid)?;
    write_script(
        &script,
        &format!(
            "#!/bin/sh\n\
             # Ignores shutdown, EOF and SIGTERM on purpose; no native effect of any kind.\n\
             trap '' TERM\n\
             printf '%s\\n' \"$$\" > '{pid}'\n\
             printf '%s\\n' '{ready}'\n\
             exec /bin/sleep 3600\n",
            pid = pid.display(),
            ready = ready_line()?
        ),
    )?;
    Ok(script)
}

/// `orbit --dev --data-dir <path> daemon --helper <helper> --parent-pid <parent>`.
pub fn daemon_arguments(path: &Path, helper: &Path, parent: u32) -> Vec<OsString> {
    vec![
        "--dev".into(),
        "--data-dir".into(),
        path.as_os_str().to_owned(),
        "daemon".into(),
        "--helper".into(),
        helper.as_os_str().to_owned(),
        "--parent-pid".into(),
        parent.to_string().into(),
    ]
}

/// The daemon command with its own process group and this test process as its
/// parent, for `std::process`.
pub fn std_daemon(path: &Path, helper: &Path) -> std::process::Command {
    use std::os::unix::process::CommandExt;
    let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_orbit"));
    command
        .args(daemon_arguments(path, helper, std::process::id()))
        .process_group(0);
    command
}

/// The same for `tokio::process`. `parent` stands in for the app; without one
/// it is this test process.
pub fn tokio_daemon(path: &Path, helper: &Path, parent: Option<u32>) -> tokio::process::Command {
    let parent = parent.unwrap_or_else(std::process::id);
    let mut command = tokio::process::Command::new(env!("CARGO_BIN_EXE_orbit"));
    command
        .args(daemon_arguments(path, helper, parent))
        .process_group(0)
        .kill_on_drop(true);
    command
}

/// Whether any process of this group — the daemon's own, since the daemon is
/// its leader — still exists, zombies included. Signal 0 delivers nothing.
pub fn group_alive(group: u32) -> bool {
    let Some(pid) = i32::try_from(group)
        .ok()
        .and_then(rustix::process::Pid::from_raw)
    else {
        return false;
    };
    !matches!(
        rustix::process::test_kill_process_group(pid),
        Err(rustix::io::Errno::SRCH)
    )
}

/// Waits for this test's group to have no process left, and names what it was
/// waiting for if it never empties.
pub fn wait_group_empty(group: u32, what: &str) -> Result<()> {
    let deadline = Instant::now() + GROUP_EMPTY_BUDGET;
    while group_alive(group) {
        if Instant::now() >= deadline {
            bail!(
                "{what}: the chain's process group {group} still has live processes \
                 {GROUP_EMPTY_BUDGET:?} after the daemon exited"
            );
        }
        thread::sleep(Duration::from_millis(10));
    }
    Ok(())
}

/// Cleanup only: `SIGKILL` to this test's own group, before the daemon is
/// reaped so its id is still reserved. Never used to pass an assertion.
pub fn kill_group(group: u32) {
    if let Some(pid) = i32::try_from(group)
        .ok()
        .and_then(rustix::process::Pid::from_raw)
    {
        let _ = rustix::process::kill_process_group(pid, rustix::process::Signal::KILL);
    }
}

/// Sends a termination signal to one daemon this test started, by its pid, the
/// way launchd, `kill` or a terminal would. `name` is `TERM` or `INT`.
pub fn signal_own_daemon(pid: u32, name: &str) -> Result<()> {
    if !matches!(name, "TERM" | "INT") {
        bail!("signal not expected in this test: {name}");
    }
    let status = std::process::Command::new("/bin/kill")
        .args(["-s", name, &pid.to_string()])
        .status()
        .context("could not run /bin/kill")?;
    if !status.success() {
        bail!("/bin/kill -s {name} {pid} ended with {status}");
    }
    Ok(())
}

/// The daemon's own diagnostic lines, as JSON values.
pub fn daemon_log(path: &Path) -> Vec<serde_json::Value> {
    std::fs::read_to_string(path.join("logs/daemon.log"))
        .unwrap_or_default()
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect()
}

/// The fields of the last `event` line in the daemon's log.
pub fn last_logged(path: &Path, event: &str) -> Option<serde_json::Value> {
    daemon_log(path)
        .into_iter()
        .rev()
        .find(|line| line["event"] == event)
        .map(|line| line["fields"].clone())
}

/// Strict mode for the daemon's loop, as Home Assistant's `block_async_io`
/// is strict in its tests: a capture test fails when one loop turn took
/// longer than this, and names the turn's source, step and wait.
///
/// What it catches: a new blocking call on the loop (a synchronous wait with
/// no bound, a sleep, a lock held across the disk) in the paths these tests
/// drive. Why 5 s: the daemon only logs turns over its 1 s threshold; on
/// Linux the capture lifecycle's slowest turns were 499 ms at load 34 (six
/// runs) and a single awaited snapshot reached 972 ms in a unit test at load
/// 60–90. Five seconds is five times that worst, so the machine's load does
/// not decide the result; a hold this long in a test is a call that waits on
/// something that never answers fast. It bounds nothing in the daemon.
pub const STRICT_TURN_BOUND: Duration = Duration::from_secs(5);

/// The fields of every `daemon_turn_slow` line in the daemon's log: the
/// loop turns that took longer than the daemon's 1 s threshold.
pub fn slow_turns(path: &Path) -> Vec<serde_json::Value> {
    daemon_log(path)
        .into_iter()
        .filter(|line| line["event"] == "daemon_turn_slow")
        .map(|line| line["fields"].clone())
        .collect()
}

/// Fails when a loop turn of the daemon at `path` took longer than
/// [`STRICT_TURN_BOUND`], with what held it.
pub fn assert_no_held_turn(path: &Path) -> Result<()> {
    let bound = STRICT_TURN_BOUND.as_millis();
    let held: Vec<_> = slow_turns(path)
        .into_iter()
        .filter(|fields| {
            fields["took_ms"]
                .as_str()
                .and_then(|took| took.parse::<u128>().ok())
                .is_none_or(|took| took > bound)
        })
        .collect();
    if !held.is_empty() {
        bail!(
            "a loop turn held the daemon for more than {STRICT_TURN_BOUND:?}: {}",
            serde_json::to_string(&held)?
        );
    }
    Ok(())
}

fn quotable(path: &Path) -> Result<()> {
    let text = path.to_str().context("test path that is not UTF-8")?;
    if text.contains('\'') || text.contains('\n') {
        bail!("the test path cannot be quoted in sh: {text:?}");
    }
    Ok(())
}

fn make_fifo(path: &Path) -> Result<()> {
    // `mkfifo(1)` rather than a syscall: rustix does not offer mkfifo on Apple
    // targets, and this suite must build on the Mac.
    let status = std::process::Command::new("mkfifo")
        .args(["-m", "600"])
        .arg(path)
        .status()
        .context("could not run mkfifo")?;
    if !status.success() {
        bail!("mkfifo {} ended with {status}", path.display());
    }
    Ok(())
}

/// Writes a script this test will have the daemon execute, without this
/// process ever holding it open for writing. The suite forks constantly (every
/// daemon is a process), a fork inherits an open descriptor until its
/// `exec`, and executing a file that someone holds open for writing fails with
/// `ETXTBSY` — which the daemon reads, correctly, as a helper that fell, and
/// latches. `cp` writes it from a process of its own instead.
fn write_script(path: &Path, body: &str) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mut source = path.as_os_str().to_owned();
    source.push(".txt");
    let source = PathBuf::from(source);
    std::fs::write(&source, body)?;
    let copied = std::process::Command::new("/bin/cp")
        .arg(&source)
        .arg(path)
        .status()
        .context("could not run /bin/cp")?;
    if !copied.success() {
        bail!("cp {} ended with {copied}", path.display());
    }
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))?;
    Ok(())
}
