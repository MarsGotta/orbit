//! Synthetic MCP fixtures: no browser, daemon or desktop API is contacted.
#![allow(clippy::unwrap_used, clippy::expect_used)]
// Compiled here on its own, the module re-exports what only the crate uses.
#[allow(dead_code, unused_imports)]
#[path = "../src/cua.rs"]
mod cua;
// cua.rs takes its macOS engine census from the crate's app_control, which
// this test crate does not compile; the library's own census stands in for it.
#[cfg(target_os = "macos")]
mod app_control {
    pub(crate) use orbit_control::app_control::engine_process_listing;
}

use cua::engine::{PeerProbeDecision, PeerProbeFact, decide_peer_probe};
use cua::{CuaClient, CuaConfig, LocalCua, VERIFIED_DRIVER_VERSION, resolve_driver_path};
use serde_json::{Value, json};
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, DuplexStream, ReadHalf, WriteHalf};

type Client = CuaClient<BufReader<ReadHalf<DuplexStream>>, WriteHalf<DuplexStream>>;
type Requests = Arc<Mutex<Vec<Value>>>;

#[derive(Clone, Copy, PartialEq)]
enum Fault {
    None,
    WrongVersion,
    ToolError,
    ServerRequest,
    WrongResponseId,
    Oversize,
    /// The first tool call is answered with one complete frame the
    /// size of a Retina full-screen capture in base64; the rest normally.
    RetinaFrame,
    /// An answer that fits in a message, carrying an image larger
    /// than any capture Orbit can deliver.
    ImageOverCaptureCeiling,
    Eof,
    Hang,
}

fn fixture(fault: Fault) -> (Client, Requests) {
    fixture_within(fault, Duration::from_millis(300))
}

/// What one request may take while the fake engine hands over a Retina-sized
/// frame (6 291 641 bytes): building, sending and reading it to its end took
/// 179–211 ms in a debug build at load 15–25 (80 runs, 2026-10-21), and the
/// shared 300 ms failed three gate runs under heavier load the same day. The
/// test is about the size refusal, not about time; 5 s is over twenty times the
/// worst seen, like the file's other requests that do real work.
const RETINA_FRAME_REQUEST: Duration = Duration::from_secs(5);

fn fixture_within(fault: Fault, request_timeout: Duration) -> (Client, Requests) {
    let (client, server) = tokio::io::duplex(8192);
    let (read, write) = tokio::io::split(client);
    let client = CuaClient::from_transport(BufReader::new(read), write, request_timeout).unwrap();
    let requests: Requests = Arc::new(Mutex::new(Vec::new()));
    let history = requests.clone();
    tokio::spawn(async move {
        let (read, mut write) = tokio::io::split(server);
        let mut lines = BufReader::new(read).lines();
        let mut retina_sent = false;
        while let Ok(Some(line)) = lines.next_line().await {
            let request: Value = serde_json::from_str(&line).unwrap();
            history.lock().unwrap().push(request.clone());
            let method = request["method"].as_str().unwrap();
            if method == "notifications/initialized" {
                continue;
            }
            let result = match method {
                "initialize" => json!({"protocolVersion":"2025-06-18","capabilities":{"tools":{}},
                    "serverInfo":{"name":"cua-driver","version":if fault == Fault::WrongVersion {"0.28.1"} else {VERIFIED_DRIVER_VERSION}}}),
                "tools/list" => json!({"tools":[
                    {"name":"list_apps","inputSchema":{"type":"object"}},
                    {"name":"list_windows","inputSchema":{"type":"object"}},
                    {"name":"get_browser_state","inputSchema":{"type":"object"}},
                    {"name":"browser_click","inputSchema":{"type":"object"}}
                ]}),
                "tools/call" => {
                    let name = request["params"]["name"].as_str().unwrap();
                    assert!(["list_apps", "list_windows"].contains(&name));
                    if fault == Fault::Eof {
                        break;
                    }
                    if fault == Fault::Hang {
                        tokio::time::sleep(Duration::from_secs(2)).await;
                    }
                    if fault == Fault::Oversize {
                        let _ = write.write_all(&vec![b'x'; 1_048_577]).await;
                        break;
                    }
                    if fault == Fault::RetinaFrame && !retina_sent {
                        retina_sent = true;
                        let mut frame = serde_json::to_vec(&retina_frame(&request["id"])).unwrap();
                        frame.push(b'\n');
                        if write.write_all(&frame).await.is_err() {
                            break;
                        }
                        continue;
                    }
                    if fault == Fault::ServerRequest {
                        let _ = write.write_all(b"{\"jsonrpc\":\"2.0\",\"id\":50,\"method\":\"elicitation/create\"}\n").await;
                        continue;
                    }
                    if fault == Fault::ImageOverCaptureCeiling {
                        json!({"content":[{"type":"image","mimeType":"image/png",
                            "data":"A".repeat(IMAGE_OVER_CAPTURE_CEILING)}],"isError":false})
                    } else if fault == Fault::ToolError {
                        json!({"isError":true,"content":[{"type":"text","text":"fixture refusal"}]})
                    } else {
                        json!({"content":[{"type":"text","text":"Fixture native inventory"}],"structuredContent":{"fixture":name}})
                    }
                }
                other => panic!("Unexpected method {other}"),
            };
            let id = if fault == Fault::WrongResponseId && method == "tools/call" {
                json!(999)
            } else {
                request["id"].clone()
            };
            let response = json!({"jsonrpc":"2.0","id":id,"result":result});
            let mut bytes = serde_json::to_vec(&response).unwrap();
            bytes.push(b'\n');
            if write.write_all(&bytes).await.is_err() {
                break;
            }
        }
    });
    (client, requests)
}

/// The base64 a Retina full-screen PNG takes: a 3024×1964 desktop is several
/// MiB of PNG, so 6 MiB of base64 is the order of size a real call met on
/// 2026-10-18.
const RETINA_BASE64_BYTES: usize = 6 * 1024 * 1024;

