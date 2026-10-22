use anyhow::{Context, Result, bail};
use base64::{Engine, engine::general_purpose::STANDARD};
use orbit_core::{DurableState, privacy_stop::StopRequester, protocol::Command, refusal::Refusal};
use orbit_daemon::{
    config::{Paths, set_secret},
    daemon, ipc,
};
use orbit_store::{Job, JobState, Store};
use serde_json::{Value, json};
use std::{fs::OpenOptions, io::Write, time::Instant};

fn recovery_fixture() -> Result<(tempfile::TempDir, Paths)> {
    let directory = tempfile::tempdir()?;
    let paths = Paths::resolve(Some(directory.path().join("orbit")))?;
    paths.initialize()?;
    set_secret(&paths, "storage", &STANDARD.encode([0x42; 32]), true)?;
    let mut store = Store::open(&paths.store, [0x42; 32])?;
    for serial in 0..12 {
        store.enqueue_job(&Job {
            id: format!("recovery-upload-{serial:02}"),
            kind: "chunk_upload".into(),
            payload: json!({"fixture": serial}),
            state: JobState::Running,
            created_at: serial,
        })?;
    }
    Ok((directory, paths))
}

#[test]
fn daemon_future_fits_the_test_thread_stack() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let paths = Paths::resolve(Some(directory.path().join("orbit")))?;
    let future = daemon::run_with_helper(paths, true, None);
    let bytes = std::mem::size_of_val(&future);
    println!("daemon_future_bytes={bytes}");
    // The harness gives this thread 2 MiB. Keep the top-level future below
    // 64 KiB (well below the observed 113,328 B regression), leaving stack space
    // for its callers; this is not a daemon memory or queue limit.
    assert!(bytes < 64 * 1024, "daemon future uses {bytes} stack bytes");
    Ok(())
}

struct DeviceKeyGate {
    release: std::sync::mpsc::Sender<()>,
    opened: tokio::sync::oneshot::Receiver<bool>,
    writer: std::thread::JoinHandle<Result<()>>,
}

fn gate_device_key(paths: &Paths) -> Result<DeviceKeyGate> {
    let fifo = paths.root.join("development-device.key");
    let status = std::process::Command::new("mkfifo")
        .arg("-m")
        .arg("600")
        .arg(&fifo)
        .status()
        .context("mkfifo could not create the fixture")?;
    if !status.success() {
        bail!("mkfifo fixture failed: {status}");
    }
    let (release, release_rx) = std::sync::mpsc::channel::<()>();
    let (opened_tx, opened) = tokio::sync::oneshot::channel();
    let socket_path = paths.socket.clone();
    let writer = std::thread::spawn(move || -> Result<()> {
        let mut file = OpenOptions::new().write(true).open(fifo)?;
        let socket_bound = socket_path.exists();
        let _ = opened_tx.send(socket_bound);
        // An old daemon blocks the current-thread test before it can poll
        // IPC; release that case on the observed socket fact.
        if socket_bound {
            let _ = release_rx.recv();
        }
        file.write_all(b"synthetic-device-token")?;
        Ok(())
    });
    Ok(DeviceKeyGate {
        release,
        opened,
        writer,
    })
}

impl DeviceKeyGate {
    async fn wait_opened(&mut self) -> Result<bool> {
        (&mut self.opened)
            .await
            .context("fixture credential was not read")
    }

    fn release(self) -> Result<()> {
        self.release.send(())?;
        self.writer
            .join()
            .map_err(|_| anyhow::anyhow!("fixture writer panicked"))??;
        Ok(())
    }
}

fn run_restart_fixture<F, Fut>(fixture: F) -> Result<()>
where
    F: FnOnce() -> Fut + Send + 'static,
    Fut: std::future::Future<Output = Result<()>> + 'static,
{
    // The test harness's 2 MiB stack aborted when a lost stop reached the
    // second daemon. Eight MiB gives this two-start fixture four times that
    // space; it is a test-thread stack, not a daemon resource limit.
    let worker = std::thread::Builder::new()
        .name("startup-restart-fixture".into())
        .stack_size(8 * 1024 * 1024)
        .spawn(move || -> Result<()> {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?;
            runtime.block_on(fixture())
        })?;
    match worker.join() {
        Ok(result) => result,
        Err(panic) => std::panic::resume_unwind(panic),
    }
}

