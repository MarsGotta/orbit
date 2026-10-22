//! The control topic: the engine with its cause, the action in progress and
//! the most recent history entries.
//!
//! The engine is `Knowledge<Engine>`: ready with how many tools it declares,
//! coming up, or absent with the refusal that names the cause — Accessibility,
//! Screen Recording, the privacy stop, the missing helper, or the engine
//! falling over. «Unknown» keeps its own word: a permission nobody has
//! reported is published as `permission_unknown`, never as denied, because
//! the lifecycle decision (wave 2, `computer_control::engine`) neither starts
//! nor revokes on it and this module only publishes that truth.
//!
//! The whole 256-entry record and the engine's
//! tool catalogue are paged reads, not topic payload.
use crate::{
    computer_control::{ControlHistoryEntry, engine},
    model::PermissionState,
    refusal::{EngineCause, Permission, Refusal, missing_system_permission_refusal},
    topics::Knowledge,
};
use serde::{Deserialize, Serialize};

/// How many history entries the head carries. An entry measured 203 B, so
/// twenty fill ~4 KiB of the 16 KiB ceiling; this bounds the frame the daemon
/// republishes on every control change, not the record, whose
/// 256 entries (50.7 KiB measured) are the paged `ControlHistory` read.
pub const RECENT_HISTORY: usize = 20;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Value {
    /// The engine, in the three values: up or coming up, absent with its
    /// cause, or unknown.
    pub engine: Knowledge<Engine>,
    /// A wall-clock retry deadline captured when the engine fails. A stable
    /// instant lets readers advance the display without republishing a topic.
    pub retry_at_ms: Option<i64>,
    /// The tool call in flight, exactly as its history entry will record it.
    /// `None` when nothing is running. The retired browser-input path
    /// publishes through the engine state and its own history entry.
    pub action: Option<ControlHistoryEntry>,
    /// A storage failure affects the record alone, not the engine or action.
    pub history: Knowledge<History>,
    /// The folders the user gave the assistant, each with whether it opens.
    /// Unknown from a daemon that predates them; absent with its cause when
    /// the store could not read them.
    /// Boxed: a `Knowledge` holds a whole `Refusal`, and inline it doubled
    /// the shared `TopicData` enum.
    #[serde(default = "folders_not_observed")]
    pub folders: Box<Knowledge<Vec<crate::assistant_files::FolderView>>>,
}