fn retina_frame(id: &Value) -> Value {
    json!({"jsonrpc":"2.0","id":id,"result":{
        "content":[{"type":"image","mimeType":"image/png","data":"A".repeat(RETINA_BASE64_BYTES)}],
        "structuredContent":{"screenshot_width":3024,"screenshot_height":1964},
        "isError":false}})
}

/// A whole answer larger than a message may be is refused with its
/// size and the ceiling, and it costs that one request only: the frame was
/// read to its end, so the connection is still in step and the engine is not
/// killed and restarted for having answered.
#[tokio::test]
async fn a_frame_over_the_ceiling_names_its_size_and_keeps_the_engine() {
    let (mut client, history) = fixture_within(Fault::RetinaFrame, RETINA_FRAME_REQUEST);
    client.initialize().await.unwrap();
    let frame_bytes = serde_json::to_vec(&retina_frame(&json!(3))).unwrap().len() + 1;
    let error = client
        .call_tool("list_apps", json!({}))
        .await
        .unwrap_err()
        .to_string();
    assert!(
        error.contains(&frame_bytes.to_string()),
        "the refusal does not say the answer's size ({frame_bytes} bytes): {error}"
    );
    assert!(
        error.contains("1048576"),
        "the refusal does not say the ceiling of an engine message: {error}"
    );
    assert!(
        client.is_connected(),
        "a large answer closed the connection with the engine: {error}"
    );
    assert_eq!(
        client
            .call_tool("list_apps", json!({}))
            .await
            .unwrap()
            .structured
            .unwrap()["fixture"],
        "list_apps",
        "the next request did not come through the same connection"
    );
    let calls = history
        .lock()
        .unwrap()
        .iter()
        .filter(|request| request["method"] == "tools/call")
        .count();
    assert_eq!(calls, 2, "the refused request was repeated");
}

/// Over the 933 336 bytes of base64 a 700 000-byte PNG takes, and still well
/// inside one message.
const IMAGE_OVER_CAPTURE_CEILING: usize = 1_000_000;

/// An image no capture could carry is refused with its size and the
/// capture ceiling, not as «fuera de formato o tamaño», and the connection
/// stays.
#[tokio::test]
async fn an_image_over_the_capture_ceiling_names_its_size() {
    let (mut client, _) = fixture(Fault::ImageOverCaptureCeiling);
    client.initialize().await.unwrap();
    let error = client
        .call_tool("list_apps", json!({}))
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("1000000"), "{error}");
    assert!(error.contains("933336"), "{error}");
    assert!(error.contains("700000"), "{error}");
    assert!(client.is_connected(), "{error}");
}

#[tokio::test]
async fn real_read_tool_names_are_preserved() {
    let (mut client, history) = fixture(Fault::None);
    client.initialize().await.unwrap();
    // The catalogue is whatever the engine declared, not a list
    // Orbit filters. A tool Orbit's own helpers never call still comes through,
    // so a newer engine reaches Deimos without a change here.
    assert_eq!(client.describe().len(), 4);
    assert!(
        client
            .describe()
            .iter()
            .any(|tool| tool.name == "browser_click")
    );
    assert_eq!(client.engine_tools().len(), 4);
    assert!(client.declares_tool("browser_click"));
    assert!(!client.declares_tool("a_tool_no_engine_declares"));
    assert_eq!(
        client
            .call_tool("list_apps", json!({}))
            .await
            .unwrap()
            .structured
            .unwrap()["fixture"],
        "list_apps"
    );
    client.windows(Some(42)).await.unwrap();
    let requests = history.lock().unwrap();
    let calls: Vec<_> = requests
        .iter()
        .filter(|r| r["method"] == "tools/call")
        .collect();
    assert_eq!(calls[0]["params"]["arguments"], json!({}));
    assert_eq!(
        calls[1]["params"]["arguments"],
        json!({"pid":42,"on_screen_only":false})
    );
}

#[tokio::test]
async fn unsupported_versions_and_tool_refusals_are_not_success() {
    let (mut client, _) = fixture(Fault::WrongVersion);
    let error = client.initialize().await.unwrap_err().to_string();
    assert!(
        error.contains("0.28.1"),
        "the received version is missing: {error}"
    );
    assert!(
        error.contains(VERIFIED_DRIVER_VERSION),
        "the expected version is missing: {error}"
    );
    assert!(client.call_tool("list_apps", json!({})).await.is_err());
    let (mut client, _) = fixture(Fault::ToolError);
    client.initialize().await.unwrap();
    assert!(client.call_tool("list_apps", json!({})).await.is_err());
}

#[test]
fn default_driver_is_resolved_inside_the_bundle_and_override_wins() {
    let daemon = PathBuf::from("/Applications/Orbit.app/Contents/MacOS/Orbit");
    assert_eq!(
        resolve_driver_path("", &daemon).unwrap(),
        PathBuf::from(
            "/Applications/Orbit.app/Contents/Helpers/Orbit Engine.app/Contents/MacOS/cua-driver"
        )
    );
    assert_eq!(
        resolve_driver_path("/opt/cua-driver-dev", &daemon).unwrap(),
        PathBuf::from("/opt/cua-driver-dev")
    );
}

/// Held by every test of this file while it starts a process or writes an
/// executable, so none of that overlaps. A process being started is, until its
/// exec, a copy of the test process holding every descriptor this process
/// holds, and the tests run on several threads at once. Two starts that
/// overlap copy each other's pipes: a write a test expects to find nobody
/// reading lands in the copy (measured in 19 of 300 whole runs of this
/// file). A start that overlaps the writing of a script copies it open for
/// writing, and the script then cannot be run, "Text file busy" (1 to 11 of
/// 300). A test that starts a process without it brings both back.
static STARTING_PROCESSES: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Writes an executable driver fixture: the contents, the execute bit, and
/// nothing writable by group or others, which is what Orbit demands of a
/// driver before it will run it.
async fn write_driver(path: &Path, script: &str) {
    write_executable(path, script, 0o500).await;
}

