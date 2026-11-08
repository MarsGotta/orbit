//! What this suite's tests do the same way: how they start a real daemon that
//! has no helper, and how a fake helper writes one reply.
#![allow(dead_code)]

use anyhow::Result;
use serde_json::Value;
use std::{
    ffi::OsStr,
    io::{self, ErrorKind, Write},
    path::Path,
    process::Command,
};

/// The one command whose reply nobody waits for.
const SHUTDOWN: &str = "shutdown";

/// `orbit --dev --data-dir <path> daemon --parent-pid <this process>`, ready to
/// take `--helper` and the rest of the daemon's arguments, which go after
/// `daemon`.
///
/// `--parent-pid` is what makes the daemon follow a test killed where no `Drop`
/// runs; without it, the daemon keeps every descriptor it inherited.
pub fn real_daemon(data_dir: &Path) -> Command {
    real_daemon_at(env!("CARGO_BIN_EXE_orbit"), data_dir)
}

/// The same from another path to the binary, such as a symlink to it.
pub fn real_daemon_at(executable: impl AsRef<OsStr>, data_dir: &Path) -> Command {
    let mut command = Command::new(executable);
    command
        .args(["--dev", "--data-dir"])
        .arg(data_dir)
        .arg("daemon")
        .arg("--parent-pid")
        .arg(std::process::id().to_string());
    command
}

/// One reply of a fake helper to its command: the JSON and its line end.
///
/// The daemon tells the helper to shut down, closes its input and waits only
/// for the process, so the pipe can be gone before the reply is whole. That is
/// no failure on the reply to `shutdown`, and it still is on every other reply;
/// any other error fails on all of them.
pub fn answer<W: Write>(writer: &mut W, command: &str, reply: &Value) -> Result<()> {
    match write_reply(writer, reply) {
        Err(error) if command == SHUTDOWN && error.kind() == ErrorKind::BrokenPipe => Ok(()),
        outcome => Ok(outcome?),
    }
}

/// Keeps the `io::Error`, so its kind can be read: `serde_json` hands back the
/// writer's own error through this conversion.
fn write_reply<W: Write>(writer: &mut W, reply: &Value) -> io::Result<()> {
    serde_json::to_writer(&mut *writer, reply)?;
    writer.write_all(b"\n")?;
    writer.flush()
}
