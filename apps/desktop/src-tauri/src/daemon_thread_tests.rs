//! On Linux and on the Mac: the real daemon on its thread inside this
//! test process, in development mode over an isolated directory, with no
//! helper and no Keychain. What only an installed bundle can say (the
//! Keychain prompt, TCC) is not here.
use super::*;
use std::error::Error;

/// Bounds a hung fixture, not a speed: the readiness wait the app itself
/// gives plus the quit wait, so a daemon that is merely slow under the whole
/// suite still decides the result by what it answers.
const FIXTURE_GUARD: Duration = Duration::from_secs(15 + 17);

fn isolated() -> Result<(tempfile::TempDir, Paths), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let paths = Paths::resolve(Some(directory.path().to_path_buf()))?;
    Ok((directory, paths))
}

/// Started, answering, and past its startup recovery: until then the daemon
/// refuses `Shutdown` and a signal ends the start instead of closing it, which
/// is what the app does then too, but not what these tests are about.
async fn started(paths: &Paths) -> Result<EmbeddedDaemon, Box<dyn Error>> {
    let daemon = tokio::time::timeout(
        FIXTURE_GUARD,
        EmbeddedDaemon::start(paths.clone(), Launch::development()),
    )
    .await??;
    tokio::time::timeout(FIXTURE_GUARD, daemon.wait_until_ready()).await??;
    tokio::time::timeout(FIXTURE_GUARD, recovered(&daemon)).await??;
    Ok(daemon)
}