/// Writes an executable fixture while no process is being started, so no
/// process can be holding it open for writing when it is run.
async fn write_executable(path: &Path, contents: &str, mode: u32) {
    let _starting = STARTING_PROCESSES.lock().await;
    fs::write(path, contents).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
}

/// Connects against an isolated driver fixture and returns the reported cause.
async fn connect_failure(directory: &Path, name: &str) -> String {
    let driver = directory.join(name);
    let socket = directory.join(format!("{name}.sock"));
    // `connect` starts the proxy, so it runs whole under the lock.
    let starting = STARTING_PROCESSES.lock().await;
    let result = LocalCua::connect(CuaConfig {
        driver_path: driver,
        socket_path: socket,
        request_timeout: Duration::from_secs(1),
    })
    .await;
    drop(starting);
    match result {
        Ok(_) => panic!("driver {name} should not have connected"),
        Err(error) => error.to_string(),
    }
}

/// Writes a forged durable record; a process of this user can do the same.
fn forge_engine_record(socket: &Path, pid: u32) {
    let record = socket.parent().unwrap().join("engine.pid");
    fs::write(&record, pid.to_string() + "\n").unwrap();
    fs::set_permissions(&record, fs::Permissions::from_mode(0o600)).unwrap();
}

/// A foreign listener with a forged record naming its actual PID cannot be
/// mistaken for an engine launched by this LocalCua instance.
#[tokio::test]
async fn a_forged_matching_pid_does_not_reuse_the_listener() {
    let directory = tempfile::tempdir().unwrap();
    fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700)).unwrap();
    let driver = directory.path().join("cua-driver");
    write_driver(&driver, &fake_driver()).await;
    let socket = directory.path().join("cua").join("driver.sock");
    // `connect` would make this directory private itself; the fixture makes it
    // first because the listener binds before `connect` runs.
    fs::create_dir_all(socket.parent().unwrap()).unwrap();
    fs::set_permissions(socket.parent().unwrap(), fs::Permissions::from_mode(0o700)).unwrap();
    let _listener = tokio::net::UnixListener::bind(&socket).unwrap();
    // Private, as a real engine leaves it: the test distinguishes listener
    // identity from socket permissions.
    fs::set_permissions(&socket, fs::Permissions::from_mode(0o600)).unwrap();
    forge_engine_record(&socket, std::process::id());

    let starting = STARTING_PROCESSES.lock().await;
    let result = LocalCua::connect(CuaConfig {
        driver_path: driver.clone(),
        socket_path: socket.clone(),
        request_timeout: Duration::from_secs(1),
    })
    .await;
    drop(starting);

    let client = result.expect("a new engine must replace the forged socket path");
    assert!(client.is_connected());
    let peer = tokio::net::UnixStream::connect(&socket)
        .await
        .unwrap()
        .peer_cred()
        .unwrap()
        .pid();
    assert!(peer.is_some() && peer != Some(std::process::id() as i32));
}

