//! A SIGTERM to the process the daemon runs in closes the daemon in
//! order, as it did when the daemon was a process of its own; the app follows
//! (`lib.rs` exits when the daemon ends unasked). A test binary of its own:
//! the signal is for the whole process, and here it has one daemon only.
use orbit_desktop::daemon_thread::{EmbeddedDaemon, Ended, Launch};
use std::{error::Error, time::Duration};

/// Bounds a hung fixture: the app's readiness wait plus its quit wait.
const FIXTURE_GUARD: Duration = Duration::from_secs(15 + 17);

#[tokio::test(flavor = "multi_thread")]
async fn a_sigterm_closes_the_daemon_in_order() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let paths = orbit_ipc::Paths::resolve(Some(directory.path().to_path_buf()))?;
    let daemon = tokio::time::timeout(
        FIXTURE_GUARD,
        EmbeddedDaemon::start(paths.clone(), Launch::development()),
    )
    .await??;
    tokio::time::timeout(FIXTURE_GUARD, daemon.wait_until_ready()).await??;
    // Past the startup recovery: a signal during it ends the start after the
    // step in hand (the daemon says which), not the closure tested here.
    tokio::time::timeout(FIXTURE_GUARD, async {
        loop {
            let reply = daemon
                .client()
                .await?
                .request_command(orbit_core::protocol::Command::Status)
                .await?;
            if reply
                .status
                .is_some_and(|state| state.recovery == orbit_core::RecoveryStatus::Ready)
            {
                return Ok::<(), Box<dyn Error>>(());
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await??;
    rustix::process::kill_process(rustix::process::getpid(), rustix::process::Signal::TERM)?;
    let ended = tokio::time::timeout(FIXTURE_GUARD, daemon.ended()).await?;
    assert_eq!(ended, Ended::Closed);
    // Nobody asked through the app: this is the end the app follows.
    assert!(!daemon.quitting());
    assert!(!paths.socket.exists(), "the closure removes its socket");
    Ok(())
}