async fn connected_status(
    paths: &Paths,
    daemon: &mut tokio::task::JoinHandle<Result<()>>,
) -> Result<Value> {
    let started = Instant::now();
    loop {
        if tokio::net::UnixStream::connect(&paths.socket).await.is_ok() {
            let response = ipc::request(paths, Command::Status).await?;
            return Ok(serde_json::to_value(
                response.status.context("status had no state")?,
            )?);
        }
        tokio::select! {
            outcome = &mut *daemon => {
                outcome.context("daemon stopped before opening its socket")??;
                bail!("daemon stopped before opening its socket");
            }
            () = tokio::time::sleep(std::time::Duration::from_millis(10)) => {}
        }
        // A jam guard, not the readiness criterion: the socket or daemon exit
        // decides the result. The old disk worst was 36.4 s under load.
        if started.elapsed() > std::time::Duration::from_secs(120) {
            bail!(
                "daemon neither opened its socket nor stopped in {:.1} s",
                started.elapsed().as_secs_f64()
            );
        }
    }
}

async fn recovery_finished(paths: &Paths) -> Result<Value> {
    recovery_event(paths, "state", "ready").await
}

async fn wait_for_step(paths: &Paths, step: &str) -> Result<Value> {
    recovery_event(paths, "step", step).await
}

async fn recovery_event(paths: &Paths, field: &str, expected: &str) -> Result<Value> {
    let wait = async {
        loop {
            let response = ipc::request(paths, Command::Status).await?;
            let state =
                serde_json::to_value(response.status.context("status omitted startup facts")?)?;
            if state["recovery"][field] == expected {
                return Ok(state);
            }
            if field == "step" && state["recovery"]["state"] == "ready" {
                bail!("recovery finished before reaching step {expected}");
            }
            tokio::task::yield_now().await;
        }
    };
    tokio::time::timeout(std::time::Duration::from_secs(120), wait)
        .await
        .context("recovery status did not arrive within the startup jam guard")?
}

#[tokio::test(flavor = "current_thread")]
async fn socket_reports_recovery_and_rejects_store_commands() -> Result<()> {
    tokio::task::LocalSet::new()
        .run_until(async {
            let (_directory, paths) = recovery_fixture()?;
            let mut gate = gate_device_key(&paths)?;
            let daemon_paths = paths.clone();
            let mut daemon =
                tokio::task::spawn_local(async move { daemon::run(daemon_paths, true).await });
            assert!(gate.wait_opened().await?, "credential preceded socket");
            let state = connected_status(&paths, &mut daemon).await?;
            let recovery = &state["recovery"];
            assert!(recovery.is_object(), "status hid recovery: {state}");
            assert_eq!(recovery["state"], "recovering");
            assert!(
                recovery["step"].is_string(),
                "status hid recovery step: {state}"
            );
            assert!(
                recovery["elapsed_ms"].is_number(),
                "status hid elapsed time: {state}"
            );
            let refusal = match ipc::request(&paths, Command::NewSession { agent_id: None }).await {
                Ok(_) => bail!("store command ran during recovery"),
                Err(error) => error
                    .downcast_ref::<Refusal>()
                    .context("recovery refusal was not typed")?
                    .clone(),
            };
            assert_eq!(serde_json::to_value(&refusal)?["code"], "recovering");
            let reason = refusal.to_string();
            assert_eq!(
                serde_json::from_str::<Value>(&reason)?,
                serde_json::to_value(&refusal)?
            );
            let wire = serde_json::to_value(&refusal)?;
            assert!(wire["step"].as_str().is_some_and(|step| !step.is_empty()));
            let elapsed = wire["elapsed_ms"]
                .as_u64()
                .context("recovery refusal lacks elapsed milliseconds")?;
            let observed = recovery["elapsed_ms"]
                .as_u64()
                .context("status lacked elapsed milliseconds")?;
            assert!(elapsed >= observed);
            let audio = ipc::voice_note_audio(&paths, "refused-during-recovery".into()).await;
            gate.release()?;
            std::fs::remove_file(paths.root.join("development-device.key"))?;
            let ready = recovery_finished(&paths).await?;
            assert_eq!(ready["recovery"]["state"], "ready");
            ipc::request(&paths, Command::Shutdown).await?;
            daemon.await??;
            let store = Store::open_existing(&paths.store, [0x42; 32])?;
            let jobs = store.list_jobs(20)?;
            assert_eq!(jobs.len(), 12);
            assert!(jobs.iter().all(|job| job.state == JobState::RetryWait));
            let error = audio.err().context("voice audio ran during recovery")?;
            let refusal = error
                .downcast_ref::<Refusal>()
                .context("voice recovery refusal was not typed")?;
            assert!(matches!(refusal, Refusal::Recovering { .. }));
            let voice = serde_json::to_value(refusal)?;
            assert_eq!(voice["step"], wire["step"]);
            assert!(
                voice["elapsed_ms"]
                    .as_u64()
                    .context("voice refusal lacks elapsed milliseconds")?
                    >= elapsed
            );

            Ok(())
        })
        .await
}

