//! Two faults of the test processes themselves.
//!
//! A fake helper can find nobody reading its reply to `shutdown`: the daemon
//! tells the helper to leave, closes its input and waits only for the process,
//! so the relay's `cat` may be gone before the reply is whole. Nobody waits for
//! that reply, so the pipe being gone is no failure there. It is still one on
//! every other reply, and so is any other error on this one.
//!
//! The race is about one in two hundred, so nothing here waits for it: a writer
//! breaks at each write of a reply in turn, and a FIFO with no reader gives the
//! kernel's own error.
//!
//! A test killed where no `Drop` runs (SIGKILL, a runner's timeout, the machine
//! running out of memory) must not leave a real daemon behind either: it would
//! hold every descriptor it inherited, such as a build lock or the gate's
//! output. The victim scenario starts one the way these tests do and is killed.
//! It holds an inheritable descriptor, the write end of a FIFO, so the daemon
//! inherits it; the scenario then waits for that FIFO to reach its end and for
//! the process to be gone. No macOS, capture or provider.
use anyhow::{Context, Result, anyhow, bail};
use serde_json::{Value, json};
use std::{
    env,
    fs::{self, File, OpenOptions},
    io::{self, BufRead, BufReader, ErrorKind, Read, Write},
    os::{fd::AsFd, unix::fs::OpenOptionsExt},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

#[path = "support/harness.rs"]
mod harness;
#[path = "support/ipc.rs"]
mod ipc;

use harness::{answer, real_daemon};

const SCENARIO: &str = "ORBIT_TEST_KILLED_DAEMON_SCENARIO";
const DIRECTORY: &str = "ORBIT_TEST_KILLED_DAEMON_DIRECTORY";
const CHANNEL: &str = "channel.fifo";
const FIFO_SCENARIO: &str = "ORBIT_TEST_FIFO_SCENARIO";

/// How long the victim may take to have its daemon answering: what the suite
/// gives a cold daemon under full load. It bounds a hang, it decides nothing.
const START_BUDGET: Duration = Duration::from_secs(60);

/// How long the daemon may take to let go of the descriptor after the kill. It
/// notices its parent in half a second and closes in order, with no helper to
/// wait for; 14 s is the bound the fake helpers' own scenarios use. A daemon
/// that does not follow its test holds the descriptor for as long as it lives,
/// so waiting longer would change nothing.
const RELEASE_BOUND: Duration = Duration::from_secs(14);

/// How long a daemon that let go of the descriptor may stay visible: a zombie
/// waiting for init, which reaps it in milliseconds.
const GONE_BOUND: Duration = Duration::from_secs(10);

/// The victim outlives its scenario only if the scenario itself died first;
/// this keeps even that from leaving it running.
const VICTIM_LIFETIME: Duration = Duration::from_secs(120);

/// No reply has this many writes; it bounds the walk over them.
const MOST_WRITES: usize = 500;

const SHUTDOWN: &str = "shutdown";
const OTHER_COMMANDS: [&str; 4] = ["hello", "status", "capture_stop", "playback_stop"];

/// A writer that fails once, at one write of the reply, the way a pipe does
/// when its reader leaves between two `write`s. `flush` counts as a write.
struct BreaksAt {
    write: usize,
    kind: ErrorKind,
    seen: usize,
    broke: bool,
}

impl BreaksAt {
    fn new(write: usize, kind: ErrorKind) -> Self {
        Self {
            write,
            kind,
            seen: 0,
            broke: false,
        }
    }

    fn step(&mut self) -> io::Result<()> {
        self.seen += 1;
        if self.seen == self.write {
            self.broke = true;
            return Err(io::Error::from(self.kind));
        }
        Ok(())
    }
}

impl Write for BreaksAt {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.step()?;
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.step()
    }
}

fn reply() -> Value {
    json!({"v":1,"event":"response","id":"7","ok":true,"result":{"drain_complete":true}})
}

/// `check` for a writer breaking at the first write of the reply, then the
/// second, and so on until a reply goes through whole.
fn at_every_write(kind: ErrorKind, command: &str, check: impl Fn(usize, Result<()>)) {
    for write in 1..MOST_WRITES {
        let mut writer = BreaksAt::new(write, kind);
        let outcome = answer(&mut writer, command, &reply());
        if !writer.broke {
            assert!(write > 1, "the reply to {command} wrote nothing");
            assert!(
                outcome.is_ok(),
                "a reply to {command} that nothing broke failed: {outcome:?}"
            );
            return;
        }
        check(write, outcome);
    }
    panic!("the reply to {command} never finished in {MOST_WRITES} writes");
}

