//! REF-301a: the full daemon and IPC with a blocked key read and a blocked
//! store writer.
use anyhow::{Context, Result, bail};
use base64::{Engine, engine::general_purpose::STANDARD};
use orbit_core::{RecoveryStatus, protocol::Command};
use orbit_daemon::{
    config::{Paths, set_secret},
    daemon, ipc,
};
use std::{fs::OpenOptions, io::Write, os::unix::fs::PermissionsExt, time::Duration};

const STORAGE_KEY: [u8; 32] = [0x42; 32];

async fn ready(paths: &Paths, daemon: &mut tokio::task::JoinHandle<Result<()>>) -> Result<()> {
    // The existing vault fixture measured a 36.4 s disk-bound startup and
    // uses 120 s as a jam guard. The socket and Ready event decide success.
    let guard = tokio::time::sleep(Duration::from_secs(120));
    tokio::pin!(guard);
    loop {
        tokio::select! {
            connected = tokio::net::UnixStream::connect(&paths.socket) => {
                if connected.is_ok() { break; }
            }
            outcome = &mut *daemon => {
                outcome.context("daemon stopped before IPC socket")??;
                bail!("daemon stopped before IPC socket");
            }
            () = &mut guard => bail!("daemon startup jammed before IPC socket"),
        }
        tokio::task::yield_now().await;
    }
    loop {
        tokio::select! {
            response = ipc::request(paths, Command::Status) => {
                if response?.status.context("status missing startup facts")?.recovery == RecoveryStatus::Ready { return Ok(()); }
            }
            outcome = &mut *daemon => {
                outcome.context("daemon stopped during recovery")??;
                bail!("daemon stopped during recovery");
            }
            () = &mut guard => bail!("daemon recovery jammed"),
        }
        tokio::task::yield_now().await;
    }
}

#[tokio::test(flavor = "current_thread")]
async fn stop_does_not_wait_for_a_key_worker() -> Result<()> {
    tokio::task::LocalSet::new()
        .run_until(async {
            let directory = tempfile::tempdir()?;
            let paths = Paths::resolve(Some(directory.path().join("orbit")))?;
            set_secret(&paths, "storage", &STANDARD.encode(STORAGE_KEY), true)?;
            let daemon_paths = paths.clone();
            let mut daemon_task =
                tokio::task::spawn_local(async move { daemon::run(daemon_paths, true).await });
            ready(&paths, &mut daemon_task).await?;

            let key_path = paths.root.join("development-storage.key");
            std::fs::remove_file(&key_path)?;
            let status = std::process::Command::new("mkfifo")
                .arg("-m")
                .arg("600")
                .arg(&key_path)
                .status()?;
            if !status.success() {
                bail!("fixture mkfifo failed: {status}");
            }
            // The FIFO has no writer: opening the key again would block.
            // REF-305 keeps the already authenticated key in the daemon.
            // The 120 s guard still only turns a jam into a named failure;
            // the IPC stop receipt determines success.
            let stopped = tokio::time::timeout(
                Duration::from_secs(120),
                ipc::request(&paths, Command::PrivacyStop),
            )
            .await??;
            assert!(stopped.status.context("stop state absent")?.privacy_stopped);
            ipc::request(&paths, Command::Shutdown).await?;
            daemon_task.await??;
            Ok(())
        })
        .await
}