/// Waits on the fact itself: the state `Status` serves says the recovery is over.
pub(crate) async fn recovered(daemon: &EmbeddedDaemon) -> Result<(), Box<dyn Error>> {
    loop {
        let reply = daemon
            .client()
            .await?
            .request_command(Command::Status)
            .await?;
        if reply
            .status
            .is_some_and(|state| state.recovery == orbit_core::RecoveryStatus::Ready)
        {
            return Ok(());
        }
        tokio::task::yield_now().await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn the_daemon_answers_status_from_its_thread_in_this_process() -> Result<(), Box<dyn Error>> {
    let (_directory, paths) = isolated()?;
    let daemon = started(&paths).await?;
    // `request_command` fails on a refusal: an answer here is an accepted one.
    daemon
        .client()
        .await?
        .request_command(Command::Status)
        .await?;
    assert_eq!(paths.socket.exists(), orbit_bundle::LOCAL_SOCKET);
    let thread = &daemon.inner.thread;
    assert_eq!(thread.name(), Some(THREAD_NAME));
    assert_ne!(
        thread.id(),
        std::thread::current().id(),
        "not the test's own thread"
    );
    assert!(!daemon.quitting());
    let quit = tokio::time::timeout(FIXTURE_GUARD, daemon.shutdown()).await?;
    assert_eq!(quit, Quit::Ended(Ended::Closed));
    assert!(!paths.socket.exists(), "the closure removes its socket");
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn a_second_start_is_refused_with_its_cause() -> Result<(), Box<dyn Error>> {
    let (_directory, paths) = isolated()?;
    let first = started(&paths).await?;
    let Err(refusal) = EmbeddedDaemon::start(paths.clone(), Launch::development()).await else {
        return Err("a second daemon started beside the one answering".into());
    };
    let refusal = refusal.to_string();
    assert!(
        refusal.contains("An Orbit service is already active or this directory cannot be locked")
            && refusal.contains(&paths.root.join("daemon.lock").display().to_string()),
        "{refusal}"
    );
    // Past that question (two starts that race), the daemon's own lock
    // refuses the second one inside the same process: flock locks belong to
    // an open file, not to a process.
    let second = EmbeddedDaemon::spawn(paths.clone(), Launch::development())?;
    match tokio::time::timeout(FIXTURE_GUARD, second.ended()).await? {
        Ended::Failed(cause) => assert!(
            cause.contains("An Orbit service is already active or this directory cannot be locked"),
            "{cause}"
        ),
        other => return Err(format!("the second daemon ended as {other:?}").into()),
    }
    first
        .client()
        .await?
        .request_command(Command::Status)
        .await?;
    let quit = tokio::time::timeout(FIXTURE_GUARD, first.shutdown()).await?;
    assert_eq!(quit, Quit::Ended(Ended::Closed));
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn quitting_twice_asks_once_and_both_see_the_thread_end_in_time() -> Result<(), Box<dyn Error>>
{
    let (_directory, paths) = isolated()?;
    let daemon = started(&paths).await?;
    let started_at = Instant::now();
    // The tray's quit and the `ExitRequested` its `exit` fires, at once.
    let (first, second) = tokio::time::timeout(FIXTURE_GUARD, async {
        tokio::join!(daemon.shutdown(), daemon.shutdown())
    })
    .await?;
    let waited = started_at.elapsed();
    assert_eq!(first, Quit::Ended(Ended::Closed));
    assert_eq!(second, Quit::Ended(Ended::Closed));
    assert!(waited < DAEMON_EXIT_WAIT, "{waited:?}");
    assert!(daemon.quitting());
    // And once more after the end, as `exit` does: nothing left to wait for.
    let again = tokio::time::timeout(Duration::from_secs(1), daemon.shutdown()).await?;
    assert_eq!(again, Quit::Ended(Ended::Closed));
    assert!(!paths.socket.exists());
    Ok(())
}

#[test]
fn the_launch_follows_the_executable() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let bundle = directory.path().join(orbit_bundle::NAME);
    let app = orbit_bundle::member_path(&bundle, Member::App).ok_or("app route")?;
    let helper = orbit_bundle::member_path(&bundle, Member::Helper).ok_or("helper route")?;
    std::fs::create_dir_all(app.parent().ok_or("MacOS directory")?)?;
    std::fs::write(&app, b"synthetic app")?;

    let missing = Launch::for_executable(&app)
        .err()
        .ok_or("a bundle without its helper was accepted")?
        .to_string();
    assert!(
        missing.contains("no contiene el helper esperado")
            && missing.contains(&helper.display().to_string()),
        "{missing}"
    );

    std::fs::write(&helper, b"synthetic helper")?;
    assert_eq!(
        Launch::for_executable(&app)?,
        Launch {
            development: false,
            helper: Some(helper),
        }
    );

    let renamed = directory.path().join("Orbit 2.app/Contents/MacOS/Orbit");
    let refusal = Launch::for_executable(&renamed)
        .err()
        .ok_or("a renamed bundle was accepted")?
        .to_string();
    assert!(refusal.contains("no es el bundle esperado"), "{refusal}");

    assert_eq!(
        Launch::for_executable(Path::new("/build/target/debug/orbit-desktop"))?,
        Launch::development()
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_app_can_acknowledge_a_privacy_stop_during_startup_recovery()
-> Result<(), Box<dyn Error>> {
    let (_directory, paths) = isolated()?;
    paths.initialize()?;
    let fifo = paths.root.join("development-device.key");
    let status = std::process::Command::new("mkfifo")
        .arg("-m")
        .arg("600")
        .arg(&fifo)
        .status()?;
    assert!(status.success());
    let (opened, opening) = tokio::sync::oneshot::channel();
    let (release, released) = std::sync::mpsc::channel::<()>();
    let writer = std::thread::spawn(move || -> std::io::Result<()> {
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new().write(true).open(fifo)?;
        let _ = opened.send(());
        let _ = released.recv();
        file.write_all(b"synthetic-device-token")
    });
    let daemon = EmbeddedDaemon::start(paths.clone(), Launch::development()).await?;
    tokio::time::timeout(FIXTURE_GUARD, opening).await??;
    tokio::time::timeout(FIXTURE_GUARD, daemon.wait_until_ready()).await??;
    let transport = daemon.client().await?;
    let status = transport
        .request_command(Command::Status)
        .await?
        .status
        .ok_or("status absent")?;
    assert!(matches!(
        status.recovery,
        orbit_core::RecoveryStatus::Recovering { .. }
    ));
    let app = orbit_app_core::DaemonClient::connect(paths, transport.clone());
    let receipt = app
        .send(orbit_app_core::AppCommand::Daemon(Box::new(
            Command::PrivacyStop,
        )))
        .await?;
    assert_eq!(
        receipt.delivery, None,
        "a startup acknowledgement must not claim an unobserved frame"
    );
    assert!(
        transport
            .request_command(Command::Status)
            .await?
            .status
            .ok_or("stop status absent")?
            .privacy_stopped
    );
    release.send(())?;
    writer.join().map_err(|_| "credential writer panicked")??;
    tokio::time::timeout(FIXTURE_GUARD, recovered(&daemon)).await??;
    assert_eq!(daemon.shutdown().await, Quit::Ended(Ended::Closed));
    Ok(())
}

/// A daemon that falls after a minute relaunches the app, which resumes the
/// recording from its snapshot; one that falls in its start does not loop.
#[test]
fn a_daemon_that_falls_after_its_start_relaunches_the_app() {
    let fell = Ended::Failed("el bucle falló".into());
    assert!(!relaunches(&fell, Duration::from_secs(5)));
    assert!(!relaunches(
        &fell,
        DAEMON_RELAUNCH_MIN_UPTIME - Duration::from_millis(1)
    ));
    assert!(relaunches(&fell, DAEMON_RELAUNCH_MIN_UPTIME));
    assert!(relaunches(&Ended::Vanished, Duration::from_secs(25 * 60)));
}

/// A SIGTERM closes the daemon in order, and the app ends with it: a test
/// harness or the system that sent it asked Orbit to end, and a relaunch
/// would bring back an app nobody wanted (seen on the Mac, 2026-10-23).
#[test]
fn a_daemon_closed_in_order_ends_the_app_and_never_relaunches_it() {
    assert!(!relaunches(&Ended::Closed, Duration::from_secs(25 * 60)));
}
