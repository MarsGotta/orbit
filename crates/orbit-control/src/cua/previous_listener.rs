//! The engine a previous daemon left listening on Orbit's socket. A socket is
//! not by itself proof that its listener is our engine: a bounded census must
//! prove the exact bundle, user and orphaned ancestry before any signal, and
//! the census after the signal says which engines really stopped. Anything
//! less retires only the pathname. macOS only; the tests drive it with
//! synthetic censuses.
use anyhow::Result;
use orbit_core::orbit_bundle::{self, Member};
use orbit_core::process_listing::{
    self, EngineListenerDecision, EngineSweepTarget, KILL_OBSERVATION_WAIT, PS_LISTING_TIMEOUT,
};
use orbit_core::topics::control::{EngineCleanup, EngineCleanupOutcome, EngineCleanupReason};
use std::path::{Path, PathBuf};

pub(super) async fn inspect_previous_socket<L, F, K, S, SF>(
    binary: &Path,
    socket: &Path,
    get_listing: L,
    kill_process: K,
    sleep: S,
) -> ListenerInspection
where
    L: Fn() -> F,
    F: std::future::Future<Output = Result<String>>,
    K: Fn(rustix::process::Pid, rustix::process::Signal) -> Result<(), rustix::io::Errno>,
    S: Fn(std::time::Duration) -> SF,
    SF: std::future::Future<Output = ()>,
{
    let stream = match tokio::net::UnixStream::connect(socket).await {
        Ok(stream) => stream,
        Err(_) => {
            return ListenerInspection {
                cleanups: Vec::new(),
                notice: None,
            };
        }
    };
    let credential = match stream.peer_cred() {
        Ok(credential) => credential,
        Err(error) => {
            return ListenerInspection::skipped(format!(
                "No se pudo identificar al proceso del socket CUA previo en {}: {error}; no se envió ninguna señal",
                socket.display()
            ));
        }
    };
    let Some(peer) = credential.pid() else {
        return ListenerInspection::skipped(format!(
            "El socket CUA previo en {} no comunicó el PID de su proceso; no se envió ninguna señal",
            socket.display()
        ));
    };
    inspect_previous_listener(
        binary,
        socket,
        ListenerIdentity {
            peer,
            peer_uid: credential.uid(),
            own_uid: rustix::process::geteuid().as_raw(),
        },
        get_listing,
        kill_process,
        sleep,
    )
    .await
}

pub(super) struct ListenerIdentity {
    pub(super) peer: i32,
    pub(super) peer_uid: u32,
    pub(super) own_uid: u32,
}

pub(super) async fn inspect_previous_listener<L, F, K, S, SF>(
    binary: &Path,
    socket: &Path,
    identity: ListenerIdentity,
    get_listing: L,
    kill_process: K,
    sleep: S,
) -> ListenerInspection
where
    L: Fn() -> F,
    F: std::future::Future<Output = Result<String>>,
    K: Fn(rustix::process::Pid, rustix::process::Signal) -> Result<(), rustix::io::Errno>,
    S: Fn(std::time::Duration) -> SF,
    SF: std::future::Future<Output = ()>,
{
    let own_uid = identity.own_uid;
    let (peer_pid, bundle) = match identify_listener(binary, socket, identity) {
        Ok(identified) => identified,
        Err(skipped) => return skipped,
    };
    let listing = match census(socket, peer_pid, &get_listing).await {
        Ok(listing) => listing,
        Err(skipped) => return skipped,
    };
    match process_listing::decide_engine_listener(&listing, peer_pid, own_uid, &bundle) {
        EngineListenerDecision::Keep(reason) => {
            eprintln!(
                "The previous CUA socket ({}) is held by PID {peer_pid}; the census does not authorise sweeping it ({reason}), so the path is removed without signalling it",
                socket.display()
            );
            ListenerInspection::skipped(format!(
                "No se barrió el PID {peer_pid} del socket CUA previo: {reason}; no se envió ninguna señal"
            ))
        }
        EngineListenerDecision::Sweep(targets) => {
            let (mut cleanups, killed_targets) = signal_orphans(targets, kill_process);
            if !killed_targets.is_empty() {
                cleanups.extend(confirm_stops(killed_targets, own_uid, &get_listing, sleep).await);
            }
            ListenerInspection {
                cleanups,
                notice: None,
            }
        }
    }
}