fn folders_not_observed() -> Box<Knowledge<Vec<crate::assistant_files::FolderView>>> {
    Box::new(Knowledge::Unknown {
        why: crate::topics::Pending::NotObservedYet,
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EngineCleanup {
    pub pid: u32,
    pub executable: String,
    pub reason: EngineCleanupReason,
    pub outcome: EngineCleanupOutcome,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EngineCleanupReason {
    OrphanedFromDaemon,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EngineCleanupOutcome {
    Stopped,
    Survived,
    Unconfirmed,
    SignalFailed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct History {
    pub total: u64,
    /// The most recent entries, newest first.
    pub recent: Vec<ControlHistoryEntry>,
}

/// The engine in the states Orbit itself reports. Everything that keeps it
/// from being up is a refusal on the `Absent` side, so no cause is ever a
/// bare «unavailable».
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Engine {
    /// Connected; `tools` is how many the engine declared.
    Ready { tools: u32 },
    /// Both permissions are granted and Orbit is starting it.
    Starting,
}

/// The engine's status as the topic publishes it. `failure_kind` says which
/// of the two ways it failed — it never came up, or it dropped while running
/// — because the status itself only carries the daemon's detail text.
/// Publication uses the two permission facts the helper actually reported.
/// A missing permission not yet granted is distinct from one macOS denied.
pub fn engine_knowledge_with_permissions(
    status: engine::Status,
    failure_kind: Option<EngineCause>,
    accessibility: PermissionState,
    screen_recording: PermissionState,
) -> Knowledge<Engine> {
    match status {
        engine::Status::Ready { tools } => Knowledge::Present {
            value: Engine::Ready { tools },
            as_of_ms: None,
        },
        engine::Status::Starting => Knowledge::Present {
            value: Engine::Starting,
            as_of_ms: None,
        },
        engine::Status::PrivacyStopped => Knowledge::Absent {
            refusal: Refusal::PrivacyStopped { since_ms: None },
        },
        engine::Status::HelperNotConnected => Knowledge::Absent {
            refusal: Refusal::HelperNotRunning,
        },
        engine::Status::PermissionMissing { permissions } => Knowledge::Absent {
            refusal: missing_system_permission_refusal(
                permissions,
                accessibility,
                screen_recording,
            ),
        },
        engine::Status::PermissionUnknown { permissions } => Knowledge::Absent {
            refusal: Refusal::PermissionUnknown {
                permission: permission_name(permissions),
            },
        },
        engine::Status::Failed { .. } => Knowledge::Absent {
            refusal: Refusal::EngineUnavailable {
                cause: failure_kind.unwrap_or(EngineCause::InitializationFailed),
            },
        },
    }
}

fn permission_name(permissions: engine::SystemPermissions) -> Permission {
    crate::refusal::missing_permission_name(permissions)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn engine_knowledge(status: engine::Status, cause: Option<EngineCause>) -> Knowledge<Engine> {
        engine_knowledge_with_permissions(
            status,
            cause,
            PermissionState::Denied,
            PermissionState::Denied,
        )
    }

    fn permissions(accessibility: bool, screen_recording: bool) -> engine::SystemPermissions {
        engine::SystemPermissions {
            accessibility,
            screen_recording,
        }
    }

    #[test]
    fn every_engine_state_carries_its_cause_or_its_count() {
        assert_eq!(
            engine_knowledge(engine::Status::Ready { tools: 12 }, None),
            Knowledge::Present {
                value: Engine::Ready { tools: 12 },
                as_of_ms: None,
            }
        );
        assert_eq!(
            engine_knowledge(engine::Status::Starting, None),
            Knowledge::Present {
                value: Engine::Starting,
                as_of_ms: None,
            }
        );
        assert_eq!(
            engine_knowledge(engine::Status::PrivacyStopped, None),
            Knowledge::Absent {
                refusal: Refusal::PrivacyStopped { since_ms: None },
            }
        );
        assert_eq!(
            engine_knowledge(engine::Status::HelperNotConnected, None),
            Knowledge::Absent {
                refusal: Refusal::HelperNotRunning,
            }
        );
    }

    #[test]
    fn a_missing_and_an_unreported_permission_never_share_a_word() {
        let missing = engine_knowledge(
            engine::Status::PermissionMissing {
                permissions: permissions(true, false),
            },
            None,
        );
        let unreported = engine_knowledge(
            engine::Status::PermissionUnknown {
                permissions: permissions(false, true),
            },
            None,
        );
        assert_eq!(
            missing,
            Knowledge::Absent {
                refusal: Refusal::PermissionDenied {
                    permission: Permission::Accessibility,
                },
            }
        );
        assert_eq!(
            unreported,
            Knowledge::Absent {
                refusal: Refusal::PermissionUnknown {
                    permission: Permission::ScreenRecording,
                },
            }
        );
        assert_ne!(missing, unreported);
    }

    #[test]
    fn both_permissions_missing_name_both() {
        assert_eq!(
            engine_knowledge(
                engine::Status::PermissionMissing {
                    permissions: permissions(true, true),
                },
                None,
            ),
            Knowledge::Absent {
                refusal: Refusal::PermissionDenied {
                    permission: Permission::ScreenAndAccessibility,
                },
            }
        );
    }

    #[test]
    fn a_non_grant_in_a_missing_set_keeps_the_bridge_cause() {
        assert_eq!(
            engine_knowledge_with_permissions(
                engine::Status::PermissionMissing {
                    permissions: permissions(true, true)
                },
                None,
                PermissionState::NotGranted,
                PermissionState::Denied,
            ),
            Knowledge::Absent {
                refusal: Refusal::PermissionNotGranted {
                    permission: Permission::ScreenAndAccessibility,
                }
            },
        );
    }

    #[test]
    fn a_failed_engine_is_absent_with_which_way_it_failed() {
        assert_eq!(
            engine_knowledge(
                engine::Status::Failed {
                    cause: "detalle del daemon".into(),
                    retry_in_ms: 5_000,
                },
                Some(EngineCause::Unhealthy),
            ),
            Knowledge::Absent {
                refusal: Refusal::EngineUnavailable {
                    cause: EngineCause::Unhealthy,
                },
            }
        );
        assert_eq!(
            engine_knowledge(
                engine::Status::Failed {
                    cause: String::new(),
                    retry_in_ms: 0,
                },
                None,
            ),
            Knowledge::Absent {
                refusal: Refusal::EngineUnavailable {
                    cause: EngineCause::InitializationFailed,
                },
            }
        );
    }

    #[test]
    fn the_wire_keeps_the_three_values_apart() -> Result<(), serde_json::Error> {
        let cases = [
            engine_knowledge(engine::Status::Ready { tools: 2 }, None),
            engine_knowledge(
                engine::Status::PermissionUnknown {
                    permissions: permissions(true, true),
                },
                None,
            ),
            Knowledge::Unknown {
                why: crate::topics::Pending::NotObservedYet,
            },
        ];
        let wired: Vec<serde_json::Value> = cases
            .iter()
            .map(serde_json::to_value)
            .collect::<Result<_, _>>()?;
        assert_eq!(wired[0]["state"], "present");
        assert_eq!(wired[0]["value"]["state"], "ready");
        assert_eq!(wired[1]["state"], "absent");
        assert_eq!(wired[1]["refusal"]["code"], "permission_unknown");
        assert_eq!(
            wired[1]["refusal"]["permission"],
            "screen_and_accessibility"
        );
        assert_eq!(wired[2]["state"], "unknown");
        for (value, back) in cases.iter().zip(wired) {
            assert_eq!(&serde_json::from_value::<Knowledge<Engine>>(back)?, value);
        }
        Ok(())
    }

    #[test]
    fn maximal_recent_record_fits_the_control_topic_ceiling()
    -> Result<(), Box<dyn std::error::Error>> {
        let entry = ControlHistoryEntry::new(
            &"i".repeat(128),
            &"r".repeat(128),
            &"a".repeat(256),
            1_000,
            crate::computer_control::ControlOutcome::Performed,
        )
        .map_err(std::io::Error::other)?
        .from_channel(&"c".repeat(64))
        .map_err(std::io::Error::other)?;
        let data = crate::topics::TopicData::Control(Knowledge::Present {
            value: Value {
                engine: Knowledge::Present {
                    value: Engine::Ready { tools: 256 },
                    as_of_ms: None,
                },
                retry_at_ms: None,
                action: None,
                history: Knowledge::Present {
                    value: History {
                        total: 256,
                        recent: vec![entry; RECENT_HISTORY],
                    },
                    as_of_ms: None,
                },
                folders: Box::new(Knowledge::Present {
                    value: (0..crate::assistant_files::MAX_FOLDERS)
                        .map(|index| crate::assistant_files::FolderView {
                            id: format!("{index:0>36}"),
                            // The costliest path: every byte a control
                            // character, six bytes once escaped.
                            path: "\u{01}".repeat(crate::assistant_files::MAX_PATH_BYTES),
                            access: crate::assistant_files::FolderAccess::Unavailable {
                                cause: crate::assistant_files::FolderCause::BookmarkUnresolvable {
                                    domain: "d"
                                        .repeat(crate::assistant_files::MAX_ERROR_DOMAIN_BYTES),
                                    code: i64::MIN,
                                },
                            },
                        })
                        .collect(),
                    as_of_ms: None,
                }),
            },
            as_of_ms: None,
        });
        let bytes = serde_json::to_vec(&data)?.len();
        assert!(
            bytes <= crate::topics::TopicId::Control.max_bytes(),
            "maximal recent control head is {bytes} bytes"
        );
        Ok(())
    }
}
