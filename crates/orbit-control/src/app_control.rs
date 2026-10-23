//! The bounded `ps` census the CUA engine cleanup asks for (`cua.rs`).
//!
//! This module once also closed and opened the app from the terminal; that
//! command line was retired on 2026-10-15 and only the engine census has a caller.
//! Reading the host's process table is an adapter, compiled for macOS only; the
//! suite drives the bounded `ps` runner below with fixture scripts, never with
//! the machine's own processes.

#[cfg(any(target_os = "macos", test))]
use anyhow::{Context, Result};
#[cfg(any(target_os = "macos", test))]
use std::io::{Read, Seek, SeekFrom};
#[cfg(any(target_os = "macos", test))]
use std::process::{Child, ExitStatus, Stdio};
#[cfg(any(target_os = "macos", test))]
use std::time::{Duration, Instant};

/// Census with the parent and user facts needed to decide whether the process
/// on an abandoned CUA socket is this bundle's orphan, read through the bounded
/// `ps` runner below; the pure decision lives in orbit-core.
#[cfg(target_os = "macos")]
pub fn engine_process_listing() -> Result<String> {
    let mut ps = std::process::Command::new("/bin/ps");
    ps.args(["-A", "-o", "pid=,ppid=,uid=,comm="]);
    process_listing_with(&mut ps, orbit_core::process_listing::PS_LISTING_TIMEOUT)
}

#[cfg(any(target_os = "macos", test))]
fn process_listing_with(command: &mut std::process::Command, timeout: Duration) -> Result<String> {
    let (status, bytes) = ps_output_with_deadline(command, timeout)
        .map_err(|error| anyhow::anyhow!("No se pudo enumerar los procesos: {error:#}"))?;
    if !status.success() {
        anyhow::bail!("ps no pudo enumerar los procesos ({status})");
    }
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

#[cfg(any(target_os = "macos", test))]
fn ps_output_with_deadline(
    command: &mut std::process::Command,
    timeout: Duration,
) -> Result<(ExitStatus, Vec<u8>)> {
    // A file lets ps finish even when its listing exceeds a pipe's capacity.
    let mut capture = tempfile::tempfile().context("No se pudo preparar la salida de ps")?;
    let writer = capture
        .try_clone()
        .context("No se pudo copiar la salida de ps")?;
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::from(writer))
        .stderr(Stdio::null())
        .spawn()
        .context("No se pudo iniciar ps")?;
    let status = wait_for_ps(&mut child, timeout)?;
    capture
        .seek(SeekFrom::Start(0))
        .context("No se pudo leer la salida de ps")?;
    let mut bytes = Vec::new();
    capture
        .read_to_end(&mut bytes)
        .context("No se pudo leer la salida de ps")?;
    Ok((status, bytes))
}

#[cfg(any(target_os = "macos", test))]
fn wait_for_ps(child: &mut Child, timeout: Duration) -> Result<ExitStatus> {
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ok(status),
            Ok(None) => {}
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error).context("No se pudo consultar si ps terminó");
            }
        }
        if Instant::now() >= deadline {
            let stopped = child.kill();
            let reaped = child.wait();
            if let Err(error) = stopped {
                anyhow::bail!(
                    "{}; no se pudo detener ps: {error}",
                    orbit_core::process_listing::ps_timeout_message(timeout)
                );
            }
            if let Err(error) = reaped {
                anyhow::bail!(
                    "{}; no se pudo recoger ps: {error}",
                    orbit_core::process_listing::ps_timeout_message(timeout)
                );
            }
            anyhow::bail!(
                "{}",
                orbit_core::process_listing::ps_timeout_message(timeout)
            );
        }
        std::thread::sleep(
            Duration::from_millis(10).min(deadline.saturating_duration_since(Instant::now())),
        );
    }
}

#[cfg(test)]
mod tests {
    // Fixtures assert on known values; the lint would only push the checks into
    // less readable shapes.
    #![allow(clippy::unwrap_used)]
    #![allow(clippy::expect_used)]
    use super::*;

    #[test]
    fn ps_census_timeout_names_its_milliseconds() {
        let fixture = tempfile::tempdir().expect("fixture directory");
        let ps = fixture.path().join("ps-hangs");
        std::fs::write(&ps, "#!/bin/sh\nexec /bin/sleep 5\n").expect("fixture ps");
        let mut command = std::process::Command::new("/bin/sh");
        command.arg(&ps);
        let error = process_listing_with(&mut command, Duration::from_millis(40))
            .expect_err("hung ps must have a deadline");
        assert!(error.to_string().contains("40 ms"), "{error:#}");
    }

    #[test]
    fn ps_census_timeout_reaps_its_fixture() {
        use std::os::unix::process::ExitStatusExt;
        let fixture = tempfile::tempdir().expect("fixture directory");
        let ps = fixture.path().join("ps-hangs");
        std::fs::write(&ps, "#!/bin/sh\nexec /bin/sleep 5\n").expect("fixture ps");
        let mut child = std::process::Command::new("/bin/sh")
            .arg(&ps)
            .spawn()
            .expect("fixture ps");
        let error = wait_for_ps(&mut child, Duration::from_millis(40))
            .expect_err("fixture ps must time out");
        assert!(error.to_string().contains("40 ms"), "{error:#}");
        assert_eq!(
            child
                .try_wait()
                .expect("fixture wait")
                .expect("reaped")
                .signal(),
            Some(9)
        );
    }
}