/// The listener's PID and the bundle whose engine Orbit runs, when the
/// listener is a valid PID of Orbit's own user and the binary is the engine of
/// an Orbit bundle; otherwise the inspection ends here, without a signal.
fn identify_listener(
    binary: &Path,
    socket: &Path,
    identity: ListenerIdentity,
) -> Result<(u32, PathBuf), ListenerInspection> {
    let ListenerIdentity {
        peer,
        peer_uid,
        own_uid,
    } = identity;
    let Ok(peer_pid) = u32::try_from(peer) else {
        eprintln!(
            "The previous CUA socket ({}) reported the invalid PID {peer}; removing the path without signalling any process",
            socket.display()
        );
        return Err(ListenerInspection::skipped(format!(
            "El socket CUA previo informó el PID inválido {peer}; no se envió ninguna señal"
        )));
    };
    if peer_uid != own_uid {
        eprintln!(
            "The previous CUA socket ({}) is held by PID {peer_pid} of user {peer_uid}, not Orbit's user {own_uid}; removing the path without signalling it",
            socket.display()
        );
        return Err(ListenerInspection::skipped(format!(
            "El socket CUA previo escucha al PID {peer_pid} del usuario {peer_uid}, no al usuario {own_uid}; no se envió ninguna señal"
        )));
    }
    let Some(bundle) = orbit_bundle::bundle_of(binary, Member::Engine) else {
        eprintln!(
            "The previous CUA socket ({}) is held by PID {peer_pid}, but {} is not the engine of an Orbit bundle; removing the path without signalling it",
            socket.display(),
            binary.display()
        );
        return Err(ListenerInspection::skipped(format!(
            "El socket CUA previo escucha al PID {peer_pid}, pero {} no es un motor de un paquete Orbit; no se envió ninguna señal",
            binary.display()
        )));
    };
    Ok((peer_pid, bundle))
}

/// The census the sweep decides on, within its own deadline. A census that
/// fails or is late ends the inspection without a signal.
async fn census<L, F>(
    socket: &Path,
    peer_pid: u32,
    get_listing: &L,
) -> Result<String, ListenerInspection>
where
    L: Fn() -> F,
    F: std::future::Future<Output = Result<String>>,
{
    match tokio::time::timeout(PS_LISTING_TIMEOUT, get_listing()).await {
        Ok(Ok(listing)) => Ok(listing),
        Ok(Err(error)) => {
            eprintln!(
                "The previous CUA socket ({}) is held by PID {peer_pid}, but its identity could not be checked: {error:#}; removing the path without signalling it",
                socket.display()
            );
            Err(ListenerInspection::skipped(format!(
                "No se pudo censar el PID {peer_pid} del socket CUA previo: {error:#}; no se envió ninguna señal"
            )))
        }
        Err(_) => Err(ListenerInspection::skipped(format!(
            "El censo del PID {peer_pid} del socket CUA previo venció a los {} ms; no se envió ninguna señal",
            PS_LISTING_TIMEOUT.as_millis()
        ))),
    }
}

/// Sends SIGKILL to each orphan. Returns the receipts already settled (a PID
/// the system cannot name, an engine already gone, a failed signal) and the
/// engines signalled, whose stop the census has yet to confirm.
fn signal_orphans<K>(
    targets: Vec<EngineSweepTarget>,
    kill_process: K,
) -> (Vec<EngineCleanup>, Vec<EngineSweepTarget>)
where
    K: Fn(rustix::process::Pid, rustix::process::Signal) -> Result<(), rustix::io::Errno>,
{
    let mut cleanups = Vec::new();
    let mut killed_targets = Vec::new();
    for target in targets {
        let Some(pid) = i32::try_from(target.pid)
            .ok()
            .and_then(rustix::process::Pid::from_raw)
        else {
            eprintln!(
                "The PID {} of the orphaned CUA engine at {} cannot be represented; removing the path without signalling it",
                target.pid,
                target.executable.display()
            );
            cleanups.push(orphan_cleanup(
                &target,
                EngineCleanupOutcome::SignalFailed,
                Some("el PID no cabe en el identificador de proceso del sistema".into()),
            ));
            continue;
        };
        eprintln!(
            "Warning: sweeping CUA engine PID {} at {} because it outlived its daemon and no longer descends from any live daemon of the bundle",
            target.pid,
            target.executable.display()
        );
        match kill_process(pid, rustix::process::Signal::KILL) {
            Err(rustix::io::Errno::SRCH) => {
                cleanups.push(orphan_cleanup(&target, EngineCleanupOutcome::Stopped, None));
            }
            Err(error) => {
                eprintln!(
                    "Warning: CUA engine PID {} at {} is still running because SIGKILL failed: {error}",
                    target.pid,
                    target.executable.display()
                );
                cleanups.push(orphan_cleanup(
                    &target,
                    EngineCleanupOutcome::SignalFailed,
                    Some(error.to_string()),
                ));
            }
            Ok(()) => {
                killed_targets.push(target);
            }
        }
    }
    (cleanups, killed_targets)
}

