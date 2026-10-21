//! The daemon owns the helper as a managed child. Real daemon subprocess
//! with a fake helper script; no capture, providers or macOS permission claims.
use anyhow::{Context, Result};
use orbit_core::protocol::Command as IpcCommand;
use serde_json::Value;
use std::{
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

#[path = "support/ipc.rs"]
mod ipc;

struct Daemon(Child);
impl Drop for Daemon {
    fn drop(&mut self) {
        // Cleanup is reached only after assertions; never kill to pass them.
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn daemon(path: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_orbit"));
    command.args(["--dev", "--data-dir"]).arg(path);
    command
}

/// A fake helper that records the arguments the daemon launched it with,
/// speaks the minimal `ready` frame and dies, so the disconnect latch, the
/// supervision relaunches and the launch arguments are all observable.
///
/// Written by `cp`, never held open for writing by this process: the suite
/// forks constantly (every daemon is a process), a fork inherits an
/// open descriptor until its `exec`, and executing a file someone holds open
/// for writing fails with `ETXTBSY` — which the daemon reads, correctly, as a
/// helper that fell.
fn fake_helper(directory: &Path, name: &str) -> Result<PathBuf> {
    let script = directory.join(name);
    let source = directory.join(format!("{name}.txt"));
    let args = directory.join(format!("{name}.args"));
    let launches = directory.join(format!("{name}.launches"));
    let mut ready: Value = serde_json::from_str(include_str!(
        "../../../contracts/fixtures/native/events/ready.json"
    ))?;
    ready["capabilities"] = serde_json::json!([]);
    ready["selected_text_available"] = serde_json::json!(false);
    let ready = serde_json::to_string(&ready)?;
    anyhow::ensure!(!ready.contains('\''), "ready fixture is not shell quotable");
    std::fs::write(
        &source,
        format!(
            "#!/bin/sh\n/bin/echo \"$@\" > '{}'\n/bin/echo '{ready}'\n\
             /bin/echo launch >> '{}'\nexit 1\n",
            args.display(),
            launches.display()
        ),
    )?;
    let copied = Command::new("/bin/cp").arg(&source).arg(&script).status()?;
    anyhow::ensure!(copied.success(), "cp of the fake helper failed: {copied}");
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755))?;
    }
    Ok(script)
}

fn launches_of(directory: &Path, name: &str) -> usize {
    std::fs::read_to_string(directory.join(format!("{name}.launches")))
        .map(|text| text.lines().filter(|line| !line.trim().is_empty()).count())
        .unwrap_or(0)
}

fn wait_until(
    directory: &Path,
    child: &mut Child,
    what: &str,
    ready: fn(&Value) -> bool,
) -> Result<Value> {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        if let Some(status) = child.try_wait()? {
            panic!("daemon exited before {what} (status: {status})");
        }
        if let Ok(state) = ipc::status(directory)
            && ready(&state)
        {
            return Ok(state);
        }
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        thread::sleep(Duration::from_millis(20));
    }
}

/// The fall is said and nothing latches: a lost helper is no stop.
fn fell(state: &Value) -> bool {
    state["privacy_stopped"] == false
        && state["capabilities"]["native_helper"] == false
        && state["notice_log"]["records"]
            .as_array()
            .is_some_and(|records| {
                // Said as a code with its cause, not a sentence.
                records.iter().any(|record| {
                    record["code"] == "contained_failure"
                        && record["data"]["cause"] == "helper_lost"
                })
            })
}

fn wait_for_exit(child: &mut Child) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        if let Some(status) = child.try_wait()? {
            assert!(status.success(), "daemon failed during graceful shutdown");
            return Ok(());
        }
        assert!(
            Instant::now() < deadline,
            "daemon did not exit after shutdown"
        );
        thread::sleep(Duration::from_millis(20));
    }
}

/// `orbit daemon --helper <absolute path>` launches the helper as a
/// managed child (`--stdio --managed`), announces its fall and keeps it under
/// supervision, exactly like a configured helper.
#[test]
fn daemon_helper_flag_launches_the_helper_with_stdio_managed_and_supervises_it() -> Result<()> {
    let directory = tempfile::Builder::new()
        .prefix("orbit-helper-")
        .tempdir_in("/tmp")?;
    let helper = fake_helper(directory.path(), "helper")?;
    let mut daemon = Daemon(
        daemon(directory.path())
            .arg("daemon")
            .arg("--helper")
            .arg(&helper)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()?,
    );
    wait_until(directory.path(), &mut daemon.0, "the helper fall", fell)?;
    // Ends on the second launch; the bound only stops a hang.
    let deadline = Instant::now() + Duration::from_secs(60);
    while launches_of(directory.path(), "helper") < 2 && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(50));
    }
    let observed = std::fs::read_to_string(directory.path().join("helper.args"))
        .context("the daemon must launch the helper given by the flag")?;
    assert_eq!(
        observed.trim(),
        "--stdio --managed",
        "the flagged helper must be launched in managed mode"
    );
    assert!(
        launches_of(directory.path(), "helper") >= 2,
        "the flagged helper must stay under supervision after its fall"
    );
    ipc::request(directory.path(), IpcCommand::Shutdown)
        .context("the supervised daemon must still answer a graceful shutdown")?;
    wait_for_exit(&mut daemon.0)?;
    Ok(())
}

/// A leftover helper setting, including one in a symlinked fake bundle,
/// never chooses an executable. Only the explicit helper argument can.
#[test]
fn configured_helper_never_launches_with_or_without_a_symlink() -> Result<()> {
    for symlink in [false, true] {
        let directory = tempfile::Builder::new()
            .prefix("orbit-configured-helper-")
            .tempdir_in("/tmp")?;
        let macos = directory.path().join("Fake.app/Contents/MacOS");
        std::fs::create_dir_all(&macos)?;
        let helper = fake_helper(&macos, "OrbitNative")?;
        let executable = if symlink {
            let alias = macos.join("Orbit");
            std::os::unix::fs::symlink(env!("CARGO_BIN_EXE_orbit"), &alias)?;
            alias
        } else {
            PathBuf::from(env!("CARGO_BIN_EXE_orbit"))
        };
        let data = directory.path().join("data");
        std::fs::create_dir_all(&data)?;
        std::fs::write(
            data.join("config.toml"),
            format!(
                "version = 1\ndevice_name = \"Mi Mac\"\nspeak_responses = false\nhelper_path = {:?}\n",
                helper.display()
            ),
        )?;
        let mut daemon = Daemon(
            Command::new(executable)
                .args(["--dev", "--data-dir"])
                .arg(&data)
                .arg("daemon")
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::inherit())
                .spawn()?,
        );
        wait_until(
            &data,
            &mut daemon.0,
            "daemon ready without a helper",
            |state| {
                state["capabilities"]["native_helper"] == false
                    && state["recovery"]["state"] == "ready"
            },
        )?;
        ipc::request(&data, IpcCommand::Shutdown)?;
        wait_for_exit(&mut daemon.0)?;
        assert!(!macos.join("OrbitNative.launches").exists());
        assert!(!macos.join("OrbitNative.args").exists());
    }
    Ok(())
}
