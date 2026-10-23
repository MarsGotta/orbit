//! The account's full name, which names the user's microphone on the hub:
//! read once off the loop, and sent to the hub each time it connects.
use super::{NoteEvent, Outcome, Runtime, deliver};
use crate::daemon::ticketed::DeadlineKind;
use crate::hub_notes::NoteHub;
use orbit_core::notes as core;
use std::time::Duration;

/// After the profile could not reach the hub it goes again no sooner than
/// this: the hub is down or the network is, and once every thirty seconds
/// keeps a failing hub from being asked every second. Not a deadline: the
/// name goes again at the next connection anyway.
const RETRY_AFTER_MS: i64 = 30_000;

/// How long the account's full name may take. macOS answers `id -F` from
/// Directory Services in milliseconds; if opendirectoryd hangs, the read hangs
/// with it. Two seconds is far above a healthy answer; meanwhile the hub names
/// the microphone «Micrófono local». It bounds this one read, which runs off
/// the daemon loop.
const ACCOUNT_NAME: DeadlineKind = DeadlineKind::new(
    "account_name",
    Duration::from_secs(2),
    "the account name is read off the loop and never holds a privacy stop",
);

/// The full name of the account (`pw_gecos`) from a program that prints it,
/// `id -F` on macOS: any failure or a late answer is «Yo», never an error.
// Used on macOS, and by its test everywhere.
#[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
pub(super) async fn read_account_name(program: &std::path::Path, deadline: Duration) -> String {
    let output = tokio::time::timeout(
        deadline,
        tokio::process::Command::new(program)
            .arg("-F")
            .stdin(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true)
            .output(),
    )
    .await;
    let gecos = match output {
        Ok(Ok(output)) if output.status.success() => String::from_utf8(output.stdout).ok(),
        Ok(Ok(_) | Err(_)) | Err(_) => None,
    };
    core::owner_name(gecos.as_deref())
}

/// macOS keeps accounts in Directory Services, so `/etc/passwd` does not hold
/// them there; elsewhere (development) `/etc/passwd` is read.
async fn account_full_name() -> String {
    #[cfg(target_os = "macos")]
    {
        read_account_name(std::path::Path::new("/usr/bin/id"), ACCOUNT_NAME.duration()).await
    }
    #[cfg(not(target_os = "macos"))]
    {
        let read = tokio::task::spawn_blocking(|| {
            let uid = rustix::process::getuid().as_raw().to_string();
            std::fs::read_to_string("/etc/passwd")
                .ok()
                .and_then(|passwd| {
                    passwd.lines().find_map(|line| {
                        let fields = line.split(':').collect::<Vec<_>>();
                        if fields.get(2) == Some(&uid.as_str()) {
                            fields.get(4).map(|gecos| (*gecos).to_owned())
                        } else {
                            None
                        }
                    })
                })
        });
        let gecos = match tokio::time::timeout(ACCOUNT_NAME.duration(), read).await {
            Ok(Ok(gecos)) => gecos,
            Ok(Err(_)) | Err(_) => None,
        };
        core::owner_name(gecos.as_deref())
    }
}

impl Runtime {
    pub(super) fn start_owner_lookup(&mut self) {
        let notes = &mut self.features.notes;
        if notes.owner_name.is_some() || notes.owner_lookup_started {
            return;
        }
        notes.owner_lookup_started = true;
        let events = notes.events.clone();
        tokio::spawn(async move {
            let name = account_full_name().await;
            deliver(&events, NoteEvent::Owner { name }).await;
        });
    }

    pub(super) fn start_profile(&mut self, hub: &NoteHub, now_ms: i64) -> bool {
        let generation = self.state.generation();
        let op = self.turn.op.clone();
        let notes = &mut self.features.notes;
        let Some(owner) = notes.owner_name.clone() else {
            return false;
        };
        if !notes.profile_owed || now_ms < notes.profile_due_ms {
            return false;
        }
        let hub = hub.clone();
        let events = notes.events.clone();
        let started = notes
            .profile
            .start_logical(generation, op, (), move |ticket| {
                tokio::spawn(async move {
                    let outcome = hub.profile(&owner).await.into();
                    deliver(&events, NoteEvent::Profile { ticket, outcome }).await;
                })
            })
            .is_some();
        if started {
            notes.profile_owed = false;
        }
        started
    }

    /// The hub's answer to the profile: what it did not take goes again later.
    pub(super) fn profile_answer(&mut self, outcome: &Outcome, now_ms: i64) {
        if !matches!(outcome, Outcome::Done { .. }) {
            self.log_outcome("device_profile_not_saved", outcome);
            let notes = &mut self.features.notes;
            notes.profile_owed = true;
            notes.profile_due_ms = now_ms.saturating_add(RETRY_AFTER_MS);
        }
    }
}