#[test]
fn privacy_stop_is_served_during_recovery_and_survives_restart() -> Result<()> {
    run_restart_fixture(|| async {
        tokio::task::LocalSet::new()
            .run_until(async {
                let (_directory, paths) = recovery_fixture()?;
                let mut gate = gate_device_key(&paths)?;
                let daemon_paths = paths.clone();
                let mut daemon =
                    tokio::task::spawn_local(async move { daemon::run(daemon_paths, true).await });
                assert!(gate.wait_opened().await?, "credential preceded socket");
                let state = connected_status(&paths, &mut daemon).await?;
                assert_eq!(
                    state["recovery"]["state"], "recovering",
                    "no recovery window observed: {state}"
                );
                let stopped = ipc::request(&paths, Command::PrivacyStop).await?;
                assert!(stopped.ok, "privacy stop failed: {:?}", stopped.error);
                let stopped_state = stopped.status.context("stop had no state")?;
                assert!(stopped_state.privacy_stopped);
                let during_stop = serde_json::to_value(stopped_state)?;
                assert_eq!(
                    during_stop["recovery"]["state"], "recovering",
                    "stop waited until recovery finished: {during_stop}"
                );
                gate.release()?;
                std::fs::remove_file(paths.root.join("development-device.key"))?;
                let ready = recovery_finished(&paths).await?;
                assert_eq!(ready["privacy_stopped"], true);
                ipc::request(&paths, Command::Shutdown).await?;
                daemon.await??;

                let daemon_paths = paths.clone();
                let mut restarted =
                    tokio::task::spawn_local(async move { daemon::run(daemon_paths, true).await });
                let state = connected_status(&paths, &mut restarted).await?;
                assert_eq!(state["privacy_stopped"], true);
                let _ = recovery_finished(&paths).await?;
                ipc::request(&paths, Command::Shutdown).await?;
                restarted.await??;
                Ok(())
            })
            .await
    })
}