#[test]
fn nobody_reading_the_reply_to_shutdown_is_not_a_failure_at_any_write() {
    at_every_write(ErrorKind::BrokenPipe, SHUTDOWN, |write, outcome| {
        assert!(
            outcome.is_ok(),
            "write {write} of the reply to shutdown found no reader and failed: {outcome:?}"
        );
    });
}

#[test]
fn nobody_reading_any_other_reply_is_still_a_failure_at_every_write() {
    for command in OTHER_COMMANDS {
        at_every_write(ErrorKind::BrokenPipe, command, |write, outcome| {
            assert!(
                outcome.is_err(),
                "write {write} of the reply to {command} found no reader and passed"
            );
        });
    }
}

#[test]
fn any_other_error_on_the_reply_to_shutdown_is_still_a_failure_at_every_write() {
    for kind in [
        ErrorKind::PermissionDenied,
        ErrorKind::ConnectionReset,
        ErrorKind::WriteZero,
        ErrorKind::Other,
    ] {
        at_every_write(kind, SHUTDOWN, |write, outcome| {
            assert!(
                outcome.is_err(),
                "write {write} of the reply to shutdown failed with {kind:?} and passed"
            );
        });
    }
}

#[test]
fn a_fifo_with_no_reader_only_excuses_the_reply_to_shutdown() -> Result<()> {
    if env::var_os(FIFO_SCENARIO).is_none() {
        return in_a_process_of_its_own("a_fifo_with_no_reader_only_excuses_the_reply_to_shutdown");
    }
    let directory = tempfile::Builder::new()
        .prefix("orbit-reply-")
        .tempdir_in("/tmp")?;
    let mut writer = fifo_without_a_reader(directory.path())?;
    let shutdown = answer(&mut writer, SHUTDOWN, &reply());
    assert!(
        shutdown.is_ok(),
        "the reply to shutdown found no reader and failed: {shutdown:?}"
    );
    for command in OTHER_COMMANDS {
        let error = answer(&mut writer, command, &reply())
            .err()
            .with_context(|| format!("the reply to {command} found no reader and passed"))?;
        assert!(
            format!("{error:#}").contains("Broken pipe"),
            "the reply to {command} did not name the pipe: {error:#}"
        );
    }
    Ok(())
}

#[test]
fn a_descriptor_that_cannot_write_is_a_failure_on_the_reply_to_shutdown() -> Result<()> {
    let directory = tempfile::Builder::new()
        .prefix("orbit-reply-")
        .tempdir_in("/tmp")?;
    let path = directory.path().join("read-only");
    fs::write(&path, "")?;
    let mut reader = File::open(&path)?;
    assert!(
        answer(&mut reader, SHUTDOWN, &reply()).is_err(),
        "writing the reply to shutdown to a read-only descriptor passed"
    );
    Ok(())
}

/// Runs a test again in a process where nothing else starts processes. A `fork`
/// in another test holds a copy of every descriptor open at that moment until
/// it `exec`s, close-on-exec or not, so the kernel could still count the FIFO's
/// reader after this test closed it, and the write would pass.
fn in_a_process_of_its_own(test: &str) -> Result<()> {
    let run = Command::new(env::current_exe()?)
        .args(["--exact", test])
        .env(FIFO_SCENARIO, "1")
        .stdin(Stdio::null())
        .output()
        .context("could not run the test in its own process")?;
    let stdout = String::from_utf8_lossy(&run.stdout);
    // A name that matches no test also ends well, having run nothing.
    anyhow::ensure!(
        run.status.success() && stdout.contains("1 passed"),
        "the test in its own process ended with {}:\n{stdout}\n{}",
        run.status,
        String::from_utf8_lossy(&run.stderr)
    );
    Ok(())
}

/// The write end of a FIFO whose reader has left, as the relay's `cat` leaves.
fn fifo_without_a_reader(directory: &Path) -> Result<File> {
    let path = directory.join("reply.fifo");
    make_fifo(&path)?;
    // Not waiting for a writer: opening the other end needs a reader to exist.
    let reader = OpenOptions::new()
        .read(true)
        .custom_flags(rustix::fs::OFlags::NONBLOCK.bits() as i32)
        .open(&path)
        .context("could not open the FIFO to read")?;
    let writer = OpenOptions::new()
        .write(true)
        .open(&path)
        .context("could not open the FIFO to write")?;
    drop(reader);
    Ok(writer)
}