#[tokio::test(flavor = "current_thread")]
async fn hotkey_latches_before_the_writer_unblocks() -> Result<()> {
    tokio::task::LocalSet::new().run_until(async {
        let directory = tempfile::tempdir()?;
        let paths = Paths::resolve(Some(directory.path().join("orbit")))?;
        set_secret(&paths, "storage", &STANDARD.encode(STORAGE_KEY), true)?;
        let trigger = directory.path().join("hotkey.fifo");
        let status = std::process::Command::new("mkfifo")
            .arg("-m").arg("600").arg(&trigger).status()?;
        if !status.success() { bail!("fixture hotkey fifo failed: {status}"); }
        let helper = directory.path().join("fake-helper.sh");
        std::fs::write(&helper, format!(
            "#!/bin/sh\ncat >/dev/null &\nIFS= read -r line < '{}'\nprintf '%s\\n' '{{\"v\":1,\"event\":\"hotkey\",\"action\":\"privacy_stop\",\"phase\":\"pressed\"}}'\nwait\n",
            trigger.display()
        ))?;
        std::fs::set_permissions(&helper, std::fs::Permissions::from_mode(0o700))?;
        let daemon_paths = paths.clone();
        let mut daemon_task = tokio::task::spawn_local(async move {
            daemon::run_with_helper(daemon_paths, true, Some(helper)).await
        });
        ready(&paths, &mut daemon_task).await?;
        let (_, mut subscription) = ipc::subscribe_topics(&paths, vec![orbit_core::topics::TopicId::Status]).await?;

        let key_path = paths.root.join("development-storage.key");
        std::fs::remove_file(&key_path)?;
        let status = std::process::Command::new("mkfifo")
            .arg("-m").arg("600").arg(&key_path).status()?;
        if !status.success() { bail!("fixture key fifo failed: {status}"); }
        // No FIFO writer is needed: the stop must use the key already held by
        // the daemon, even while the durable writer is blocked below.

        let db_path = paths.store.join("orbit.sqlite3");
        let (locked_tx, locked_rx) = tokio::sync::oneshot::channel();
        let (release_store_tx, release_store_rx) = std::sync::mpsc::channel::<()>();
        let store_holder = std::thread::spawn(move || -> Result<()> {
            let connection = rusqlite::Connection::open(db_path)?;
            connection.execute_batch("BEGIN IMMEDIATE")?;
            locked_tx.send(()).map_err(|_| anyhow::anyhow!("store observer closed"))?;
            release_store_rx.recv().context("store release missing")?;
            connection.execute_batch("ROLLBACK")?;
            Ok(())
        });
        locked_rx.await.context("fixture writer did not hold SQLite")?;
        let trigger_task = tokio::task::spawn_blocking(move || -> Result<()> {
            let mut file = OpenOptions::new().write(true).open(trigger)?;
            file.write_all(b"pressed\n")?;
            Ok(())
        });
        trigger_task.await??;

        // The store still holds the write lock. Observe the local privacy
        // latch before releasing it. An already-admitted local read's early
        // refusal is covered by daemon::ref213_privacy_read_tests.
        // The 120 s guard merely turns a jam into a test failure.
        let guard = tokio::time::sleep(Duration::from_secs(120));
        tokio::pin!(guard);
        loop {
            tokio::select! {
                event = subscription.next_topic() => {
                    let frame = event?;
                    if matches!(frame.data, orbit_core::topics::TopicData::Status(orbit_core::topics::Knowledge::Present { value, .. }) if value.privacy_stopped) {
                        break;
                    }
                }
                () = &mut guard => bail!("hotkey stop did not latch while the writer was blocked"),
            }
        }
        release_store_tx.send(())?;
        store_holder.join().map_err(|_| anyhow::anyhow!("store holder panicked"))??;
        let status = ipc::request(&paths, Command::Status).await?;
        let state = status.status.context("status state absent")?;
        assert!(state.privacy_stopped);
        ipc::request(&paths, Command::Shutdown).await?;
        daemon_task.await??;
        Ok(())
    }).await
}

#[tokio::test(flavor = "current_thread")]
async fn embedded_channel_serves_commands_reads_and_the_same_topic_revisions() -> Result<()> {
    tokio::task::LocalSet::new().run_until(async {
        let directory = tempfile::tempdir()?;
        let paths = Paths::resolve(Some(directory.path().join("orbit")))?;
        paths.initialize()?;
        let retired_socket = std::os::unix::net::UnixListener::bind(&paths.socket)?;
        drop(retired_socket);
        assert!(paths.socket.exists(), "fixture must leave the previous socket path");
        let daemon_paths = paths.clone();
        let (connected, client) = tokio::sync::oneshot::channel();
        let (ended, end) = tokio::sync::watch::channel(None);
        let task = tokio::task::spawn_local(async move {
            let result = daemon::run_embedded(daemon_paths, true, None, connected, end).await;
            ended.send_replace(Some(match &result {
                Ok(()) => ipc::DaemonEnded::Closed,
                Err(error) => ipc::DaemonEnded::Failed(format!("{error:#}")),
            }));
            result
        });
        let client = tokio::time::timeout(Duration::from_secs(120), client).await??;
        tokio::time::timeout(Duration::from_secs(120), async {
            loop {
                let response = client.request_command(Command::Status).await?;
                if response.status.context("startup status absent")?.recovery == RecoveryStatus::Ready {
                    return Ok::<_, anyhow::Error>(());
                }
                tokio::task::yield_now().await;
            }
        }).await??;
        assert_eq!(paths.socket.exists(), orbit_core::orbit_bundle::LOCAL_SOCKET);
        assert!(ipc::lock_instance(&paths).is_err(), "embedded daemon lost daemon.lock");
        let response = client.request_command(Command::PrivacyStop).await?;
        let expected = response.revisions.context("revisions absent")?[&orbit_core::topics::TopicId::Status];
        let mut topics = client.topics();
        let status = topics.get_mut(&orbit_core::topics::TopicId::Status).context("status watch absent")?;
        let frame = status.borrow_and_update().clone();
        assert_eq!(frame.epoch, response.epoch.context("epoch absent")?);
        assert_eq!(frame.revision, expected);
        assert!(matches!(frame.data, orbit_core::topics::TopicData::Status(orbit_core::topics::Knowledge::Present { value, .. }) if value.privacy_stopped));
        client.request_command(Command::Shutdown).await?;
        assert_eq!(client.ended().await, ipc::DaemonEnded::Closed);
        let stopped = client.request_command(Command::Status).await.err().context("closed daemon accepted Status")?;
        assert_eq!(stopped.to_string(), "orbit_closing");
        task.await??;
        assert!(!paths.socket.exists());
        assert!(ipc::lock_instance(&paths).is_ok(), "closure kept daemon.lock");
        Ok(())
    }).await
}
