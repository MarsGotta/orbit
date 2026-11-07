//! Each Orbit has an installation id of its own: made once in its data
//! folder, the same after a restart, and said to the hub when it pairs, with
//! nothing about what kind of device it is or what it handles (a device that
//! says nothing is a Mac that handles everything). The real daemon, spoken to
//! through its socket as the app speaks it, pairs against a synthetic hub that
//! takes each pairing and refuses it; nothing here is evidence about the real
//! hub.
use anyhow::{Context, Result};
use orbit_core::protocol::Command as IpcCommand;
use serde_json::Value;
use std::{
    io::{Read, Write},
    net::TcpListener,
    path::Path,
    process::{Child, Command, Stdio},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

#[path = "support/ipc.rs"]
mod ipc;

const CODE: &str = "codigo-sintetico-3";
const NAME: &str = "Estudio Lira";

struct Daemon(Child);
impl Drop for Daemon {
    fn drop(&mut self) {
        // Cleanup is reached only after assertions; never kill to pass them.
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn data_folder() -> Result<tempfile::TempDir> {
    Ok(tempfile::Builder::new()
        .prefix("orbit-installation-")
        .tempdir_in("/tmp")?)
}

/// The daemon on `folder`, once its store recovered: an order sent during
/// recovery is refused before it reaches the hub.
fn start(folder: &Path) -> Result<Daemon> {
    let mut daemon = Daemon(
        Command::new(env!("CARGO_BIN_EXE_orbit"))
            .args(["--dev", "--data-dir"])
            .arg(folder)
            .arg("daemon")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .spawn()?,
    );
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        if let Some(status) = daemon.0.try_wait()? {
            anyhow::bail!("the daemon exited before answering (status: {status})");
        }
        if ipc::status(folder).is_ok_and(|state| state["recovery"]["state"] == "ready") {
            return Ok(daemon);
        }
        anyhow::ensure!(Instant::now() < deadline, "the daemon never answered");
        thread::sleep(Duration::from_millis(20));
    }
}

fn stop(mut daemon: Daemon, folder: &Path) -> Result<()> {
    ipc::request(folder, IpcCommand::Shutdown)?;
    let deadline = Instant::now() + Duration::from_secs(60);
    while daemon.0.try_wait()?.is_none() {
        anyhow::ensure!(Instant::now() < deadline, "the daemon did not exit");
        thread::sleep(Duration::from_millis(20));
    }
    Ok(())
}

/// A hub that refuses every pairing and hands each pairing's JSON body to
/// the test.
fn refusing_hub() -> Result<(String, mpsc::Receiver<Value>)> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let base = format!("http://{}/", listener.local_addr()?);
    let (bodies, received) = mpsc::channel();
    thread::spawn(move || {
        for connection in listener.incoming() {
            let Ok(mut socket) = connection else {
                return;
            };
            let mut bytes = Vec::new();
            let mut chunk = [0_u8; 4096];
            let head_end = loop {
                if let Some(at) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
                    break Some(at + 4);
                }
                match socket.read(&mut chunk) {
                    Ok(0) | Err(_) => break None,
                    Ok(count) => bytes.extend_from_slice(&chunk[..count]),
                }
            };
            let Some(head_end) = head_end else {
                continue;
            };
            let head = String::from_utf8_lossy(&bytes[..head_end]).to_ascii_lowercase();
            let length = head
                .lines()
                .find_map(|line| line.strip_prefix("content-length:"))
                .and_then(|value| value.trim().parse::<usize>().ok())
                .unwrap_or(0);
            while bytes.len() < head_end + length {
                match socket.read(&mut chunk) {
                    Ok(0) | Err(_) => break,
                    Ok(count) => bytes.extend_from_slice(&chunk[..count]),
                }
            }
            if head.starts_with("post /v1/pair ") {
                let body = bytes
                    .get(head_end..head_end + length)
                    .and_then(|body| serde_json::from_slice(body).ok())
                    .unwrap_or(Value::Null);
                if bodies.send(body).is_err() {
                    return;
                }
            }
            let answer = br#"{"error":"bad pairing code"}"#;
            let _ = write!(
                socket,
                "HTTP/1.1 403 Forbidden\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                answer.len()
            );
            let _ = socket.write_all(answer);
        }
    });
    Ok((base, received))
}

/// Starts the daemon on `folder`, asks it to pair with `hub` as the app
/// does, stops it, and gives the body the hub received.
fn pairing_body(folder: &Path, hub: &str, bodies: &mpsc::Receiver<Value>) -> Result<Value> {
    let daemon = start(folder)?;
    // The synthetic hub refuses the pairing; what matters is what reached it.
    let _refused = ipc::request(
        folder,
        IpcCommand::Pair {
            hub_url: hub.into(),
            code: CODE.into(),
            device_name: NAME.into(),
        },
    );
    let body = bodies
        .recv_timeout(Duration::from_secs(30))
        .context("the pairing never reached the hub")?;
    stop(daemon, folder)?;
    Ok(body)
}

fn installation_id(body: &Value) -> Result<String> {
    body["installation_id"]
        .as_str()
        .map(str::to_owned)
        .with_context(|| format!("the pairing says no installation id: {body}"))
}

/// A version 4 UUID, written as 36 lowercase hexadecimal characters with
/// hyphens.
fn is_canonical_v4(id: &str) -> bool {
    uuid::Uuid::parse_str(id).is_ok_and(|parsed| {
        parsed.get_version_num() == 4
            && parsed.get_variant() == uuid::Variant::RFC4122
            && parsed.hyphenated().to_string() == id
    })
}

#[test]
fn pairing_says_this_installations_id_and_nothing_of_kind_or_capabilities() -> Result<()> {
    let folder = data_folder()?;
    let (hub, bodies) = refusing_hub()?;
    let body = pairing_body(folder.path(), &hub, &bodies)?;
    let mut keys = body
        .as_object()
        .with_context(|| format!("the pairing body is not an object: {body}"))?
        .keys()
        .map(String::as_str)
        .collect::<Vec<_>>();
    keys.sort_unstable();
    assert_eq!(keys, ["code", "installation_id", "name"], "{body}");
    assert_eq!(body["code"], CODE, "{body}");
    assert_eq!(body["name"], NAME, "{body}");
    let id = installation_id(&body)?;
    assert!(is_canonical_v4(&id), "not a canonical version 4 UUID: {id}");
    Ok(())
}

#[test]
fn the_installation_id_survives_a_restart_and_another_data_folder_has_another() -> Result<()> {
    let (hub, bodies) = refusing_hub()?;
    let folder = data_folder()?;
    let other_folder = data_folder()?;
    let first = installation_id(&pairing_body(folder.path(), &hub, &bodies)?)?;
    let after_restart = installation_id(&pairing_body(folder.path(), &hub, &bodies)?)?;
    let other = installation_id(&pairing_body(other_folder.path(), &hub, &bodies)?)?;
    assert_eq!(
        after_restart, first,
        "a restart changed the installation id"
    );
    assert_ne!(
        other, first,
        "two data folders have the same installation id"
    );
    Ok(())
}
