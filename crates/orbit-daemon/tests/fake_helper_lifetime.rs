//! A test that dies takes its fake helper with it.
//!
//! A test can be killed where no `Drop` runs: SIGKILL, a runner's own timeout,
//! the machine running out of memory. Whatever it started must not outlive it,
//! because a leftover script keeps every descriptor it inherited, such as a
//! build lock held by whoever ran the tests or the gate's output pipe, and
//! whoever waits on those waits for hours.
//!
//! Each scenario runs this same binary again as a victim, which starts a fake
//! helper the way the suite does and is then killed with SIGKILL. The victim
//! holds an inheritable descriptor, the write end of a FIFO, so everything it
//! starts inherits it. The scenario then waits for two facts: the FIFO reaches
//! its end (nobody holds what was inherited) and the helper's process group
//! empties (no process is left). No macOS, capture or provider.
use anyhow::{Context, Result, anyhow, bail};
use std::{
    env,
    fs::{File, OpenOptions},
    io::{BufRead, BufReader, Read, Write},
    os::{fd::AsFd, unix::process::CommandExt},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

#[path = "support/helper_relay.rs"]
mod helper_relay;

use helper_relay::{
    CONNECT_BUDGET, Relay, hotkey_helper, kill_group, std_daemon, stubborn_helper, wait_group_empty,
};

const SCENARIO: &str = "ORBIT_TEST_KILLED_SCENARIO";
const DIRECTORY: &str = "ORBIT_TEST_KILLED_DIRECTORY";
const CHANNEL: &str = "channel.fifo";

/// How long the victim may take to have its helper running. It starts a
/// script or a daemon; this bounds a hang, it decides nothing.
const START_BUDGET: Duration = Duration::from_secs(60);

/// How long what the victim started may take to let go of the descriptor after
/// the kill. The slowest honest path is the daemon's: it notices its parent
/// in half a second and closes with a grace of three for its helper. This is
/// four times that; a helper that outlives its test holds the descriptor for
/// as long as it lives, so waiting longer would change nothing.
const RELEASE_BOUND: Duration = Duration::from_secs(14);

/// The victim outlives its scenario only if the scenario itself died first;
/// this keeps even that from leaving it running.
const VICTIM_LIFETIME: Duration = Duration::from_secs(120);

#[test]
fn a_relay_started_by_the_test_dies_with_it() -> Result<()> {
    leaves_nothing_behind("a_relay_started_by_the_test_dies_with_it")
}

#[test]
fn a_relay_started_by_the_daemon_dies_with_the_test() -> Result<()> {
    leaves_nothing_behind("a_relay_started_by_the_daemon_dies_with_the_test")
}

#[test]
fn a_stubborn_helper_dies_with_its_test() -> Result<()> {
    leaves_nothing_behind("a_stubborn_helper_dies_with_its_test")
}

#[test]
fn a_helper_waiting_on_its_trigger_dies_with_its_test() -> Result<()> {
    leaves_nothing_behind("a_helper_waiting_on_its_trigger_dies_with_its_test")
}

/// The body of the process a scenario kills: the same test, run again with
/// [`SCENARIO`] set.
fn victim(scenario: &str) -> Result<()> {
    let directory = PathBuf::from(env::var_os(DIRECTORY).context("the victim's directory")?);
    let mut channel = OpenOptions::new()
        .write(true)
        .open(directory.join(CHANNEL))
        .context("the victim could not open its channel")?;
    // From here on, everything this process starts inherits the channel, as
    // everything below a lock's holder inherits the lock.
    rustix::io::fcntl_setfd(channel.as_fd(), rustix::io::FdFlags::empty())?;
    let group = start(scenario, &directory)?;
    writeln!(channel, "{group}")?;
    channel.flush()?;
    thread::sleep(VICTIM_LIFETIME);
    Ok(())
}

fn start(scenario: &str, directory: &Path) -> Result<u32> {
    match scenario {
        "a_relay_started_by_the_test_dies_with_it" => {
            let relay = Relay::create(directory, "helper")?;
            let group = keep(helper_child(&relay.script)?);
            until(|| !relay.launches().is_empty(), "the relay to start")?;
            Ok(group)
        }
        "a_relay_started_by_the_daemon_dies_with_the_test" => {
            let relay = Relay::create(directory, "helper")?;
            let daemon = std_daemon(directory, &relay.script)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()?;
            let group = keep(daemon);
            until(
                || !relay.launches().is_empty(),
                "the daemon to launch its helper",
            )?;
            Ok(group)
        }
        "a_stubborn_helper_dies_with_its_test" => {
            let script = stubborn_helper(directory, "stubborn")?;
            let pid = directory.join("stubborn.pid");
            let group = keep(helper_child(&script)?);
            until(|| pid.exists(), "the stubborn helper to start")?;
            Ok(group)
        }
        "a_helper_waiting_on_its_trigger_dies_with_its_test" => {
            let trigger = directory.join("hotkey.fifo");
            make_fifo(&trigger)?;
            let script = hotkey_helper(directory, &trigger)?;
            Ok(keep(helper_child(&script)?))
        }
        other => bail!("no scenario is called {other}"),
    }
}

/// The helper as a child of the test, in a group of its own so the scenario
/// can tell that nothing of it is left.
fn helper_child(script: &Path) -> Result<Child> {
    Ok(Command::new(script)
        .args(["--stdio", "--managed"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()?)
}

/// Keeps the child and its pipes open until the victim is killed.
fn keep(child: Child) -> u32 {
    let child = Box::leak(Box::new(child));
    child.id()
}

fn until(ready: impl Fn() -> bool, what: &str) -> Result<()> {
    let deadline = Instant::now() + CONNECT_BUDGET;
    while !ready() {
        if Instant::now() >= deadline {
            bail!("timed out waiting for {what}");
        }
        thread::sleep(Duration::from_millis(10));
    }
    Ok(())
}

fn leaves_nothing_behind(scenario: &str) -> Result<()> {
    if env::var_os(SCENARIO).is_some() {
        return victim(scenario);
    }
    let directory = tempfile::Builder::new()
        .prefix("orbit-killed-")
        .tempdir_in("/tmp")?;
    make_fifo(&directory.path().join(CHANNEL))?;
    let log_path = directory.path().join("victim.log");
    let log = File::create(&log_path)?;
    let (sender, receiver) = mpsc::channel();
    let channel = directory.path().join(CHANNEL);
    // Opening the FIFO and reading it to its end both wait for other
    // processes, so they run apart and this test waits with a bound.
    thread::spawn(move || {
        let reader = match File::open(&channel) {
            Ok(file) => file,
            Err(error) => {
                let _ = sender.send(Err(error));
                return;
            }
        };
        let mut reader = BufReader::new(reader);
        let mut group = String::new();
        let line = reader.read_line(&mut group).map(|_| group);
        let _ = sender.send(line);
        let mut rest = Vec::new();
        let _ = reader.read_to_end(&mut rest);
        let _ = sender.send(Ok(String::new()));
    });
    let mut victim = Command::new(env::current_exe()?)
        .args(["--exact", scenario, "--nocapture"])
        .env(SCENARIO, "1")
        .env(DIRECTORY, directory.path())
        .stdin(Stdio::null())
        .stdout(Stdio::from(log.try_clone()?))
        .stderr(Stdio::from(log))
        .spawn()?;
    let group = match receiver.recv_timeout(START_BUDGET) {
        Ok(Ok(line)) if !line.trim().is_empty() => {
            line.trim().parse::<u32>().context("the victim's group")?
        }
        other => {
            let _ = victim.kill();
            let _ = victim.wait();
            let log = std::fs::read_to_string(&log_path).unwrap_or_default();
            bail!("the victim never started {scenario}: {other:?}\n{log}");
        }
    };
    victim.kill()?;
    victim.wait()?;
    match receiver.recv_timeout(RELEASE_BOUND) {
        Ok(Ok(rest)) if rest.is_empty() => {}
        other => {
            return Err(left_behind(
                group,
                &format!(
                    "{scenario}: the descriptor it inherited is still held \
                     {RELEASE_BOUND:?} after its test was killed ({other:?})"
                ),
            ));
        }
    }
    wait_group_empty(group, scenario).map_err(|problem| left_behind(group, &problem.to_string()))
}

/// Names what survived, then ends it: a failing run must not itself leave a
/// helper behind.
fn left_behind(group: u32, what: &str) -> anyhow::Error {
    let listing = Command::new("/bin/ps")
        .args(["-A", "-o", "pid=,pgid=,command="])
        .output()
        .map(|output| String::from_utf8_lossy(&output.stdout).into_owned())
        .unwrap_or_default();
    let survivors: Vec<&str> = listing
        .lines()
        .filter(|line| line.split_whitespace().nth(1) == Some(&group.to_string()))
        .collect();
    kill_group(group);
    anyhow!(
        "{what}\nstill running in group {group}:\n{}",
        survivors.join("\n")
    )
}

fn make_fifo(path: &Path) -> Result<()> {
    let status = Command::new("mkfifo")
        .args(["-m", "600"])
        .arg(path)
        .status()
        .context("could not run mkfifo")?;
    anyhow::ensure!(
        status.success(),
        "mkfifo {} ended with {status}",
        path.display()
    );
    Ok(())
}