/// Even an actual orphaned engine cannot be adopted by a new daemon. Its
/// process remains alive; only the socket pathname is replaced.
#[tokio::test]
async fn an_orphaned_engine_is_not_reused_or_killed() {
    let directory = tempfile::tempdir().unwrap();
    fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700)).unwrap();
    let driver = directory.path().join("cua-driver");
    write_driver(&driver, &fake_driver()).await;
    let socket = directory.path().join("cua").join("driver.sock");
    // `connect` would make this directory private itself; the fixture engine
    // binds before `connect` runs, so the fixture makes it.
    fs::create_dir_all(socket.parent().unwrap()).unwrap();
    fs::set_permissions(socket.parent().unwrap(), fs::Permissions::from_mode(0o700)).unwrap();

    // The engine Orbit would have started: a `serve` process holding the
    // socket, launched under the same lock as every process this file starts.
    let starting = STARTING_PROCESSES.lock().await;
    let mut engine = tokio::process::Command::new(&driver)
        .arg("serve")
        .arg("--socket")
        .arg(&socket)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    // An upper bound on a fixture event, never a measurement: 60 s is the floor
    // these integration tests retain for a daemon under load.
    let deadline = std::time::Instant::now() + Duration::from_secs(60);
    while !std::os::unix::net::UnixStream::connect(&socket).is_ok() {
        assert!(
            std::time::Instant::now() < deadline,
            "the test engine did not open its socket"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    forge_engine_record(&socket, engine.id().unwrap());
    let result = LocalCua::connect(CuaConfig {
        driver_path: driver,
        socket_path: socket.clone(),
        request_timeout: Duration::from_secs(5),
    })
    .await;
    drop(starting);

    let client = result.expect("a new engine must replace the orphaned socket");
    assert!(client.is_connected());
    assert!(
        engine.try_wait().unwrap().is_none(),
        "the orphaned engine must not be killed without binary verification"
    );
    let peer = tokio::net::UnixStream::connect(&socket)
        .await
        .unwrap()
        .peer_cred()
        .unwrap()
        .pid();
    assert!(peer.is_some() && peer != engine.id().map(|pid| pid as i32));
    drop(client);
}

/// A forged nonmatching PID also cannot turn a live listener into a refusal
/// that prevents Orbit from starting its own engine.
#[tokio::test]
async fn a_forged_different_pid_does_not_block_own_engine() {
    let directory = tempfile::tempdir().unwrap();
    let driver = directory.path().join("cua-driver");
    write_driver(&driver, &fake_driver()).await;
    let socket = directory.path().join("driver.sock");
    let _listener = tokio::net::UnixListener::bind(&socket).unwrap();
    fs::set_permissions(&socket, fs::Permissions::from_mode(0o600)).unwrap();
    forge_engine_record(&socket, std::process::id() + 1);
    let starting = STARTING_PROCESSES.lock().await;
    let result = LocalCua::connect(CuaConfig {
        driver_path: driver,
        socket_path: socket.clone(),
        request_timeout: Duration::from_secs(5),
    })
    .await;
    drop(starting);
    let client = result.expect("a forged different PID must not block a fresh engine");
    assert!(client.is_connected());
    let peer = tokio::net::UnixStream::connect(&socket)
        .await
        .unwrap()
        .peer_cred()
        .unwrap()
        .pid();
    assert!(peer.is_some() && peer != Some(std::process::id() as i32));
}

/// The packaged macOS driver forks one `serve` child before binding. Reproduce
/// that topology with an isolated Linux fixture and require the listener to
/// descend from the child Orbit launched, rather than have the same PID.
#[cfg(target_os = "linux")]
#[tokio::test]
async fn a_descendant_of_the_launched_engine_can_serve() {
    let directory = tempfile::tempdir().unwrap();
    let driver = directory.path().join("cua-driver");
    write_driver(&driver, &descendant_driver()).await;
    let socket = directory.path().join("driver.sock");
    let starting = STARTING_PROCESSES.lock().await;
    let result = LocalCua::connect(CuaConfig {
        driver_path: driver,
        socket_path: socket.clone(),
        request_timeout: Duration::from_secs(5),
    })
    .await;
    drop(starting);

    let client = result.expect("the listener descended from Orbit's engine must be accepted");
    let parent: i32 = fs::read_to_string(format!("{}.parent", socket.display()))
        .unwrap()
        .parse()
        .unwrap();
    let listener: i32 = fs::read_to_string(format!("{}.listener", socket.display()))
        .unwrap()
        .parse()
        .unwrap();
    let peer = tokio::net::UnixStream::connect(&socket)
        .await
        .unwrap()
        .peer_cred()
        .unwrap()
        .pid();
    assert_ne!(
        parent, listener,
        "the fixture must fork a separate listener"
    );
    assert_eq!(peer, Some(listener));
    drop(client);
    // An upper bound on a fixture event, never a measurement: 60 s is the floor
    // these integration tests retain for a daemon under load.
    let deadline = std::time::Instant::now() + Duration::from_secs(60);
    while std::os::unix::net::UnixStream::connect(&socket).is_ok() {
        assert!(
            std::time::Instant::now() < deadline,
            "the fixture listener remained after its parent was dropped"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// A listener that races the new child to the pathname is still foreign.
#[tokio::test]
async fn a_listener_racing_the_new_child_is_refused() {
    let directory = tempfile::tempdir().unwrap();
    let driver = directory.path().join("cua-driver");
    write_driver(
        &driver,
        "#!/usr/bin/env python3\nimport os,sys,time\npath=sys.argv[-1]\nopen(path+'.ready','w').close()\nwhile not os.path.exists(path+'.go'):\n time.sleep(.01)\n",
    )
    .await;
    let socket = directory.path().join("driver.sock");
    let starting = STARTING_PROCESSES.lock().await;
    let connect = tokio::spawn(LocalCua::connect(CuaConfig {
        driver_path: driver,
        socket_path: socket.clone(),
        request_timeout: Duration::from_secs(1),
    }));
    let ready = socket.with_extension("sock.ready");
    // An upper bound on a fixture event, never a measurement: 60 s is the floor
    // these integration tests retain for a daemon under load.
    let deadline = std::time::Instant::now() + Duration::from_secs(60);
    while !ready.exists() {
        assert!(
            std::time::Instant::now() < deadline,
            "the fixture child did not start"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let _foreign = tokio::net::UnixListener::bind(&socket).unwrap();
    fs::set_permissions(&socket, fs::Permissions::from_mode(0o600)).unwrap();
    let result = connect.await.unwrap();
    drop(starting);
    let error = match result {
        Ok(_) => panic!("the racing listener must not be used"),
        Err(error) => error.to_string(),
    };
    assert!(error.contains("is held by PID"), "wrong refusal: {error}");
    assert!(
        error.contains("engine Orbit started"),
        "wrong refusal: {error}"
    );
    assert!(
        !error.contains("Could not verify"),
        "a foreign listener must be refused at once, not after the startup deadline: {error}"
    );
}

#[tokio::test]
async fn missing_and_unstartable_drivers_explain_the_failure() {
    let directory = tempfile::tempdir().unwrap();
    fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700)).unwrap();
    for (name, expected) in [
        ("missing-cua-driver", "CUA driver not found"),
        (
            "broken-cua-driver",
            "Could not start the bundled CUA engine",
        ),
    ] {
        if name == "broken-cua-driver" {
            // This has to be a driver that genuinely cannot start
            // on the platform running the test. A plain text file with the
            // execute bit is not: Linux refuses it with ENOEXEC, but macOS
            // hands such a file to the shell, so there it starts, dies, and
            // proves something else. A `#!` naming an interpreter that does
            // not exist fails inside exec itself, with ENOENT, on both
            // platforms — the kernel reads the header, looks the interpreter
            // up, finds nothing, and no shell fallback can rescue it.
            write_driver(
                &directory.path().join(name),
                "#!/orbit/no/existe/este/interprete\n",
            )
            .await;
        }
        let error = connect_failure(directory.path(), name).await;
        assert!(error.contains(expected), "failure without context: {error}");
    }
}

/// This is the failure the Mac actually produces: a driver binary
/// that starts and dies at once. Saying only that the connection closed is
/// true and useless, so the message has to name the death and its code.
#[tokio::test]
async fn an_engine_that_dies_at_birth_names_its_cause() {
    let directory = tempfile::tempdir().unwrap();
    fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700)).unwrap();
    write_driver(
        &directory.path().join("suicidal-cua-driver"),
        "#!/bin/sh\nexit 3\n",
    )
    .await;
    let error = connect_failure(directory.path(), "suicidal-cua-driver").await;
    assert!(
        error.contains("The CUA engine exited"),
        "the failure does not say the engine ended: {error}"
    );
    assert!(
        error.contains('3'),
        "the failure does not say which code it ended with: {error}"
    );
    assert!(
        !error.contains("MCP connection closed"),
        "the failure stops at the connection having closed: {error}"
    );
}

/// The engine serves normally; its proxy closes stdout while
/// remaining alive. No engine death may be attributed to it.
#[tokio::test]
async fn a_connection_that_really_closes_keeps_its_message() {
    let directory = tempfile::tempdir().unwrap();
    fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700)).unwrap();
    // Closes stdout — Orbit sees EOF at once — and then stays alive well past
    // any grace Orbit gives a process to finish dying.
    let driver = fake_driver().replace(
        "for line in sys.stdin:",
        "if mode == 'mcp':\n    os.close(1)\n    import time\n    time.sleep(30)\nfor line in sys.stdin:",
    );
    write_driver(&directory.path().join("mute-cua-driver"), &driver).await;
    let error = connect_failure(directory.path(), "mute-cua-driver").await;
    assert!(
        error.contains("MCP connection closed before a response"),
        "the closed connection lost its message: {error}"
    );
    assert!(
        !error.contains("exited"),
        "a death that did not happen is attributed to it: {error}"
    );
}