/// The receipt of each engine signalled, from the last census after the
/// signal: still listed is a survivor, absent is stopped, and no census at
/// all is unconfirmed, with its cause.
async fn confirm_stops<L, F, S, SF>(
    killed_targets: Vec<EngineSweepTarget>,
    own_uid: u32,
    get_listing: &L,
    sleep: S,
) -> Vec<EngineCleanup>
where
    L: Fn() -> F,
    F: std::future::Future<Output = Result<String>>,
    S: Fn(std::time::Duration) -> SF,
    SF: std::future::Future<Output = ()>,
{
    let (last_observation, observation_error) =
        observe_after_kill(&killed_targets, own_uid, get_listing, sleep).await;
    let mut cleanups = Vec::new();
    for target in killed_targets {
        let outcome = last_observation
            .as_ref()
            .map(|listing| observed_cleanup_outcome(listing, &target, own_uid));
        let (out, fail) = match outcome {
            Some(ObservedCleanupOutcome::Survived) => {
                eprintln!(
                    "Warning: CUA engine PID {} at {} survived SIGKILL; it is not claimed to have stopped",
                    target.pid,
                    target.executable.display()
                );
                (EngineCleanupOutcome::Survived, None)
            }
            Some(ObservedCleanupOutcome::Stopped) => {
                eprintln!(
                    "Confirmed the sweep of CUA engine PID {} at {}: reason, orphaned from its daemon",
                    target.pid,
                    target.executable.display()
                );
                (EngineCleanupOutcome::Stopped, None)
            }
            None => (
                EngineCleanupOutcome::Unconfirmed,
                Some(
                    observation_error
                        .clone()
                        .unwrap_or_else(|| "no llegó ningún censo posterior a SIGKILL".into()),
                ),
            ),
        };
        cleanups.push(orphan_cleanup(&target, out, fail));
    }
    cleanups
}

/// Censuses after the signal, until every engine signalled is gone or the
/// wait ends: the last census taken, or why none could be.
async fn observe_after_kill<L, F, S, SF>(
    killed_targets: &[EngineSweepTarget],
    own_uid: u32,
    get_listing: &L,
    sleep: S,
) -> (Option<String>, Option<String>)
where
    L: Fn() -> F,
    F: std::future::Future<Output = Result<String>>,
    S: Fn(std::time::Duration) -> SF,
    SF: std::future::Future<Output = ()>,
{
    let start = std::time::Instant::now();
    let mut last_observation = None;
    let mut observation_error = None;
    while start.elapsed() < KILL_OBSERVATION_WAIT {
        sleep(std::time::Duration::from_millis(10)).await;
        match get_listing().await {
            Ok(listing) => {
                let all_stopped = killed_targets.iter().all(|t| {
                    observed_cleanup_outcome(&listing, t, own_uid)
                        == ObservedCleanupOutcome::Stopped
                });
                last_observation = Some(listing);
                if all_stopped {
                    break;
                }
            }
            Err(error) => {
                observation_error = Some(error.to_string());
                last_observation = None;
                break;
            }
        }
    }
    (last_observation, observation_error)
}

fn orphan_cleanup(
    target: &EngineSweepTarget,
    outcome: EngineCleanupOutcome,
    failure: Option<String>,
) -> EngineCleanup {
    EngineCleanup {
        pid: target.pid,
        executable: target.executable.display().to_string(),
        reason: EngineCleanupReason::OrphanedFromDaemon,
        outcome,
        failure,
    }
}

pub(super) struct ListenerInspection {
    pub(super) cleanups: Vec<EngineCleanup>,
    pub(super) notice: Option<String>,
}

impl ListenerInspection {
    fn skipped(notice: String) -> Self {
        Self {
            cleanups: Vec::new(),
            notice: Some(notice),
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ObservedCleanupOutcome {
    Stopped,
    Survived,
}

pub(super) fn observed_cleanup_outcome(
    listing: &str,
    target: &EngineSweepTarget,
    own_uid: u32,
) -> ObservedCleanupOutcome {
    if process_listing::tree_entries(listing)
        .into_iter()
        .any(|entry| {
            entry.pid == target.pid
                && entry.user_id == own_uid
                && Path::new(entry.executable) == target.executable
        })
    {
        ObservedCleanupOutcome::Survived
    } else {
        ObservedCleanupOutcome::Stopped
    }
}

#[cfg(target_os = "macos")]
pub(super) async fn engine_process_listing() -> Result<String> {
    use anyhow::Context;
    tokio::task::spawn_blocking(crate::app_control::engine_process_listing)
        .await
        .context("No se pudo recoger la tarea del censo de motores CUA")?
}
