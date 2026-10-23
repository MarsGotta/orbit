//! A panic of the helper's supervision: the helper it ran goes with it, the
//! daemon hears a helper that fell, and a new supervision launches a new
//! helper. Past the budget the supervision stays down. Fixture helper
//! scripts only; nothing of macOS.
use super::*;
use crate::supervision::{Exhausted, RESTART_BUDGET};
use tokio::time::timeout;

/// A jam guard for a launch under load, not a measured budget.
const JAM: Duration = Duration::from_secs(30);

/// A helper that says `ready`, counts its launches and reads its input until
/// it closes; the shell keeps its output open meanwhile (an `exec` with a
/// redirection would close it, which reads as a helper that left). Written by `cp`, never held open for writing by this process
/// (`ETXTBSY`, as `native_tests.rs` explains).
fn helper(directory: &std::path::Path) -> Result<(PathBuf, PathBuf)> {
    use std::os::unix::fs::PermissionsExt;
    let ready: Value = serde_json::from_str(include_str!(
        "../../../../../contracts/fixtures/native/events/ready.json"
    ))?;
    let launches = directory.join("launches");
    let source = directory.join("helper.txt");
    let script = directory.join("helper");
    std::fs::write(
        &source,
        format!(
            "#!/bin/sh\n/bin/echo launch >> '{}'\nprintf '%s\\n' '{ready}'\n/bin/cat > /dev/null\n",
            launches.display()
        ),
    )?;
    let copied = std::process::Command::new("/bin/cp")
        .arg(&source)
        .arg(&script)
        .status()?;
    anyhow::ensure!(copied.success(), "cp of the fixture helper failed");
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700))?;
    Ok((script, launches))
}

/// The next event of the helper's route, by name, skipping none.
async fn next(events: &mut mpsc::Receiver<Value>) -> Result<String> {
    let event = timeout(JAM, events.recv())
        .await?
        .context("the supervision's route closed")?;
    Ok(event["event"].as_str().unwrap_or_default().to_owned())
}

fn launches(path: &std::path::Path) -> usize {
    std::fs::read_to_string(path).map_or(0, |text| text.lines().count())
}

#[tokio::test]
async fn a_supervision_that_panics_is_relaunched_with_a_new_helper() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (script, launched) = helper(directory.path())?;
    let (_send, commands) = mpsc::channel(1);
    let (events, mut observed) = mpsc::channel(8);
    let supervision = Supervision::default();
    let helper = Supervisor::spawn(Some(script.clone()), commands, events, supervision.clone());
    assert_eq!(next(&mut observed).await?, "ready");
    std::fs::write(script.with_extension("panic"), "")?;
    assert_eq!(
        next(&mut observed).await?,
        "helper_disconnected",
        "the daemon was not told its helper went with the panic"
    );
    assert_eq!(next(&mut observed).await?, "ready", "no new helper");
    assert_eq!(launches(&launched), 2);
    assert_eq!(supervision.restarts(Actor::HelperSupervisor), 1);
    let end = helper.stop(&mut observed).await;
    assert!(
        matches!(end, HelperEnd::Exited { .. } | HelperEnd::Killed { .. }),
        "the relaunched supervision did not end in order: {end:?}"
    );
    Ok(())
}

#[tokio::test]
async fn a_supervision_past_its_budget_stays_down() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (script, launched) = helper(directory.path())?;
    let (_send, commands) = mpsc::channel(1);
    let (events, mut observed) = mpsc::channel(8);
    let supervision = Supervision::default();
    let helper = Supervisor::spawn(Some(script.clone()), commands, events, supervision.clone());
    assert_eq!(next(&mut observed).await?, "ready");
    for panic in 1..=RESTART_BUDGET + 1 {
        std::fs::write(script.with_extension("panic"), "")?;
        assert_eq!(next(&mut observed).await?, "helper_disconnected");
        if panic <= RESTART_BUDGET {
            assert_eq!(next(&mut observed).await?, "ready");
        }
    }
    assert_eq!(
        supervision.exhausted(),
        Some(Exhausted {
            actor: Actor::HelperSupervisor,
            panics: RESTART_BUDGET + 1,
        })
    );
    assert_eq!(launches(&launched), RESTART_BUDGET + 1);
    assert_eq!(helper.stop(&mut observed).await, HelperEnd::GaveUp);
    Ok(())
}