/// A stand-in for the MCP proxy, started here rather than by `connect`, so the
/// test decides what has happened to it before Orbit writes its first byte.
/// The pipes are the ones `connect` gives the proxy; `/bin/sh` by absolute
/// path, and nothing from this environment, as in `driver_command`.
async fn spawn_proxy(script: &str, arguments: &[&Path]) -> tokio::process::Child {
    let _starting = STARTING_PROCESSES.lock().await;
    tokio::process::Command::new("/bin/sh")
        .arg("-c")
        .arg(script)
        .arg("engine-proxy")
        .args(arguments)
        .env_clear()
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .unwrap()
}

/// Returns once the process has exited, without collecting its status, so the
/// status is still there for Orbit to learn. It waits on the exit itself; the
/// bound only turns a fixture that never exits into a failure instead of a
/// hang, and says nothing about how long an exit takes.
async fn exited_but_not_collected(pid: u32) {
    let pid = rustix::process::Pid::from_raw(i32::try_from(pid).unwrap()).unwrap();
    let waiting = tokio::task::spawn_blocking(move || {
        loop {
            match rustix::process::waitid(
                rustix::process::WaitId::Pid(pid),
                rustix::process::WaitIdOptions::EXITED | rustix::process::WaitIdOptions::NOWAIT,
            ) {
                Err(rustix::io::Errno::INTR) => continue,
                // Only whether it returned crosses the thread: on macOS the
                // status holds a raw pointer, which is not `Send`.
                other => return other.map(|_| ()),
            }
        }
    });
    // Without NOHANG, `waitid` returns only once the process has exited. What
    // it reports about the exit is not read, so the test does not depend on
    // how each platform fills that record in.
    tokio::time::timeout(Duration::from_secs(60), waiting)
        .await
        .expect("the fixture process did not end within 60 s")
        .unwrap()
        .unwrap();
}

/// The race behind `an_engine_that_dies_at_birth_names_its_cause`, with its
/// outcome fixed: the engine
/// has exited with code 3 before Orbit writes to it, so it is the write that
/// finds it gone, with a broken pipe (run alone, 50 of 50 times before the
/// fix). That is how the transport learns the engine is gone, never why; the
/// message has to name the death, as it does for end of file.
#[tokio::test]
async fn a_write_to_an_engine_already_dead_names_its_death() {
    let proxy = spawn_proxy("exit 3", &[]).await;
    exited_but_not_collected(proxy.id().unwrap()).await;
    let mut client = LocalCua::from_process(proxy, Duration::from_secs(1)).unwrap();
    let error = client.initialize().await.unwrap_err().to_string();
    assert!(
        error.contains("The CUA engine exited immediately after startup"),
        "the failure does not say the engine ended at startup: {error}"
    );
    assert!(
        error.contains("exit status: 3"),
        "the failure does not say which code it ended with: {error}"
    );
    assert!(
        !error.contains("Broken pipe"),
        "the failure stops at the broken pipe: {error}"
    );
    assert!(!client.is_connected());
}

/// The answer a verified engine gives to `initialize`, the first request.
fn initialize_answer() -> Value {
    json!({"jsonrpc":"2.0","id":1,"result":{
        "protocolVersion":"2025-06-18","capabilities":{"tools":{}},
        "serverInfo":{"name":"cua-driver","version":VERIFIED_DRIVER_VERSION}}})
}

/// What a death after the engine answered has to say: that this request went
/// unanswered, and that earlier ones did not.
const DEATH_AFTER_ANSWERING: &str =
    "without answering this request, after answering previous requests";

/// After the handshake. The engine answered, so saying it died at
/// birth without answering, with a binary that may be broken, would be false.
/// The death is named with its code and as what it was, whichever side of the
/// transport finds it: the next write, when the engine is already gone, or the
/// read after it, when the engine takes the request and then exits.
#[tokio::test]
async fn a_death_after_the_handshake_is_not_called_a_death_at_birth() {
    let initialize = initialize_answer();
    let tools = json!({"jsonrpc":"2.0","id":2,"result":{"tools":[
        {"name":"list_apps","inputSchema":{"type":"object"}},
        {"name":"list_windows","inputSchema":{"type":"object"}},
        {"name":"get_browser_state","inputSchema":{"type":"object"}}]}});
    for found_by_the_write in [true, false] {
        // Reads `initialize`, answers it, reads the notification and
        // `tools/list`, answers that, and exits with 3: at once, or after
        // reading the next request.
        let next_request = if found_by_the_write {
            ""
        } else {
            "read -r line\n"
        };
        let script = format!(
            "read -r line\nprintf '%s\\n' '{initialize}'\nread -r line\nread -r line\nprintf '%s\\n' '{tools}'\n{next_request}exit 3\n"
        );
        let proxy = spawn_proxy(&script, &[]).await;
        let pid = proxy.id().unwrap();
        let mut client = LocalCua::from_process(proxy, Duration::from_secs(5)).unwrap();
        client.initialize().await.unwrap();
        if found_by_the_write {
            // Dead before the next write, so the write is what fails.
            exited_but_not_collected(pid).await;
        }
        let error = client
            .call_tool("list_apps", json!({}))
            .await
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("The CUA engine exited (exit status: 3)"),
            "the failure does not say the engine ended, with its code: {error}"
        );
        assert!(
            error.contains(DEATH_AFTER_ANSWERING),
            "the failure does not say this request went unanswered: {error}"
        );
        assert!(
            !error.contains("immediately after startup") && !error.contains("answering MCP"),
            "a death at startup is attributed to an engine that answered: {error}"
        );
        assert!(
            !error.contains("Broken pipe") && !error.contains("MCP connection closed"),
            "the failure stops at the transport: {error}"
        );
    }
}

