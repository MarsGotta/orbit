//! `orbit`: the daemon as a process of its own, for development and tests.
//!
//! Since SIM-51 Orbit.app runs the daemon on a thread of its own process
//! (`apps/desktop/src-tauri/src/daemon_thread.rs`) and this binary is not
//! shipped. The hub's e2e, app-core's tests and the daemon's process tests
//! start it as `orbit [--dev] [--data-dir DIR] daemon [--helper PATH]
//! [--parent-pid PID]`: that grammar is the whole interface (the command line
//! was retired on 2026-10-15, SIM-01).
use std::{ffi::OsString, path::PathBuf, process::ExitCode, time::Duration};

const USAGE: &str =
    "uso: orbit [--dev] [--data-dir DIRECTORIO] daemon [--helper RUTA] [--parent-pid PID]";

#[derive(Debug, Default, PartialEq, Eq)]
struct Launch {
    development: bool,
    data_dir: Option<PathBuf>,
    helper: Option<PathBuf>,
    parent_pid: Option<rustix::process::Pid>,
}

/// Reads `[--dev] [--data-dir DIR] daemon [--helper PATH] [--parent-pid N]`.
/// Anything else is refused by name, never ignored: a launcher that passes an
/// argument expects it to mean something.
fn parse(arguments: impl IntoIterator<Item = OsString>) -> Result<Launch, String> {
    let mut launch = Launch::default();
    let mut arguments = arguments.into_iter();
    let mut daemon = false;
    while let Some(argument) = arguments.next() {
        let mut value = |name: &str| {
            arguments
                .next()
                .ok_or_else(|| format!("falta el valor de {name}"))
        };
        match (daemon, argument.to_str()) {
            (false, Some("--dev")) => launch.development = true,
            (false, Some("--data-dir")) => {
                launch.data_dir = Some(PathBuf::from(value("--data-dir")?));
            }
            (false, Some("daemon")) => daemon = true,
            (true, Some("--helper")) => launch.helper = Some(PathBuf::from(value("--helper")?)),
            (true, Some("--parent-pid")) => {
                let raw = value("--parent-pid")?;
                let pid = raw
                    .to_str()
                    .and_then(|text| text.parse::<u32>().ok())
                    .ok_or_else(|| {
                        format!(
                            "--parent-pid necesita un número de proceso y recibió {}",
                            raw.to_string_lossy()
                        )
                    })?;
                launch.parent_pid = Some(
                    i32::try_from(pid)
                        .ok()
                        .and_then(rustix::process::Pid::from_raw)
                        .ok_or_else(|| {
                            format!("--parent-pid debe identificar un proceso y recibió {pid}")
                        })?,
                );
            }
            _ => {
                return Err(format!(
                    "argumento no reconocido: {}",
                    argument.to_string_lossy()
                ));
            }
        }
    }
    if !daemon {
        return Err("falta la orden daemon".into());
    }
    Ok(launch)
}

/// How often the binary looks for the process given by `--parent-pid`. Half a
/// second is the delay that adds to its closure after that process is gone.
const PARENT_POLL: Duration = Duration::from_millis(500);

/// `--parent-pid`: a test that starts this binary and dies (a runner killed by
/// its own timeout, a crashed pytest) does not leave an `orbit` daemon running
/// on the machine that ran it (AGENTS.md: a test has no effects on that
/// machine). The hub's e2e (`server/tests/support_client_e2e.py`) and the
/// daemon's process tests (`tests/support/helper_relay.rs`) pass it. When that process is gone the
/// binary sends itself SIGTERM, which the daemon hears as any other SIGTERM:
/// the same orderly closure, with the same budget. The app never passes it;
/// its daemon is a thread of its own process.
async fn follow_parent(parent: rustix::process::Pid) {
    let mut poll = tokio::time::interval(PARENT_POLL);
    poll.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        poll.tick().await;
        if matches!(
            rustix::process::test_kill_process(parent),
            Err(rustix::io::Errno::SRCH)
        ) {
            if let Err(error) = rustix::process::kill_process(
                rustix::process::getpid(),
                rustix::process::Signal::TERM,
            ) {
                eprintln!(
                    "orbit: el proceso {parent:?} ya no existe y no se pudo cerrar el daemon: {error}"
                );
            }
            return;
        }
    }
}

/// Errors leave through `main`'s ordinary return, never `std::process::exit`:
/// returning drops the runtime and with it every task, so a supervised child
/// such as the native helper is killed by its own handle instead of being left
/// orphaned with the global shortcuts still registered (REF-06 A4).
#[tokio::main]
async fn main() -> ExitCode {
    let launch = match parse(std::env::args_os().skip(1)) {
        Ok(launch) => launch,
        Err(problem) => {
            eprintln!("orbit: {problem}\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    if let Some(parent) = launch.parent_pid {
        tokio::spawn(follow_parent(parent));
    }
    let result = match orbit_daemon::config::Paths::resolve(launch.data_dir) {
        Ok(paths) => {
            orbit_daemon::daemon::run_with_helper(paths, launch.development, launch.helper).await
        }
        Err(error) => Err(error),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("orbit: {error:#}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(arguments: &[&str]) -> Result<Launch, String> {
        parse(arguments.iter().map(OsString::from))
    }

    #[test]
    fn the_two_launches_the_tests_make_are_understood() {
        assert_eq!(
            parsed(&[
                "daemon",
                "--helper",
                "/Applications/Orbit.app/Contents/MacOS/OrbitNative",
                "--parent-pid",
                "4242",
            ]),
            Ok(Launch {
                helper: Some("/Applications/Orbit.app/Contents/MacOS/OrbitNative".into()),
                parent_pid: rustix::process::Pid::from_raw(4242),
                ..Launch::default()
            })
        );
        assert_eq!(
            parsed(&[
                "--dev",
                "--data-dir",
                "/tmp/orbit-dev",
                "daemon",
                "--parent-pid",
                "7"
            ]),
            Ok(Launch {
                development: true,
                data_dir: Some("/tmp/orbit-dev".into()),
                parent_pid: rustix::process::Pid::from_raw(7),
                ..Launch::default()
            })
        );
    }

    #[test]
    fn anything_else_is_refused_by_name() {
        assert_eq!(parsed(&[]), Err("falta la orden daemon".into()));
        assert_eq!(
            parsed(&["status", "--json"]),
            Err("argumento no reconocido: status".into())
        );
        assert_eq!(
            parsed(&["daemon", "--dev"]),
            Err("argumento no reconocido: --dev".into())
        );
        assert_eq!(
            parsed(&["daemon", "--parent-pid", "uno"]),
            Err("--parent-pid necesita un número de proceso y recibió uno".into())
        );
        assert_eq!(
            parsed(&["daemon", "--parent-pid", "0"]),
            Err("--parent-pid debe identificar un proceso y recibió 0".into())
        );
        assert_eq!(
            parsed(&["--data-dir"]),
            Err("falta el valor de --data-dir".into())
        );
    }
}
