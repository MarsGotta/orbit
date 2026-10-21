//! Real process/IPC with the production helper launch (`--helper`) and a relay
//! standing where `OrbitNative` stands. No capture, providers or macOS
//! permission claims. These two cases used to drive the daemon through the
//! retired mode in which the helper hosted it; they now drive the path
//! production runs.
use anyhow::{Context, Result, bail};
use orbit_core::protocol::Command as IpcCommand;
use serde_json::{Value, json};
use std::{
    fs::File,
    io::{BufRead, BufReader, Write},
    path::Path,
    process::{Child, Stdio},
    sync::{Arc, Mutex},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

#[path = "support/helper_relay.rs"]
mod helper_relay;
use helper_relay::{Relay, kill_group, std_daemon};
#[path = "support/ipc.rs"]
mod ipc;

struct Peer {
    child: Child,
    group: u32,
    /// The helper's output towards the daemon, shared so a test can hold it
    /// open past the daemon's exit or close it to end the helper's output.
    events: Arc<Mutex<Option<File>>>,
    reader: Option<JoinHandle<Result<Vec<String>>>>,
}

impl Peer {
    fn start(path: &Path) -> Result<Self> {
        let relay = Relay::create(path, "helper")?;
        let child = std_daemon(path, &relay.script)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()?;
        let group = child.id();
        let (commands, events) = relay.connect()?;
        let events = Arc::new(Mutex::new(Some(events)));
        let peer_events = events.clone();
        let reader = thread::spawn(move || -> Result<Vec<String>> {
            send(&peer_events, helper_relay::ready_event()?)?;
            let mut effects = Vec::new();
            for line in BufReader::new(commands).lines() {
                let command: Value = serde_json::from_str(&line?)?;
                let name = command["command"]
                    .as_str()
                    .context("native command missing")?;
                effects.push(name.to_string());
                let result = match name {
                    "hello" | "heartbeat" | "status" => helper_relay::state_result(json!({
                        "microphone":"not_determined","screen":false,"accessibility":false,
                        "calendars":"not_determined"}))?,
                    "privacy_stop" => {
                        let mut result = helper_relay::state_result(json!({
                            "microphone":"not_determined","screen":false,"accessibility":false,
                            "calendars":"not_determined"}))?;
                        result["privacy_stopped"] = json!(true);
                        result["drain_complete"] = json!(true);
                        result
                    }
                    // Closing cuts without the latch.
                    "capture_stop" => {
                        let mut result = helper_relay::state_result(json!({
                            "microphone":"not_determined","screen":false,"accessibility":false,
                            "calendars":"not_determined"}))?;
                        result["drain_complete"] = json!(true);
                        result
                    }
                    "playback_stop" => json!({}),
                    "shutdown" => json!({"drain_complete":true}),
                    _ => bail!("No capture or other native action is authorized in this fixture"),
                };
                send(
                    &peer_events,
                    json!({"v":1,"event":"response","id":command["id"],
                    "ok":true,"result":result}),
                )?;
            }
            Ok(effects)
        });
        let mut peer = Self {
            child,
            group,
            events,
            reader: Some(reader),
        };
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            assert!(
                peer.child.try_wait()?.is_none(),
                "daemon exited before readiness"
            );
            if ipc::status(path).is_ok_and(|state| state["capabilities"]["native_helper"] == true) {
                return Ok(peer);
            }
            assert!(
                Instant::now() < deadline,
                "synthetic native readiness timed out"
            );
            thread::sleep(Duration::from_millis(20));
        }
    }

    fn wait_for_exit(&mut self) -> Result<()> {
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            if let Some(status) = self.child.try_wait()? {
                assert!(status.success(), "daemon failed during graceful shutdown");
                return Ok(());
            }
            assert!(
                Instant::now() < deadline,
                "daemon did not exit with the helper's output still open"
            );
            thread::sleep(Duration::from_millis(20));
        }
    }

    fn close_events(&self) -> Result<()> {
        self.events
            .lock()
            .map_err(|_| anyhow::anyhow!("peer lock poisoned"))?
            .take();
        Ok(())
    }

    fn finish_reader(&mut self) -> Result<Vec<String>> {
        self.reader
            .take()
            .context("peer already joined")?
            .join()
            .map_err(|_| anyhow::anyhow!("native peer panicked"))?
    }
}

impl Drop for Peer {
    fn drop(&mut self) {
        // This cleanup is reached only after assertions; never use EOF/kill to
        // pass them. The group is this test's daemon and its helpers only.
        kill_group(self.group);
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = self.close_events();
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

fn send(events: &Mutex<Option<File>>, value: Value) -> Result<()> {
    let mut guard = events
        .lock()
        .map_err(|_| anyhow::anyhow!("peer lock poisoned"))?;
    if let Some(writer) = guard.as_mut() {
        serde_json::to_writer(&mut *writer, &value)?;
        writer.write_all(b"\n")?;
        writer.flush()?;
    }
    Ok(())
}

#[test]
fn graceful_shutdown_finishes_while_the_helper_keeps_its_output_open() -> Result<()> {
    let directory = tempfile::Builder::new()
        .prefix("orbit-pipes-")
        .tempdir_in("/tmp")?;
    let mut peer = Peer::start(directory.path())?;
    ipc::request(directory.path(), IpcCommand::Shutdown)?;
    // The helper still holds its output towards the daemon open: an end the
    // helper keeps cannot hold the daemon's exit.
    peer.wait_for_exit()?;
    assert!(
        peer.events
            .lock()
            .map_err(|_| anyhow::anyhow!("peer lock poisoned"))?
            .is_some()
    );
    assert!(!directory.path().join("orbit.sock").exists());
    let effects = peer.finish_reader()?;
    assert_eq!(effects.first().map(String::as_str), Some("hello"));
    assert!(effects.iter().all(|name| matches!(
        name.as_str(),
        "hello" | "heartbeat" | "status" | "capture_stop" | "playback_stop" | "shutdown"
    )));
    Ok(())
}

#[test]
fn helper_eof_is_a_disconnect_not_authority_to_skip_shutdown() -> Result<()> {
    let directory = tempfile::Builder::new()
        .prefix("orbit-eof-")
        .tempdir_in("/tmp")?;
    let mut peer = Peer::start(directory.path())?;
    peer.close_events()?;
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        assert!(
            peer.child.try_wait()?.is_none(),
            "EOF must not bypass actor shutdown"
        );
        let state = ipc::status(directory.path())?;
        // The lost helper is said and latches nothing.
        if state["privacy_stopped"] == false
            && state["capabilities"]["native_helper"] == false
            // Said as a code with its cause, not a sentence.
            && state["notice"]["code"] == "contained_failure"
            && state["notice"]["data"]["cause"] == "helper_lost"
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "native EOF was not taken as a lost helper"
        );
        thread::sleep(Duration::from_millis(20));
    }
    ipc::request(directory.path(), IpcCommand::Shutdown)?;
    peer.wait_for_exit()?;
    peer.finish_reader()?;
    Ok(())
}