#[tokio::test(flavor = "current_thread")]
async fn stop_during_environment_preparation_is_durable() -> Result<()> {
    tokio::task::LocalSet::new()
        .run_until(async {
            let (_directory, paths) = recovery_fixture()?;
            let mut gate = gate_device_key(&paths)?;
            let daemon_paths = paths.clone();
            let mut daemon =
                tokio::task::spawn_local(async move { daemon::run(daemon_paths, true).await });
            assert!(
                gate.wait_opened().await?,
                "credential read began before the IPC socket was bound"
            );
            let first = connected_status(&paths, &mut daemon).await?;
            assert!(
                first["recovery"].is_object(),
                "status hid recovery: {first}"
            );
            let preparing = wait_for_step(&paths, "environment").await?;
            assert_eq!(preparing["recovery"]["state"], "recovering");
            let stopped = ipc::request(&paths, Command::PrivacyStop).await?;
            assert!(stopped.status.context("stop had no state")?.privacy_stopped);
            let disk_before_release = Store::open_existing(&paths.store, [0x42; 32])?
                .load_snapshot::<DurableState>()?
                .context("stop was acknowledged before a snapshot existed")?;
            gate.release()?;
            assert!(
                disk_before_release.privacy_stopped(),
                "stop was acknowledged before it was durable"
            );
            let ready = recovery_finished(&paths).await?;
            assert_eq!(ready["privacy_stopped"], true);
            ipc::request(&paths, Command::Shutdown).await?;
            daemon.await??;
            let store = Store::open_existing(&paths.store, [0x42; 32])?;
            let snapshot = store
                .load_snapshot::<DurableState>()?
                .context("saved snapshot missing")?;
            assert!(snapshot.privacy_stopped());
            assert_eq!(snapshot.privacy_stop_by(), Some(StopRequester::User));
            Ok(())
        })
        .await
}

#[tokio::test(flavor = "current_thread")]
async fn a_user_stop_during_recovery_attributes_an_old_unknown_stop() -> Result<()> {
    tokio::task::LocalSet::new()
        .run_until(async {
            let (_directory, paths) = recovery_fixture()?;
            let mut store = Store::open_existing(&paths.store, [0x42; 32])?;
            let mut wire = serde_json::to_value(DurableState::default())?;
            wire["privacy_stopped"] = serde_json::json!(true);
            let old_snapshot: DurableState = serde_json::from_value(wire)?;
            store.save_snapshot(&old_snapshot)?;
            drop(store);

            let mut gate = gate_device_key(&paths)?;
            let daemon_paths = paths.clone();
            let mut daemon =
                tokio::task::spawn_local(async move { daemon::run(daemon_paths, true).await });
            assert!(gate.wait_opened().await?, "credential preceded socket");
            let first = connected_status(&paths, &mut daemon).await?;
            assert_eq!(first["privacy_stopped"], true);
            assert_eq!(first["recovery"]["state"], "recovering");
            let _ = wait_for_step(&paths, "environment").await?;
            let stopped = ipc::request(&paths, Command::PrivacyStop).await?;
            let state = stopped.status.context("stop had no state")?;
            assert_eq!(state.privacy_stop_by, Some(StopRequester::User));
            gate.release()?;
            std::fs::remove_file(paths.root.join("development-device.key"))?;
            let _ = recovery_finished(&paths).await?;
            ipc::request(&paths, Command::Shutdown).await?;
            daemon.await??;
            let store = Store::open_existing(&paths.store, [0x42; 32])?;
            let saved = store
                .load_snapshot::<DurableState>()?
                .context("saved snapshot missing")?;
            assert_eq!(saved.privacy_stop_by(), Some(StopRequester::User));
            Ok(())
        })
        .await
}

#[test]
fn startup_daemon_child() -> Result<()> {
    let Some(root) = std::env::var_os("ORBIT_STARTUP_CHILD_ROOT") else {
        return Ok(());
    };
    let paths = Paths::resolve(Some(root.into()))?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let _ = runtime.block_on(daemon::run(paths, true));
    Ok(())
}

struct ChildDaemon(std::process::Child);

