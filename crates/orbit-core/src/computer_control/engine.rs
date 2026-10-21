//! When the control engine runs, decided from the macOS permissions alone.
//!
//! How the engine starts, lives and dies, and why it is decided here.
//!
//! Before this module the engine (`cua-driver serve` plus its MCP proxy, both
//! children of the daemon) was started by one switch, «Permitir control»
//! (`settings.remote_action`): turning it on called `ensure_engine_connected`,
//! and the actor's one-second maintenance called it again while the switch
//! stayed on, but only with Orbit paired, the hub online and privacy armed.
//! It then lived in the actor's `ComputerRuntime::service`, lent to one worker
//! per operation. Every `revoke_computer` killed it — the privacy stop, but
//! also opening or starting another conversation, the hub dropping, any state
//! decision taken while offline, and turning the switch off — and the next
//! maintenance tick started a new one and asked it for its catalogue again.
//! The macOS permissions were consulted nowhere on that path.
//!
//! Now the decision is this module, a pure function of facts the
//! daemon passes in:
//!
//! - the engine starts when macOS grants both Accessibility and Screen
//!   Recording, privacy is armed and no engine is running. Nothing else: no
//!   switch, no order, no pairing and no hub, and it is the packaged driver:
//!   whatever Orbit launches inherits what macOS granted Orbit, so no order
//!   names the executable or its socket. The helper already reports both
//!   permissions; the daemon now asks it again at a fixed interval, so a grant
//!   or a withdrawal made in System Settings reaches this decision without a
//!   restart;
//! - it stops when privacy is stopped or when macOS is known not to grant one
//!   of the two, and the stop names which one is missing;
//! - a permission the Mac has not reported yet is `Unknown`: it neither
//!   starts an engine nor stops a running one. A refused start and a refused
//!   request say «todavía no se sabe», never «no». With no helper connected
//!   nothing reports the permissions at all; that is its own cause, which
//!   does not invite asking again, and it neither starts nor stops either;
//! - changing conversation and losing the hub are not facts here at all, so
//!   they cannot touch the engine or make it fetch its catalogue again.
//!
//! How the hub knows this Mac can act, without the user deciding anything twice:
//! it does not keep a copy. The hub's record of the device is the pairing;
//! whether the Mac can act right now is answered by the Mac, on every request,
//! from this same decision — served, or refused with the permission that is
//! missing or still unknown. A copy on the hub would be one more state to fall
//! out of step with macOS, and a new field in the frame would be refused by a
//! hub that has not been updated.
use crate::{
    PermissionState,
    refusal::{Permission, Refusal, missing_permission_name, missing_system_permission_refusal},
};
use serde::{Deserialize, Serialize};

/// Which of the two macOS permissions a decision is about. A set rather than a
/// list, so «both» cannot be written twice or in two orders.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SystemPermissions {
    pub accessibility: bool,
    pub screen_recording: bool,
}

impl SystemPermissions {
    /// What the engine needs: it reads the screen and it acts on it.
    pub const ENGINE: Self = Self {
        accessibility: true,
        screen_recording: true,
    };
    /// A picture or the list of windows: the screen, nothing else.
    pub const SCREEN: Self = Self {
        accessibility: false,
        screen_recording: true,
    };

    pub fn is_empty(self) -> bool {
        !self.accessibility && !self.screen_recording
    }
}

/// What macOS reports for the permissions a decision needs, in three values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Grant {
    /// Every required permission is granted.
    Granted,
    /// macOS is known not to grant these (at least one). A known «no» wins
    /// over an unknown, because it is already certain the answer is no.
    Missing(SystemPermissions),
    /// None is known to be missing, and these have not been reported yet.
    Unknown(SystemPermissions),
}

fn granted(state: PermissionState) -> Option<bool> {
    match state {
        PermissionState::Granted => Some(true),
        PermissionState::Denied | PermissionState::NotGranted | PermissionState::NotDetermined => {
            Some(false)
        }
        PermissionState::Unknown => None,
    }
}

/// Reads what the Mac last reported against what is `required`.
pub fn grant(
    required: SystemPermissions,
    accessibility: PermissionState,
    screen_recording: PermissionState,
) -> Grant {
    let accessibility = required.accessibility.then(|| granted(accessibility));
    let screen_recording = required.screen_recording.then(|| granted(screen_recording));
    let missing = SystemPermissions {
        accessibility: accessibility == Some(Some(false)),
        screen_recording: screen_recording == Some(Some(false)),
    };
    if !missing.is_empty() {
        return Grant::Missing(missing);
    }
    let unknown = SystemPermissions {
        accessibility: accessibility == Some(None),
        screen_recording: screen_recording == Some(None),
    };
    if !unknown.is_empty() {
        return Grant::Unknown(unknown);
    }
    Grant::Granted
}

/// What the Mac has told the daemon, passed in rather than read: the privacy
/// stop, whether its helper — the one that reports what macOS grants — is
/// connected, and the two permissions as it last reported them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Observed {
    pub privacy_stopped: bool,
    /// Whether the helper is connected. Without it nobody reports the
    /// permissions, so what was reported last is not known to be current,
    /// and waiting brings no answer.
    pub helper_connected: bool,
    pub accessibility: PermissionState,
    pub screen_recording: PermissionState,
}

