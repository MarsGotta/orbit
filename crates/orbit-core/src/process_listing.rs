//! Pure parsing and selection over a supplied process listing. Callers own
//! enumeration and signals; tests use only synthetic text.
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    time::Duration,
};

use crate::orbit_bundle::{self, Member};

/// Bounds one synchronous process census (`ps`) of the daemon: the old engine
/// listener's ancestry at startup and the meeting applications. Two seconds
/// allow a short local census; it was first sized against the app's quit
/// wait, whose census left when the daemon moved inside the app. It is not a measured macOS `ps`
/// percentile or grace for an engine to exit.
pub const PS_LISTING_TIMEOUT: Duration = Duration::from_secs(2);

/// Observation window after SIGKILL, shared by the CLI and
/// startup cleanup. It bounds how long they wait to report whether the exact
/// process is still listed. 500 ms was chosen as a bounded observation
/// window; no macOS disappearance distribution has been measured for it.
/// It is not a grace period before signaling or proof of an orderly shutdown.
pub const KILL_OBSERVATION_WAIT: Duration = Duration::from_millis(500);

/// The timeout report of a census bounded by [`PS_LISTING_TIMEOUT`].
pub fn ps_timeout_message(timeout: Duration) -> String {
    format!("ps no respondió en {} ms", timeout.as_millis())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProcessTreeEntry<'a> {
    pub pid: u32,
    pub parent_pid: u32,
    pub user_id: u32,
    pub executable: &'a str,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EngineSweepTarget {
    pub pid: u32,
    pub executable: PathBuf,
    pub reason: EngineSweepReason,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EngineSweepReason {
    OrphanedFromDaemon,
}

impl std::fmt::Display for EngineKeepReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ListenerNotListed => write!(f, "el proceso no aparece en el listado"),
            Self::DifferentUser { expected, found } => {
                write!(f, "pertenece al usuario {found}, no a {expected}")
            }
            Self::DifferentExecutable { expected, found } => write!(
                f,
                "su ejecutable es {}, no {}",
                found.display(),
                expected.display()
            ),
            Self::LiveDaemonAncestor { pid } => {
                write!(f, "su daemon sigue vivo dentro de la app (PID {pid})")
            }
            Self::IncompleteAncestry { pid, parent_pid } => write!(
                f,
                "el ancestro {parent_pid} de {pid} no aparece en el listado"
            ),
            Self::CyclicAncestry { pid } => write!(f, "el ancestro {pid} forma un ciclo"),
            Self::MissingBundleMember(member) => {
                write!(f, "falta {} en el paquete", member.relative_path())
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EngineKeepReason {
    MissingBundleMember(orbit_bundle::Member),
    ListenerNotListed,
    DifferentUser { found: u32, expected: u32 },
    DifferentExecutable { found: PathBuf, expected: PathBuf },
    LiveDaemonAncestor { pid: u32 },
    IncompleteAncestry { pid: u32, parent_pid: u32 },
    CyclicAncestry { pid: u32 },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EngineListenerDecision {
    Sweep(Vec<EngineSweepTarget>),
    Keep(EngineKeepReason),
}

/// Parse the macOS tree census used for an old socket listener. Invalid lines
/// remain absent facts; callers must not turn that absence into permission to
/// signal a process.
pub fn tree_entries(listing: &str) -> Vec<ProcessTreeEntry<'_>> {
    listing
        .lines()
        .filter_map(|line| {
            let (pid, rest) = line.trim_start().split_once(char::is_whitespace)?;
            let (parent_pid, rest) = rest.trim_start().split_once(char::is_whitespace)?;
            let (user_id, executable) = rest.trim_start().split_once(char::is_whitespace)?;
            Some(ProcessTreeEntry {
                pid: pid.parse::<u32>().ok()?,
                parent_pid: parent_pid.parse::<u32>().ok()?,
                user_id: user_id.parse::<u32>().ok()?,
                executable: executable.trim(),
            })
        })
        .filter(|entry| entry.pid != 0)
        .collect()
}

/// Decide whether the process listening on an old CUA socket is an orphan from
/// this exact package. A missing row or ancestry link is unknown and keeps the
/// listener alive; only a chain known to end at launchd without reaching a live
/// daemon authorizes the sweep. The daemon runs inside the app, so a
/// live daemon is a live `Contents/MacOS/Orbit` of this bundle among the
/// ancestors.
pub fn decide_engine_listener(
    listing: &str,
    listener_pid: u32,
    own_user_id: u32,
    bundle: &Path,
) -> EngineListenerDecision {
    let expected_engine = match orbit_bundle::member_path(bundle, Member::Engine) {
        Some(path) => path,
        None => {
            return EngineListenerDecision::Keep(EngineKeepReason::MissingBundleMember(
                Member::Engine,
            ));
        }
    };
    let expected_daemon = match orbit_bundle::member_path(bundle, Member::App) {
        Some(path) => path,
        None => {
            return EngineListenerDecision::Keep(EngineKeepReason::MissingBundleMember(
                Member::App,
            ));
        }
    };
    let entries: BTreeMap<u32, ProcessTreeEntry<'_>> = tree_entries(listing)
        .into_iter()
        .map(|entry| (entry.pid, entry))
        .collect();
    let Some(listener) = entries.get(&listener_pid).copied() else {
        return EngineListenerDecision::Keep(EngineKeepReason::ListenerNotListed);
    };
    if listener.user_id != own_user_id {
        return EngineListenerDecision::Keep(EngineKeepReason::DifferentUser {
            found: listener.user_id,
            expected: own_user_id,
        });
    }
    if Path::new(listener.executable) != expected_engine {
        return EngineListenerDecision::Keep(EngineKeepReason::DifferentExecutable {
            found: PathBuf::from(listener.executable),
            expected: expected_engine,
        });
    }

    decide_by_ancestry(
        &entries,
        listener,
        own_user_id,
        &expected_engine,
        &expected_daemon,
    )
}

/// Walks the listener's ancestors: an orphan of this bundle (its chain reaches
/// launchd without a live daemon of the bundle) is swept with the engines of
/// the bundle above it; a live daemon, a missing link or a cycle keeps it.
fn decide_by_ancestry(
    entries: &BTreeMap<u32, ProcessTreeEntry<'_>>,
    listener: ProcessTreeEntry<'_>,
    own_user_id: u32,
    expected_engine: &Path,
    expected_daemon: &Path,
) -> EngineListenerDecision {
    let mut current = listener;
    let mut visited = BTreeSet::from([listener.pid]);
    let mut bundle_parents = Vec::<ProcessTreeEntry<'_>>::new();
    loop {
        if current.parent_pid <= 1 {
            let mut targets = vec![EngineSweepTarget {
                pid: listener.pid,
                executable: PathBuf::from(listener.executable),
                reason: EngineSweepReason::OrphanedFromDaemon,
            }];
            for p in bundle_parents {
                targets.push(EngineSweepTarget {
                    pid: p.pid,
                    executable: PathBuf::from(p.executable).to_path_buf(),
                    reason: EngineSweepReason::OrphanedFromDaemon,
                });
            }
            return EngineListenerDecision::Sweep(targets);
        }
        let Some(parent) = entries.get(&current.parent_pid).copied() else {
            return EngineListenerDecision::Keep(EngineKeepReason::IncompleteAncestry {
                pid: current.pid,
                parent_pid: current.parent_pid,
            });
        };
        if !visited.insert(parent.pid) {
            return EngineListenerDecision::Keep(EngineKeepReason::CyclicAncestry {
                pid: parent.pid,
            });
        }
        if parent.user_id == own_user_id && Path::new(parent.executable) == expected_daemon {
            return EngineListenerDecision::Keep(EngineKeepReason::LiveDaemonAncestor {
                pid: parent.pid,
            });
        }
        if parent.user_id == own_user_id && Path::new(parent.executable) == expected_engine {
            bundle_parents.push(parent);
        }
        current = parent;
    }
}

#[cfg(test)]
mod tests {
    use super::{
        EngineKeepReason, EngineListenerDecision, EngineSweepReason, EngineSweepTarget,
        decide_engine_listener,
    };
    use std::path::PathBuf;

    fn bundle_path(path: &str) -> PathBuf {
        PathBuf::from(path.replace("Orbit.app", crate::orbit_bundle::NAME))
    }

    #[test]
    fn only_an_orphaned_listener_from_this_bundle_and_user_is_selected() {
        let bundle = bundle_path("/Users/sample/Applications/Orbit.app");
        let cases = [
            (
                "orphaned bundled engine",
                concat!(
                    "  40   1 501 /Users/sample/Applications/Orbit.app/Contents/MacOS/Orbit\n",
                    "  41   1 501 /Users/sample/Applications/Orbit.app/Contents/Helpers/Orbit Engine.app/Contents/MacOS/cua-driver\n",
                ),
                41,
                EngineListenerDecision::Sweep(vec![EngineSweepTarget {
                    pid: 41,
                    executable: bundle_path(
                        "/Users/sample/Applications/Orbit.app/Contents/Helpers/Orbit Engine.app/Contents/MacOS/cua-driver",
                    ),
                    reason: EngineSweepReason::OrphanedFromDaemon,
                }]),
            ),
            (
                // The daemon runs inside the app, so the engine's
                // parent is `MacOS/Orbit` itself; that app is alive.
                "engine descended from the live app its daemon runs in",
                concat!(
                    "  40   1 501 /Users/sample/Applications/Orbit.app/Contents/MacOS/Orbit\n",
                    "  51  40 501 /Users/sample/Applications/Orbit.app/Contents/Helpers/Orbit Engine.app/Contents/MacOS/cua-driver\n",
                ),
                51,
                EngineListenerDecision::Keep(EngineKeepReason::LiveDaemonAncestor { pid: 40 }),
            ),
            (
                "engine from another installation",
                concat!(
                    "  40   1 501 /Users/sample/Applications/Orbit.app/Contents/MacOS/Orbit\n",
                    "  61   1 501 /Applications/Orbit.app/Contents/Helpers/Orbit Engine.app/Contents/MacOS/cua-driver\n",
                ),
                61,
                EngineListenerDecision::Keep(EngineKeepReason::DifferentExecutable {
                    found: bundle_path(
                        "/Applications/Orbit.app/Contents/Helpers/Orbit Engine.app/Contents/MacOS/cua-driver",
                    ),
                    expected: bundle_path(
                        "/Users/sample/Applications/Orbit.app/Contents/Helpers/Orbit Engine.app/Contents/MacOS/cua-driver",
                    ),
                }),
            ),
            (
                "engine from another user",
                concat!(
                    "  40   1 501 /Users/sample/Applications/Orbit.app/Contents/MacOS/Orbit\n",
                    "  71   1 502 /Users/sample/Applications/Orbit.app/Contents/Helpers/Orbit Engine.app/Contents/MacOS/cua-driver\n",
                ),
                71,
                EngineListenerDecision::Keep(EngineKeepReason::DifferentUser {
                    found: 502,
                    expected: 501,
                }),
            ),
            (
                "foreign socket listener",
                concat!(
                    "  40   1 501 /Users/sample/Applications/Orbit.app/Contents/MacOS/Orbit\n",
                    "  81   1 501 /Applications/Editor.app/Contents/MacOS/editor\n",
                ),
                81,
                EngineListenerDecision::Keep(EngineKeepReason::DifferentExecutable {
                    found: bundle_path("/Applications/Editor.app/Contents/MacOS/editor"),
                    expected: bundle_path(
                        "/Users/sample/Applications/Orbit.app/Contents/Helpers/Orbit Engine.app/Contents/MacOS/cua-driver",
                    ),
                }),
            ),
            (
                "unreadable listener line",
                concat!(
                    "  40   1 501 /Users/sample/Applications/Orbit.app/Contents/MacOS/Orbit\n",
                    "  bad parent user /Users/sample/Applications/Orbit.app/Contents/Helpers/Orbit Engine.app/Contents/MacOS/cua-driver\n",
                ),
                91,
                EngineListenerDecision::Keep(EngineKeepReason::ListenerNotListed),
            ),
            (
                "linux comm listing has no executable path",
                "  40   1 501 Orbit\n  91   1 501 cua-driver\n",
                91,
                EngineListenerDecision::Keep(EngineKeepReason::DifferentExecutable {
                    found: bundle_path("cua-driver"),
                    expected: bundle_path(
                        "/Users/sample/Applications/Orbit.app/Contents/Helpers/Orbit Engine.app/Contents/MacOS/cua-driver",
                    ),
                }),
            ),
        ];
        for (case, listing, listener, expected) in cases {
            assert_eq!(
                decide_engine_listener(
                    &listing.replace("Orbit.app", crate::orbit_bundle::NAME),
                    listener,
                    501,
                    &bundle
                ),
                expected,
                "{case}"
            );
        }
    }

    #[test]
    fn absent_parent_is_unknown_not_orphaned() {
        let listing = concat!(
            "  40   1 501 /Users/sample/Applications/Orbit.app/Contents/MacOS/Orbit\n",
            "  92 777 501 /Users/sample/Applications/Orbit.app/Contents/Helpers/Orbit Engine.app/Contents/MacOS/cua-driver\n",
        );
        assert_eq!(
            decide_engine_listener(
                &listing.replace("Orbit.app", crate::orbit_bundle::NAME),
                92,
                501,
                &bundle_path("/Users/sample/Applications/Orbit.app"),
            ),
            EngineListenerDecision::Keep(EngineKeepReason::IncompleteAncestry {
                pid: 92,
                parent_pid: 777,
            })
        );
    }

    #[test]
    fn foreign_bundle_parent_is_never_a_sweep_target() {
        let listing = concat!(
            "  77 1 501 /Applications/Other.app/Contents/MacOS/cua-driver\n",
            "  78 77 501 /Applications/Orbit.app/Contents/Helpers/Orbit Engine.app/Contents/MacOS/cua-driver\n",
        );
        let EngineListenerDecision::Sweep(targets) = decide_engine_listener(
            &listing.replace("Orbit.app", crate::orbit_bundle::NAME),
            78,
            501,
            &bundle_path("/Applications/Orbit.app"),
        ) else {
            panic!("the exact listener is orphaned");
        };
        assert_eq!(
            targets.iter().map(|target| target.pid).collect::<Vec<_>>(),
            vec![78]
        );
    }
}