impl Drop for ChildDaemon {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn acknowledged_stop_survives_sigterm_during_preparation() -> Result<()> {
    run_restart_fixture(|| signal_then_restart(false))
}

async fn signal_then_restart(corrupt_stop: bool) -> Result<()> {
    tokio::task::LocalSet::new()
        .run_until(async {
            let (_directory, paths) = recovery_fixture()?;
            let mut gate = gate_device_key(&paths)?;
            let child_log = paths.root.join("fixture-child.log");
            let child_output = OpenOptions::new()
                .create(true)
                .truncate(true)
                .write(true)
                .open(&child_log)?;
            let child = std::process::Command::new(std::env::current_exe()?)
                .arg("--exact")
                .arg("startup_daemon_child")
                .arg("--nocapture")
                .env("ORBIT_STARTUP_CHILD_ROOT", &paths.root)
                .stdout(child_output.try_clone()?)
                .stderr(child_output)
                .spawn()?;
            let mut child = ChildDaemon(child);
            let started = Instant::now();
            loop {
                tokio::select! {
                    opened = gate.wait_opened() => {
                        assert!(opened?, "credential preceded socket");
                        break;
                    }
                    () = tokio::time::sleep(std::time::Duration::from_millis(10)) => {
                        if let Some(outcome) = child.0.try_wait()? {
                            bail!("fixture daemon exited {outcome} before credential read: {}", std::fs::read_to_string(&child_log)?);
                        }
                    }
                }
                if started.elapsed() > std::time::Duration::from_secs(120) {
                    bail!("fixture daemon did not read the credential: {}", std::fs::read_to_string(&child_log)?);
                }
            }
            let preparing = wait_for_step(&paths, "environment").await?;
            assert_eq!(preparing["recovery"]["state"], "recovering");
            let stop = ipc::request(&paths, Command::PrivacyStop).await?;
            assert!(stop.ok);
            // The target PID comes only from the Child this fixture spawned;
            // no process listing, service name or host process is consulted.
            let signal = std::process::Command::new("/bin/kill")
                .arg("-TERM")
                .arg(child.0.id().to_string())
                .status()?;
            assert!(signal.success(), "SIGTERM did not reach the fixture child");
            gate.release()?;
            std::fs::remove_file(paths.root.join("development-device.key"))?;
            let started = Instant::now();
            loop {
                if child.0.try_wait()?.is_some() {
                    break;
                }
                if started.elapsed() > std::time::Duration::from_secs(120) {
                    bail!("fixture daemon did not exit after SIGTERM");
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
            if corrupt_stop {
                let mut store = Store::open_existing(&paths.store, [0x42; 32])?;
                let mut saved = store
                    .load_snapshot::<DurableState>()?
                    .context("fixture stop snapshot absent")?;
                let mut wire = serde_json::to_value(&saved)?;
                wire["privacy_stopped"] = serde_json::json!(false);
                wire["privacy_stop_by"] = serde_json::Value::Null;
                saved = serde_json::from_value(wire)?;
                store.save_snapshot(&saved)?;
            }
            let restarted_paths = paths.clone();
            let mut restarted =
                tokio::task::spawn_local(async move { daemon::run(restarted_paths, true).await });
            let state = connected_status(&paths, &mut restarted).await?;
            let _ = recovery_finished(&paths).await?;
            ipc::request(&paths, Command::Shutdown).await?;
            restarted.await??;
            assert_eq!(state["privacy_stopped"], true, "stop vanished on restart");
            Ok(())
        })
        .await
}

#[tokio::test(flavor = "current_thread")]
async fn listener_bind_failure_names_the_occupied_socket() -> Result<()> {
    let (_directory, paths) = recovery_fixture()?;
    std::fs::write(&paths.socket, b"fixture-occupied-socket")?;
    let error = match daemon::run(paths, true).await {
        Ok(()) => bail!("a regular file did not prevent the IPC bind"),
        Err(error) => error,
    };
    assert!(
        error.to_string().contains("Ruta IPC ocupada"),
        "listener cause was hidden: {error}"
    );
    Ok(())
}

#[test]
fn a_lost_stop_restarts_with_an_assertion() -> Result<()> {
    let mismatch = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        run_restart_fixture(|| signal_then_restart(true))
    }));
    match mismatch {
        Err(panic) => {
            let message = panic
                .downcast_ref::<String>()
                .map(String::as_str)
                .or_else(|| panic.downcast_ref::<&str>().copied())
                .context("lost stop panicked without its assertion")?;
            assert!(message.contains("stop vanished on restart"));
            Ok(())
        }
        Ok(Err(error)) => Err(error),
        Ok(Ok(())) => bail!("the synthetic lost stop did not fail its assertion"),
    }
}

