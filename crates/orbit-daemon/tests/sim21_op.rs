//! SIM-21 acceptance 4: the op of an order travels end to end. The real
//! daemon, spoken to through its socket as the app speaks it, pairs against a
//! test hub that refuses: the request to the hub carries `X-Orbit-Op` with the
//! order's op, the notice the refusal causes carries the same op, and the
//! daemon's local report log has its line under that op.
use anyhow::{Context, Result};
use orbit_core::notice::Op;
use orbit_core::protocol::Command as IpcCommand;
use serde_json::Value;
use std::{
    io::{Read, Write},
    net::TcpListener,
    path::Path,
    process::{Child, Command},
    sync::mpsc,
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

/// A hub that answers 403 to every request and hands each request's head to
/// the test.
fn refusing_hub() -> Result<(String, mpsc::Receiver<String>)> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let base = format!("http://{}/", listener.local_addr()?);
    let (heads, received) = mpsc::channel();
    thread::spawn(move || {
        for connection in listener.incoming() {
            let Ok(mut socket) = connection else {
                return;
            };
            let mut bytes = Vec::new();
            let mut chunk = [0_u8; 1024];
            while !bytes.windows(4).any(|window| window == b"\r\n\r\n") {
                match socket.read(&mut chunk) {
                    Ok(0) | Err(_) => break,
                    Ok(count) => bytes.extend_from_slice(&chunk[..count]),
                }
            }
            if bytes.is_empty() {
                continue;
            }
            if heads
                .send(String::from_utf8_lossy(&bytes).into_owned())
                .is_err()
            {
                return;
            }
            let body = br#"{"error":"bad pairing code"}"#;
            let _ = write!(
                socket,
                "HTTP/1.1 403 Forbidden\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                body.len()
            );
            let _ = socket.write_all(body);
        }
    });
    Ok((base, received))
}

fn wait_for_status(directory: &Path, child: &mut Child) -> Result<Value> {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        if let Some(status) = child.try_wait()? {
            anyhow::bail!("daemon exited before answering (status: {status})");
        }
        // Ready means the store recovered, not just a socket that answers:
        // an order sent during recovery is refused before it reaches the hub
        // (3 runs in 5 on the Mac, «El almacén se está recuperando»).
        if let Ok(state) = ipc::status(directory)
            && state["recovery"]["state"] == "ready"
        {
            return Ok(state);
        }
        anyhow::ensure!(Instant::now() < deadline, "the daemon never answered");
        thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn an_orders_op_reaches_the_hub_header_its_notice_and_the_report_log() -> Result<()> {
    let directory = tempfile::Builder::new()
        .prefix("orbit-sim21-")
        .tempdir_in("/tmp")?;
    let mut daemon = Daemon(
        Command::new(env!("CARGO_BIN_EXE_orbit"))
            .args(["--dev", "--data-dir"])
            .arg(directory.path())
            .arg("daemon")
            .spawn()?,
    );
    wait_for_status(directory.path(), &mut daemon.0)?;
    let (hub, heads) = refusing_hub()?;
    let op = Op::parse("7f3a2c9e-sim21-acceptance").context("op")?;
    let paths = orbit_ipc::Paths::resolve(Some(directory.path().to_owned()))?;
    let refused = thread::scope(|scope| {
        scope
            .spawn(|| {
                tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()?
                    .block_on(orbit_ipc::request_command_as(
                        &paths,
                        IpcCommand::Pair {
                            hub_url: hub.clone(),
                            code: "codigo-de-prueba-sim21".into(),
                            device_name: "Mac de prueba".into(),
                        },
                        op.clone(),
                    ))
            })
            .join()
            .map_err(|_| anyhow::anyhow!("the IPC thread failed"))?
    });
    assert!(refused.is_err(), "the test hub refuses the pairing");

    // The pairing's request, among whatever else reached the hub first.
    let head = loop {
        let head = heads.recv_timeout(Duration::from_secs(30))?;
        if head.starts_with("POST /v1/pair ") {
            break head;
        }
    };
    assert!(
        head.to_ascii_lowercase()
            .contains(&format!("x-orbit-op: {}", op.as_str().to_ascii_lowercase())),
        "the request to the hub carries the order's op: {head}"
    );

    let state = ipc::status(directory.path())?;
    let notice = &state["notice"];
    assert_eq!(notice["code"], "pairing_failed", "{state}");
    assert_eq!(notice["op"], op.as_str(), "{notice}");

    let report = std::fs::read_to_string(directory.path().join("logs/report-daemon.log"))?;
    let line: Value = report
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .find(|line| line["code"] == "pairing_failed")
        .context("the refusal has its report line")?;
    assert_eq!(line["op"], op.as_str(), "{line}");
    assert_eq!(line["class"], "fixable", "{line}");
    assert_eq!(line["source"], "daemon", "{line}");
    // The cause sentence is in daemon.log under the same op, never in the
    // report line.
    let diagnostics = std::fs::read_to_string(directory.path().join("logs/daemon.log"))?;
    assert!(
        diagnostics
            .lines()
            .any(|line| line.contains("notice_cause") && line.contains(op.as_str())),
        "{diagnostics}"
    );
    assert!(!report.contains("403"), "{report}");

    ipc::request(directory.path(), IpcCommand::Shutdown)?;
    let deadline = Instant::now() + Duration::from_secs(60);
    while daemon.0.try_wait()?.is_none() {
        anyhow::ensure!(Instant::now() < deadline, "the daemon did not exit");
        thread::sleep(Duration::from_millis(20));
    }
    Ok(())
}