/// The engine answers `initialize`, reads the notification
/// and `tools/list`, and exits with 3: the request that goes unanswered is the
/// second one, the first that is not `initialize`. That is the boundary between
/// a death at birth and a death after answering, and this is its edge.
#[tokio::test]
async fn a_death_at_the_second_request_is_a_death_after_answering() {
    let initialize = initialize_answer();
    let proxy = spawn_proxy(
        &format!(
            "read -r line\nprintf '%s\\n' '{initialize}'\nread -r line\nread -r line\nexit 3\n"
        ),
        &[],
    )
    .await;
    let mut client = LocalCua::from_process(proxy, Duration::from_secs(5)).unwrap();
    let error = client.initialize().await.unwrap_err().to_string();
    assert!(
        !error.contains("immediately after startup"),
        "a death at startup is attributed to an engine that answered initialize: {error}"
    );
    assert!(
        error.contains("The CUA engine exited (exit status: 3)"),
        "the failure does not say the engine ended, with its code: {error}"
    );
    assert!(
        error.contains(DEATH_AFTER_ANSWERING),
        "the failure does not say this request went unanswered: {error}"
    );
}

/// The engine reads `initialize`, lets go of its stdin,
/// and only then answers and exits with 3, so the `initialized` notification
/// Orbit sends next finds nobody reading. That write is where Orbit learns of
/// the death, and it is named with its code like a request's would be.
#[tokio::test]
async fn a_death_before_the_initialized_notification_is_named() {
    let initialize = initialize_answer();
    let proxy = spawn_proxy(
        &format!("read -r line\nexec <&-\nprintf '%s\\n' '{initialize}'\nexit 3\n"),
        &[],
    )
    .await;
    let mut client = LocalCua::from_process(proxy, Duration::from_secs(5)).unwrap();
    let error = client.initialize().await.unwrap_err().to_string();
    assert!(
        error.contains("The CUA engine exited (exit status: 3)"),
        "the failure does not say the engine ended, with its code: {error}"
    );
    assert!(
        error.contains("without receiving notification notifications/initialized"),
        "the failure does not say it was the notification that did not arrive: {error}"
    );
    assert!(
        !error.contains("immediately after startup") && !error.contains("Broken pipe"),
        "the failure does not name the death after answering initialize: {error}"
    );
    assert!(!client.is_connected());
}

/// A notification that cannot be written in time is a
/// timeout, said as one with its ceiling, and not the same words as a death or
/// a broken pipe. The peer answers `initialize` and then never reads again, on
/// a transport too small to take the notification, so the write can only end
/// by the timeout under test.
#[tokio::test]
async fn a_notification_that_cannot_be_written_in_time_says_it_timed_out() {
    let (client, server) = tokio::io::duplex(16);
    let (read, write) = tokio::io::split(client);
    // Generous for an answer that crosses memory inside this process, which
    // the scheduler has delayed by at most 77 ms here (measured); the
    // notification never gets through, so this is also how long the test takes.
    let mut client =
        CuaClient::from_transport(BufReader::new(read), write, Duration::from_millis(500)).unwrap();
    tokio::spawn(async move {
        let (read, mut write) = tokio::io::split(server);
        let mut lines = BufReader::new(read).lines();
        let _ = lines.next_line().await;
        let mut answer = serde_json::to_vec(&initialize_answer()).unwrap();
        answer.push(b'\n');
        let _ = write.write_all(&answer).await;
        // Keeps both halves open and reads nothing more.
        std::future::pending::<()>().await;
    });
    let error = client.initialize().await.unwrap_err().to_string();
    assert!(
        error.contains("timed out after 500 ms"),
        "the failure does not say the time ran out, with its limit: {error}"
    );
    assert!(
        error.contains("notifications/initialized"),
        "the failure does not say what could not be sent: {error}"
    );
    assert!(!client.is_connected());
}