#[tokio::test(flavor = "current_thread")]
async fn embedded_recovery_serves_privacy_stop_without_a_listener() -> Result<()> {
    tokio::task::LocalSet::new()
        .run_until(async {
            let (_directory, paths) = recovery_fixture()?;
            let key_path = paths.root.join("development-device.key");
            let status = std::process::Command::new("mkfifo")
                .arg("-m")
                .arg("600")
                .arg(&key_path)
                .status()?;
            assert!(status.success());
            let (opened, opening) = tokio::sync::oneshot::channel();
            let (release, released) = std::sync::mpsc::channel::<()>();
            let writer = std::thread::spawn(move || -> Result<()> {
                let mut file = OpenOptions::new().write(true).open(key_path)?;
                let _ = opened.send(());
                // Dropping `release` also frees this fixture on an assertion failure.
                let _ = released.recv();
                file.write_all(b"synthetic-device-token")?;
                Ok(())
            });
            let (connected, client) = tokio::sync::oneshot::channel();
            let (ended, end) = tokio::sync::watch::channel(None);
            let daemon_paths = paths.clone();
            let task = tokio::task::spawn_local(async move {
                let result = daemon::run_embedded(daemon_paths, true, None, connected, end).await;
                ended.send_replace(Some(match &result {
                    Ok(()) => ipc::DaemonEnded::Closed,
                    Err(error) => ipc::DaemonEnded::Failed(format!("{error:#}")),
                }));
                result
            });
            // Same 120 s disk jam guard as the recovery fixtures above. Events,
            // rather than elapsed time, establish the blocked phase and the stop.
            let client =
                tokio::time::timeout(std::time::Duration::from_secs(120), client).await??;
            tokio::time::timeout(std::time::Duration::from_secs(120), opening).await??;
            let recovering = client
                .request_command(Command::Status)
                .await?
                .status
                .context("startup status absent")?;
            assert!(matches!(
                recovering.recovery,
                orbit_core::RecoveryStatus::Recovering {
                    step: orbit_core::RecoveryStep::Environment,
                    ..
                }
            ));
            let error = client
                .read_topic(orbit_core::reads::ReadRequest::EngineTools { cursor: None })
                .await
                .err()
                .context("recovering daemon accepted a read")?;
            let refusal = error
                .downcast_ref::<Refusal>()
                .context("recovery lost its typed refusal")?;
            assert!(matches!(
                refusal,
                Refusal::Recovering {
                    step: orbit_core::RecoveryStep::Environment,
                    ..
                }
            ));
            let stop = client.request_command(Command::PrivacyStop).await?;
            assert!(stop.status.context("stop status absent")?.privacy_stopped);
            assert_eq!(
                paths.socket.exists(),
                orbit_core::orbit_bundle::LOCAL_SOCKET
            );
            release.send(())?;
            writer
                .join()
                .map_err(|_| anyhow::anyhow!("fixture key writer panicked"))??;
            tokio::time::timeout(std::time::Duration::from_secs(120), async {
                loop {
                    if client
                        .request_command(Command::Status)
                        .await?
                        .status
                        .context("status absent")?
                        .recovery
                        == orbit_core::RecoveryStatus::Ready
                    {
                        return Ok::<_, anyhow::Error>(());
                    }
                    tokio::task::yield_now().await;
                }
            })
            .await??;
            assert!(
                client
                    .request_command(Command::Status)
                    .await?
                    .status
                    .context("ready status absent")?
                    .privacy_stopped
            );
            client.request_command(Command::Shutdown).await?;
            task.await??;
            let saved = Store::open_existing(&paths.store, [0x42; 32])?
                .load_snapshot::<DurableState>()?
                .context("stop snapshot absent")?;
            assert!(saved.recovered().privacy_stopped());
            Ok(())
        })
        .await
}