#[test]
fn a_real_daemon_started_the_way_these_tests_do_dies_with_its_test() -> Result<()> {
    let scenario = "a_real_daemon_started_the_way_these_tests_do_dies_with_its_test";
    if env::var_os(SCENARIO).is_some() {
        return victim();
    }
    let directory = tempfile::Builder::new()
        .prefix("orbit-killed-daemon-")
        .tempdir_in("/tmp")?;
    let channel = directory.path().join(CHANNEL);
    make_fifo(&channel)?;
    let log_path = directory.path().join("victim.log");
    let log = File::create(&log_path)?;
    let (sender, receiver) = mpsc::channel();
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
        let mut pid = String::new();
        let line = reader.read_line(&mut pid).map(|_| pid);
        let _ = sender.send(line);
        let mut rest = Vec::new();
        let _ = reader.read_to_end(&mut rest);
        let _ = sender.send(Ok(String::new()));
    });
    let mut test = Command::new(env::current_exe()?)
        .args(["--exact", scenario, "--nocapture"])
        .env(SCENARIO, "1")
        .env(DIRECTORY, directory.path())
        .stdin(Stdio::null())
        .stdout(Stdio::from(log.try_clone()?))
        .stderr(Stdio::from(log))
        .spawn()?;
    let pid = match receiver.recv_timeout(START_BUDGET) {
        Ok(Ok(line)) if !line.trim().is_empty() => {
            line.trim().parse::<i32>().context("the daemon's pid")?
        }
        other => {
            let _ = test.kill();
            let _ = test.wait();
            let log = fs::read_to_string(&log_path).unwrap_or_default();
            bail!("the victim never started its daemon: {other:?}\n{log}");
        }
    };
    test.kill()?;
    test.wait()?;
    match receiver.recv_timeout(RELEASE_BOUND) {
        Ok(Ok(rest)) if rest.is_empty() => {}
        other => {
            return Err(left_behind(
                pid,
                &format!(
                    "the descriptor the daemon inherited is still held \
                     {RELEASE_BOUND:?} after its test was killed ({other:?})"
                ),
            ));
        }
    }
    let deadline = Instant::now() + GONE_BOUND;
    while exists(pid) {
        if Instant::now() >= deadline {
            return Err(left_behind(
                pid,
                &format!("the daemon is still there {GONE_BOUND:?} after letting go"),
            ));
        }
        thread::sleep(Duration::from_millis(10));
    }
    Ok(())
}

/// The body of the process the scenario kills.
fn victim() -> Result<()> {
    let directory = PathBuf::from(env::var_os(DIRECTORY).context("the victim's directory")?);
    let mut channel = OpenOptions::new()
        .write(true)
        .open(directory.join(CHANNEL))
        .context("the victim could not open its channel")?;
    // From here on, everything this process starts inherits the channel.
    rustix::io::fcntl_setfd(channel.as_fd(), rustix::io::FdFlags::empty())?;
    let data = directory.join("data");
    fs::create_dir_all(&data)?;
    let daemon = real_daemon(&data)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("could not start the daemon")?;
    let pid = daemon.id();
    // Kept until the kill: no `Drop` is to run on this path.
    std::mem::forget(daemon);
    let deadline = Instant::now() + START_BUDGET;
    while ipc::status(&data).is_err() {
        if Instant::now() >= deadline {
            bail!("the daemon {pid} never answered");
        }
        thread::sleep(Duration::from_millis(20));
    }
    writeln!(channel, "{pid}")?;
    channel.flush()?;
    thread::sleep(VICTIM_LIFETIME);
    Ok(())
}

/// Whether a process with this id still exists, zombies included.
fn exists(pid: i32) -> bool {
    rustix::process::Pid::from_raw(pid).is_some_and(|pid| {
        !matches!(
            rustix::process::test_kill_process(pid),
            Err(rustix::io::Errno::SRCH)
        )
    })
}

/// Names what survived, then ends it: a failing run must not itself leave a
/// daemon behind.
fn left_behind(pid: i32, what: &str) -> anyhow::Error {
    let listing = Command::new("/bin/ps")
        .args(["-p", &pid.to_string(), "-o", "pid=,command="])
        .output()
        .map(|output| String::from_utf8_lossy(&output.stdout).into_owned())
        .unwrap_or_default();
    if let Some(process) = rustix::process::Pid::from_raw(pid) {
        let _ = rustix::process::kill_process(process, rustix::process::Signal::KILL);
    }
    anyhow!("{what}\nstill running:\n{}", listing.trim_end())
}

fn make_fifo(path: &Path) -> Result<()> {
    // `mkfifo(1)` rather than a syscall: rustix does not offer mkfifo on Apple
    // targets, and this suite must build on the Mac.
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