/// Whether something that needs `required` may start now.
///
/// In this order: the privacy stop; the helper, because without it nothing is
/// being reported and a stale report is not a fact; then what macOS grants.
/// Unknown refuses a start like a missing permission, but with its own cause:
/// starting on a permission nobody has reported would be deciding for macOS.
pub fn admits(required: SystemPermissions, observed: Observed) -> Result<(), Refusal> {
    if observed.privacy_stopped {
        return Err(Refusal::PrivacyStopped { since_ms: None });
    }
    if !observed.helper_connected {
        return Err(Refusal::HelperNotRunning);
    }
    match grant(required, observed.accessibility, observed.screen_recording) {
        Grant::Granted => Ok(()),
        Grant::Missing(missing) => Err(missing_system_permission_refusal(
            missing,
            observed.accessibility,
            observed.screen_recording,
        )),
        Grant::Unknown(unknown) => Err(Refusal::PermissionUnknown {
            permission: missing_permission_name(unknown),
        }),
    }
}

/// Why work already running on the engine has to stop, if it has to.
///
/// Only known facts revoke: the privacy stop and a permission macOS is known
/// not to grant. What is not known — a report not in yet, or no helper to make
/// one — leaves running work alone. One decision per cause.
pub fn revokes(observed: Observed) -> Option<Refusal> {
    if observed.privacy_stopped {
        return Some(Refusal::PrivacyStopped { since_ms: None });
    }
    if observed.helper_connected
        && let Grant::Missing(missing) = grant(
            SystemPermissions::ENGINE,
            observed.accessibility,
            observed.screen_recording,
        )
    {
        return Some(missing_system_permission_refusal(
            missing,
            observed.accessibility,
            observed.screen_recording,
        ));
    }
    None
}

/// Everything the lifecycle decision reads, passed in rather than observed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Facts {
    pub observed: Observed,
    /// The daemon has begun its one-way close; permissions do not reopen it.
    pub closing: bool,
    /// An engine process exists: connected, connecting, or lent to a worker.
    pub running: bool,
    /// False only during the pause after an engine failed to come up, so a
    /// broken engine is not restarted in a loop. The daemon owns that clock.
    pub retry_due: bool,
}

/// What the daemon does with the engine this turn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    Start,
    Keep,
    Stop(Refusal),
    StopForExit,
}

/// The engine's lifecycle: start with both permissions, stop on a known
/// refusal, and leave it as it is on anything unknown. One decision per cause.
pub fn decide(facts: Facts) -> Decision {
    if facts.closing {
        return if facts.running {
            Decision::StopForExit
        } else {
            Decision::Keep
        };
    }
    if facts.running {
        return match revokes(facts.observed) {
            Some(refusal) => Decision::Stop(refusal),
            None => Decision::Keep,
        };
    }
    if facts.retry_due && admits(SystemPermissions::ENGINE, facts.observed).is_ok() {
        Decision::Start
    } else {
        Decision::Keep
    }
}

/// The engine as the app shows it and a remote caller is told about it.
///
/// Every state that is not `Ready` carries its cause and its numbers, so
/// «the engine is not available» is never the whole answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum Status {
    /// Connected; `tools` is how many the engine declared.
    Ready { tools: u32 },
    /// Both permissions are granted and Orbit is starting it.
    Starting,
    /// Privacy is latched; it starts again once the user rearms.
    PrivacyStopped,
    /// The helper that reports the permissions is not connected.
    HelperNotConnected,
    /// macOS does not grant these.
    PermissionMissing { permissions: SystemPermissions },
    /// The helper has not reported these yet.
    PermissionUnknown { permissions: SystemPermissions },
    /// It failed to come up; Orbit tries again in `retry_in_ms`.
    Failed { cause: String, retry_in_ms: u64 },
}

impl Status {
    /// The status that a refusal describes.
    pub fn refused(refusal: Refusal) -> Option<Self> {
        if matches!(refusal, Refusal::PrivacyStopped { .. }) {
            Some(Self::PrivacyStopped)
        } else if matches!(refusal, Refusal::HelperNotRunning) {
            Some(Self::HelperNotConnected)
        } else if let Refusal::PermissionDenied { permission }
        | Refusal::PermissionNotGranted { permission }
        | Refusal::PermissionUnknown { permission } = &refusal
        {
            let permissions = SystemPermissions {
                accessibility: matches!(
                    permission,
                    Permission::Accessibility | Permission::ScreenAndAccessibility
                ),
                screen_recording: matches!(
                    permission,
                    Permission::ScreenRecording | Permission::ScreenAndAccessibility
                ),
            };
            if matches!(refusal, Refusal::PermissionUnknown { .. }) {
                Some(Self::PermissionUnknown { permissions })
            } else {
                Some(Self::PermissionMissing { permissions })
            }
        } else {
            None
        }
    }
}

#[cfg(test)]
#[path = "engine_tests.rs"]
mod tests;
