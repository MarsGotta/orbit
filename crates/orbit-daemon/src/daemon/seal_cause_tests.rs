//! A seal that fails says why, with the cause the notice carries: the
//! person is told what to do about a full disk or a folder macOS will not
//! let Orbit write, never only that something failed. Synthetic store only.
use super::*;
use crate::daemon::audio::tests::fixture;
use crate::daemon::capture_lane::seal_cause;
use crate::daemon::meeting_continuity_tests::{frame, start};
use orbit_core::notice::{DataValue, NoticeCode, NoticeKey};
use std::os::unix::fs::PermissionsExt;

fn said_cause(runtime: &Runtime) -> Option<String> {
    let notice = runtime.state.durable.notice.as_ref()?;
    (notice.code == NoticeCode::RecordingSealFailed).then_some(())?;
    match notice.data.get(&NoticeKey::Cause)? {
        DataValue::Token(token) => Some(token.as_str().to_owned()),
        _ => None,
    }
}

#[test]
fn each_failure_has_the_cause_the_person_can_act_on() {
    let io = |kind| anyhow::Error::new(std::io::Error::from(kind));
    assert_eq!(
        seal_cause(&io(std::io::ErrorKind::StorageFull)),
        "disk_full"
    );
    assert_eq!(
        seal_cause(&io(std::io::ErrorKind::QuotaExceeded)),
        "disk_full"
    );
    assert_eq!(
        seal_cause(&io(std::io::ErrorKind::PermissionDenied)),
        "not_writable"
    );
    assert_eq!(seal_cause(&io(std::io::ErrorKind::Other)), "disk_error");
    let store = |error| anyhow::Error::new(error).context("sellado");
    assert_eq!(
        seal_cause(&store(orbit_store::Error::Authentication)),
        "integrity"
    );
    assert_eq!(
        seal_cause(&store(orbit_store::Error::UnsafePath)),
        "destination_invalid"
    );
    assert_eq!(
        seal_cause(&store(orbit_store::Error::NotFound)),
        "store_error"
    );
    let reserve = anyhow::Error::new(capture_lane::ReserveExhausted);
    assert_eq!(seal_cause(&reserve), "reserve_exhausted");
    assert_eq!(
        seal_cause(&anyhow::anyhow!("cola llena")),
        "writer_unavailable"
    );
}

/// The writer cannot create the chunk's file: macOS refuses the folder.
#[tokio::test]
async fn a_seal_the_writer_refuses_is_said_with_its_cause_and_what_it_still_owes() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    let stream = start(&mut runtime, &mut receiver).await?;
    let chunks = runtime.paths.store.join("chunks");
    std::fs::set_permissions(&chunks, std::fs::Permissions::from_mode(0o500))?;
    runtime.handle_native_event(frame(&stream, 0, 8_000)).await;
    let started = std::time::Instant::now();
    while said_cause(&runtime).is_none() && started.elapsed() < Duration::from_secs(30) {
        tokio::time::sleep(Duration::from_millis(10)).await;
        runtime.collect_ready_seals(Duration::ZERO, false)?;
    }
    std::fs::set_permissions(&chunks, std::fs::Permissions::from_mode(0o700))?;
    assert_eq!(said_cause(&runtime).as_deref(), Some("not_writable"));
    let notice = runtime.state.durable.notice.as_ref().context("notice")?;
    assert_eq!(
        notice.data.get(&NoticeKey::Fragments),
        Some(&DataValue::Int(1))
    );
    Ok(())
}