/// The guardrail on the dead-engine write case, from the writing side: this
/// engine is alive,
/// it only stopped reading, so the write fails with a broken pipe exactly as it
/// would against a dead one. No death may be attributed to it, and the message
/// stays the transport's own.
#[tokio::test]
async fn a_live_engine_that_stops_reading_is_not_declared_dead() {
    let directory = tempfile::tempdir().unwrap();
    let closed = directory.path().join("closed");
    // Closes its stdin and stdout, says so with a file, and stays alive well
    // past any grace Orbit gives a process to finish dying.
    let proxy = spawn_proxy("exec <&- >&-\n: > \"$1\"\nexec /bin/sleep 30\n", &[&closed]).await;
    let deadline = std::time::Instant::now() + Duration::from_secs(60);
    while !closed.exists() {
        assert!(
            std::time::Instant::now() < deadline,
            "the fixture did not close its input within 60 s"
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    let mut client = LocalCua::from_process(proxy, Duration::from_secs(1)).unwrap();
    let error = client.initialize().await.unwrap_err().to_string();
    assert!(
        !error.contains("exited"),
        "a death that did not happen is attributed to it: {error}"
    );
    // Only because no other start overlaps this test's (`STARTING_PROCESSES`):
    // otherwise another process can still hold this pipe and the write lands.
    assert!(
        error.contains("Broken pipe"),
        "the broken write lost its message: {error}"
    );
}

#[tokio::test]
async fn transport_failures_poison_the_session_without_retry() {
    for fault in [
        Fault::ServerRequest,
        Fault::WrongResponseId,
        Fault::Oversize,
        Fault::Eof,
        Fault::Hang,
    ] {
        let (mut client, history) = fixture(fault);
        client.initialize().await.unwrap();
        assert!(client.call_tool("list_apps", json!({})).await.is_err());
        let before = history.lock().unwrap().len();
        assert!(client.call_tool("list_apps", json!({})).await.is_err());
        assert_eq!(before, history.lock().unwrap().len());
    }
}

#[tokio::test]
async fn connection_rejects_paths_before_any_process_can_start() {
    for socket in ["relative.sock", "/tmp/../driver.sock"] {
        let result = LocalCua::connect(CuaConfig {
            driver_path: PathBuf::from("/nonexistent-driver"),
            socket_path: PathBuf::from(socket),
            request_timeout: Duration::from_secs(1),
        })
        .await;
        assert!(result.is_err());
    }
    // The driver is verified before anything is created or started, so
    // a missing binary is named as such and no engine process is ever spawned.
    let file = tempfile::NamedTempFile::new().unwrap();
    let result = LocalCua::connect(CuaConfig {
        driver_path: PathBuf::from("/nonexistent-driver"),
        socket_path: file.path().to_owned(),
        request_timeout: Duration::from_secs(1),
    })
    .await;
    assert!(
        result
            .err()
            .unwrap()
            .to_string()
            .contains("CUA driver not found")
    );
}

/// Orbit chooses the socket path inside its own state root, so the
/// directory it will make private is one it owns.
#[test]
fn the_default_socket_lives_under_orbit_own_private_root() {
    let root = PathBuf::from("/home/someone/.orbit");
    let socket = orbit_control::cua::default_socket_path(&root);
    assert!(socket.starts_with(&root));
    assert!(socket.parent().is_some_and(|parent| parent != root));
}

#[tokio::test]
async fn calls_before_initialization_and_invalid_timeouts_fail_locally() {
    let (mut client, history) = fixture(Fault::None);
    assert!(client.call_tool("list_apps", json!({})).await.is_err());
    assert!(history.lock().unwrap().is_empty());
    let (stream, _) = tokio::io::duplex(8);
    let (read, write) = tokio::io::split(stream);
    assert!(CuaClient::from_transport(BufReader::new(read), write, Duration::ZERO).is_err());
}

/// A stand-in for the packaged engine: `serve` binds the private socket Orbit
/// hands it and keeps it, `mcp` answers the handshake over stdin and stdout.
/// It has no desktop API, no browser and no network.
fn fake_driver() -> String {
    format!(
        r#"#!/usr/bin/env python3
import json, os, socket, sys

mode = sys.argv[1]
path = sys.argv[sys.argv.index("--socket") + 1]

if mode == "serve":
    server = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    server.bind(path)
    os.chmod(path, 0o600)
    server.listen(8)
    while True:
        connection, _ = server.accept()
        connection.close()

for line in sys.stdin:
    request = json.loads(line)
    method = request.get("method")
    if method == "initialize":
        result = {{"protocolVersion": "2025-06-18", "capabilities": {{"tools": {{}}}},
                  "serverInfo": {{"name": "cua-driver", "version": "{version}"}}}}
    elif method == "tools/list":
        result = {{"tools": [{{"name": name, "inputSchema": {{"type": "object"}}}}
                            for name in ("list_apps", "list_windows")]}}
    else:
        continue
    sys.stdout.write(json.dumps({{"jsonrpc": "2.0", "id": request["id"], "result": result}}) + "\n")
    sys.stdout.flush()
"#,
        version = VERIFIED_DRIVER_VERSION
    )
}

#[cfg(target_os = "linux")]
fn descendant_driver() -> String {
    let driver = fake_driver().replace(
        "import json, os, socket, sys",
        "import json, os, select, socket, sys",
    );
    let forked = driver.replace(
        "if mode == \"serve\":\n    server =",
        "if mode == \"serve\":\n    reader, writer = os.pipe()\n    listener = os.fork()\n    if listener:\n        os.close(reader)\n        with open(path + '.parent', 'w') as marker:\n            marker.write(str(os.getpid()))\n        os.waitpid(listener, 0)\n        sys.exit(0)\n    os.close(writer)\n    with open(path + '.listener', 'w') as marker:\n        marker.write(str(os.getpid()))\n    server =",
    );
    assert_ne!(forked, driver, "the fixture must fork before binding");
    let multiplexed = forked.replace(
        "    while True:\n        connection, _ = server.accept()",
        "    while True:\n        readable, _, _ = select.select([server, reader], [], [])\n        if reader in readable:\n            break\n        connection, _ = server.accept()",
    );
    assert_ne!(
        multiplexed, forked,
        "the fixture must exit when its parent dies"
    );
    multiplexed
}

/// The packaged driver leaves its listener running when the first `serve`
/// dies. This fixture does too; a marker lets the test clean it up even when
/// the assertion fails against the old single-process cleanup.
#[cfg(target_os = "linux")]
fn orphaning_descendant_driver() -> String {
    let driver = fake_driver();
    let forked = driver.replace(
        "if mode == \"serve\":\n    server =",
        "if mode == \"serve\":\n    listener = os.fork()\n    if listener:\n        with open(path + '.parent', 'w') as marker:\n            marker.write(str(os.getpid()))\n        with open(path + '.group', 'w') as marker:\n            marker.write(str(os.getpgrp()))\n        os.waitpid(listener, 0)\n        sys.exit(0)\n    with open(path + '.listener', 'w') as marker:\n        marker.write(str(os.getpid()))\n    server =",
    );
    assert_ne!(forked, driver, "the fixture must fork before binding");
    let bounded = forked.replace(
        "    while True:\n        connection, _ = server.accept()",
        "    server.settimeout(0.1)\n    while os.path.exists(path + '.keep'):\n        try:\n            connection, _ = server.accept()\n        except socket.timeout:\n            continue",
    );
    assert_ne!(bounded, forked, "the fixture must honor its cleanup marker");
    bounded
}

/// Killing only A must not leave B serving after the client is gone.
#[cfg(target_os = "linux")]
#[tokio::test]
async fn forked_listener_stops_with_the_connection() {
    for poison in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let driver = directory.path().join("cua-driver");
        write_driver(&driver, &orphaning_descendant_driver()).await;
        let socket = directory.path().join("driver.sock");
        let marker = PathBuf::from(format!("{}.keep", socket.display()));
        fs::write(&marker, "keep").unwrap();
        let starting = STARTING_PROCESSES.lock().await;
        let mut client = LocalCua::connect(CuaConfig {
            driver_path: driver,
            socket_path: socket.clone(),
            request_timeout: Duration::from_secs(5),
        })
        .await
        .unwrap();
        drop(starting);
        let parent: u32 = fs::read_to_string(format!("{}.parent", socket.display()))
            .unwrap()
            .parse()
            .unwrap();
        let listener: u32 = fs::read_to_string(format!("{}.listener", socket.display()))
            .unwrap()
            .parse()
            .unwrap();
        let group: u32 = fs::read_to_string(format!("{}.group", socket.display()))
            .unwrap()
            .parse()
            .unwrap();
        assert_ne!(parent, listener);
        assert_eq!(group, parent, "the fixture must run in its own group");
        assert!(std::os::unix::net::UnixStream::connect(&socket).is_ok());
        if poison {
            client.invalidate();
        } else {
            drop(client);
        }
        // SIGKILL to the group is immediate; 60 s only bounds a failure, and is
        // the existing budget for these tests under load.
        let deadline = std::time::Instant::now() + Duration::from_secs(60);
        let mut still_listening = false;
        while std::time::Instant::now() < deadline {
            still_listening = std::os::unix::net::UnixStream::connect(&socket).is_ok();
            if !still_listening {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        fs::remove_file(marker).unwrap();
        assert!(
            !still_listening,
            "the forked listener PID {listener} outlived launched engine PID {parent}"
        );
    }
}

#[test]
fn parent_lookup_error_keeps_the_specific_cause() {
    let inner = anyhow::anyhow!("unreadable PPID: PID 42 disappeared");
    let error = cua::engine::parent_lookup_context(42, 7, 2, inner).to_string();
    assert!(error.contains("peer PID 42"), "missing peer: {error}");
    assert!(error.contains("engine PID 7"), "missing child: {error}");
    assert!(error.contains("step 2/4"), "missing hop: {error}");
    assert!(
        error.contains("unreadable PPID: PID 42 disappeared"),
        "missing underlying cause: {error}"
    );
}

#[test]
fn each_peer_probe_decision_preserves_identity_and_cause() {
    let socket = Path::new("/fixture/driver.sock");
    let cases = [
        (
            PeerProbeFact::CredentialError(std::io::Error::other("socket closed")),
            "peer_cred()",
        ),
        (PeerProbeFact::PidMissing, "pid()"),
        (
            PeerProbeFact::Ancestry {
                peer: 42,
                result: Err(anyhow::anyhow!("ps: PID 42 disappeared")),
            },
            "ps: PID 42 disappeared",
        ),
    ];
    for (fact, cause) in cases {
        match decide_peer_probe(fact, 7, socket) {
            PeerProbeDecision::Retry(message) => assert!(message.contains(cause), "{message}"),
            other => panic!("missing identity must retry with cause, got {other:?}"),
        }
    }
    match decide_peer_probe(
        PeerProbeFact::Ancestry {
            peer: 42,
            result: Ok(None),
        },
        7,
        socket,
    ) {
        PeerProbeDecision::Reject(message) => {
            assert!(message.contains("PID 42"), "{message}");
            assert!(message.contains("PID 7"), "{message}");
            assert!(message.contains("4 ancestry steps"), "{message}");
        }
        other => panic!("foreign listener must be rejected immediately, got {other:?}"),
    }
    assert_eq!(
        decide_peer_probe(
            PeerProbeFact::Ancestry {
                peer: 42,
                result: Ok(Some(1)),
            },
            7,
            socket,
        ),
        PeerProbeDecision::Accept
    );
}

#[test]
fn ps_timeout_names_the_remaining_startup_budget() {
    let message = cua::engine::ps_timeout_message(42, Duration::from_millis(137));
    assert!(
        message
            .contains("did not respond within the 137 ms remaining of the 10000 ms startup budget"),
        "wrong deadline: {message}"
    );
    assert!(message.contains("PID 42"), "missing PID: {message}");
}

/// Closing Orbit stops the directly bound fixture engine. The
/// packaged macOS driver may fork, so descendant cleanup needs native evidence.
#[tokio::test]
async fn closing_the_connection_stops_the_engine_orbit_started() {
    let directory = tempfile::tempdir().unwrap();
    fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700)).unwrap();
    let driver = directory.path().join("cua-driver");
    write_executable(&driver, &fake_driver(), 0o700).await;
    let socket = directory.path().join("cua").join("driver.sock");

    // `connect` starts the engine and the proxy, so it runs whole under the lock.
    let starting = STARTING_PROCESSES.lock().await;
    let mut client = LocalCua::connect(CuaConfig {
        driver_path: driver,
        socket_path: socket.clone(),
        request_timeout: Duration::from_secs(5),
    })
    .await
    .unwrap();
    drop(starting);

    // The engine Orbit started is serving: that is what has to stop.
    assert!(
        std::os::unix::net::UnixStream::connect(&socket).is_ok(),
        "the engine was not left serving its socket"
    );

    client.invalidate();

    // Generous on purpose: the loop leaves the moment the socket stops
    // answering, and a contended machine must not turn this into noise.
    let deadline = std::time::Instant::now() + Duration::from_secs(60);
    while std::os::unix::net::UnixStream::connect(&socket).is_ok() {
        assert!(
            std::time::Instant::now() < deadline,
            "the engine kept running after the connection closed"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}
