use super::{FeatureAvailability, FeatureInput};
use crate::AppSnapshot;
use crate::topics::TopicFreshness;
use orbit_core::{
    refusal::{Permission, Refusal},
    topics::{Knowledge, TopicData, TopicId, control},
};

pub type View = Option<Box<crate::ComputerControlSnapshot>>;

pub fn project(input: &FeatureInput<'_>) -> View {
    if let Some(cell) = input.topics.topic(TopicId::Control)
        && let TopicData::Control(knowledge) = &cell.knowledge
    {
        return Some(Box::new(from_topic(
            knowledge,
            input.topics,
            &cell.freshness,
            input.copy,
        )));
    }
    Some(Box::new(unknown_view(FeatureAvailability::missing(
        input.topics,
    ))))
}

pub fn apply(view: &View, snapshot: &mut AppSnapshot) {
    snapshot.computer_control = view.clone();
}

/// The control view is projected from its topic alone. A daemon that never
/// sent a control topic leaves its availability unknown.
fn from_topic(
    knowledge: &Knowledge<control::Value>,
    topics: &crate::topics::TopicCache,
    freshness: &TopicFreshness,
    copy: crate::PresentationCopy,
) -> crate::ComputerControlSnapshot {
    let value = match knowledge {
        Knowledge::Present { value, .. } => value,
        Knowledge::Unknown { why } => {
            return unknown_view(FeatureAvailability::unknown_pending(why));
        }
        Knowledge::Absent { refusal } => {
            return unknown_view(FeatureAvailability::Absent {
                cause: refusal.to_string(),
            });
        }
    };
    let (history, history_detail) = match &value.history {
        Knowledge::Present { value, .. } => (value.recent.as_slice(), String::new()),
        Knowledge::Absent { refusal } => (&[][..], refusal.to_string()),
        Knowledge::Unknown { .. } => (&[][..], String::new()),
    };
    let (engine_tools, engine_tools_error) = engine_tools(topics, copy);
    let folders = folders_snapshot(&value.folders);
    crate::ComputerControlSnapshot {
        folders,
        availability: Some(match freshness {
            TopicFreshness::Current => FeatureAvailability::Current,
            TopicFreshness::Stale { since_ms } => FeatureAvailability::Stale {
                since_ms: *since_ms,
            },
        }),
        operation_id: value
            .action
            .as_ref()
            .map(|action| action.operation_id.clone())
            .unwrap_or_default(),
        // No stage names: the stage-as-text this projection used to read is
        // gone with the observation JSON.
        stage: String::new(),
        detail: String::new(),
        remaining_steps: 0,
        expires_in_ms: 0,
        engine: engine_snapshot(&value.engine, value.retry_at_ms),
        engine_tools,
        engine_tools_error,
        history_error: (!history_detail.is_empty()).then_some(history_detail),
        history: history
            .iter()
            .map(|entry| crate::ControlHistorySnapshot {
                agent: entry.agent.clone(),
                operation_id: entry.operation_id.clone(),
                requested: entry.requested.clone(),
                application: entry.application.clone(),
                channel: entry.channel.clone(),
                at_ms: entry.at_ms,
                outcome: entry.outcome.as_str().into(),
            })
            .collect(),
    }
}

fn unknown_view(availability: FeatureAvailability) -> crate::ComputerControlSnapshot {
    let detail = match &availability {
        FeatureAvailability::Unknown { cause } | FeatureAvailability::Absent { cause } => {
            cause.clone()
        }
        FeatureAvailability::Current | FeatureAvailability::Stale { .. } => String::new(),
    };
    crate::ComputerControlSnapshot {
        availability: Some(availability),
        operation_id: String::new(),
        stage: String::new(),
        detail,
        remaining_steps: 0,
        expires_in_ms: 0,
        engine: crate::EngineSnapshot::Unknown,
        engine_tools: Vec::new(),
        engine_tools_error: None,
        history_error: None,
        history: Vec::new(),
        folders: crate::AssistantFoldersSnapshot {
            max: MAX_FOLDERS,
            ..Default::default()
        },
    }
}

const MAX_FOLDERS: u16 = orbit_core::assistant_files::MAX_FOLDERS as u16;

/// The folders in their three values: listed, not reported yet, or a list
/// the store could not read (with its cause).
fn folders_snapshot(
    knowledge: &Knowledge<Vec<orbit_core::assistant_files::FolderView>>,
) -> crate::AssistantFoldersSnapshot {
    match knowledge {
        Knowledge::Present { value, .. } => crate::AssistantFoldersSnapshot {
            state: crate::AssistantFoldersState::Known,
            folders: value.clone(),
            max: MAX_FOLDERS,
            unreadable: None,
        },
        Knowledge::Unknown { .. } => crate::AssistantFoldersSnapshot {
            max: MAX_FOLDERS,
            ..Default::default()
        },
        Knowledge::Absent { refusal } => crate::AssistantFoldersSnapshot {
            state: crate::AssistantFoldersState::Unreadable,
            folders: Vec::new(),
            max: MAX_FOLDERS,
            unreadable: Some(refusal.to_string()),
        },
    }
}

fn engine_snapshot(
    engine: &Knowledge<control::Engine>,
    retry_at_ms: Option<i64>,
) -> crate::EngineSnapshot {
    match engine {
        Knowledge::Present { value, .. } => match *value {
            control::Engine::Ready { tools } => crate::EngineSnapshot::Ready { tools },
            control::Engine::Starting => crate::EngineSnapshot::Starting,
        },
        Knowledge::Unknown { .. } => crate::EngineSnapshot::Unknown,
        Knowledge::Absent { refusal } => refused_engine(refusal, retry_at_ms),
    }
}

fn refused_engine(refusal: &Refusal, retry_at_ms: Option<i64>) -> crate::EngineSnapshot {
    match refusal {
        Refusal::PrivacyStopped { .. } => crate::EngineSnapshot::PrivacyStopped,
        Refusal::HelperNotRunning => crate::EngineSnapshot::HelperNotConnected,
        // macOS does not say whether a missing permission was denied or never
        // decided; either way the engine lacks it.
        Refusal::PermissionDenied { permission } | Refusal::PermissionNotGranted { permission } => {
            crate::EngineSnapshot::PermissionMissing {
                accessibility: includes_accessibility(permission),
                screen_recording: includes_screen_recording(permission),
            }
        }
        Refusal::PermissionUnknown { permission } => crate::EngineSnapshot::PermissionUnknown {
            accessibility: includes_accessibility(permission),
            screen_recording: includes_screen_recording(permission),
        },
        Refusal::EngineUnavailable { cause } => crate::EngineSnapshot::Failed {
            cause: cause.to_string(),
            retry_in_ms: None,
            retry_at_ms,
        },
        // The daemon never refuses the engine head with these; an unrelated
        // refusal stays unsaid rather than mislabeled.
        Refusal::NotPaired
        | Refusal::DeviceReplyInvalid { .. }
        | Refusal::DeviceReplyTimeout { .. }
        | Refusal::MacDisconnected { .. }
        | Refusal::HubUnreachable { .. }
        | Refusal::Busy { .. }
        | Refusal::PendingRecording { .. }
        | Refusal::TooLarge { .. }
        | Refusal::TooMany { .. }
        | Refusal::InvalidParameter { .. }
        | Refusal::RevisionChanged { .. }
        | Refusal::Timeout { .. }
        | Refusal::Expired { .. }
        | Refusal::NotFound { .. }
        | Refusal::OutcomeUnknown { .. }
        | Refusal::UpstreamFailed { .. }
        | Refusal::StorageFailed { .. }
        | Refusal::Update { .. }
        | Refusal::CalendarBackendUnavailable
        | Refusal::HubSessionsUnpaged { .. }
        | Refusal::HubSessionInvalidRow { .. }
        | Refusal::HubSessionsCursorMismatch { .. }
        | Refusal::WorkInFlight { .. }
        | Refusal::PhysicalWorkFinishing { .. }
        | Refusal::WorkExpired { .. }
        | Refusal::Recovering { .. }
        | Refusal::CalendarReadLimit { .. }
        | Refusal::Attachment { .. }
        | Refusal::EncodeFailed { .. }
        | Refusal::HelperReplyInvalid { .. }
        | Refusal::ReadInterrupted { .. }
        | Refusal::HelperRefused { .. }
        | Refusal::Internal { .. }
        | Refusal::ProtocolVersion { .. }
        | Refusal::HubReplyInvalid { .. }
        | Refusal::DaemonReplyInvalid { .. }
        | Refusal::Legacy { .. }
        | Refusal::NoFolders
        | Refusal::OutsideFolders { .. }
        | Refusal::FolderUnavailable { .. }
        | Refusal::FileUnavailable { .. }
        | Refusal::FileTooLarge { .. } => crate::EngineSnapshot::Unknown,
    }
}

fn includes_accessibility(permission: &Permission) -> bool {
    matches!(
        permission,
        Permission::Accessibility | Permission::ScreenAndAccessibility
    )
}

fn includes_screen_recording(permission: &Permission) -> bool {
    matches!(
        permission,
        Permission::ScreenRecording | Permission::ScreenAndAccessibility
    )
}

fn engine_tools(
    topics: &crate::topics::TopicCache,
    copy: crate::PresentationCopy,
) -> (Vec<String>, Option<String>) {
    match topics.read(
        &orbit_core::reads::ReadRequest::EngineTools { cursor: None },
        TopicId::Control,
    ) {
        Some(Knowledge::Present { value, .. }) => {
            let names = value
                .items
                .iter()
                .map(|item| item.as_str().map(str::to_owned))
                .collect::<Option<Vec<_>>>();
            match names {
                Some(names) => (names, None),
                None => (
                    Vec::new(),
                    Some(copy.detail(&crate::presentation::read_issue(
                        "read_engine_tools_invalid",
                        serde_json::json!({}),
                    ))),
                ),
            }
        }
        Some(Knowledge::Absent { refusal }) => (Vec::new(), Some(refusal.to_string())),
        Some(Knowledge::Unknown { .. }) | None => (Vec::new(), None),
    }
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod topic_tests {
    use super::*;
    use crate::{
        ComputerControlSnapshot, EngineSnapshot,
        features::{FeatureId, FrameRevision, project_changed},
        topics::TopicCache,
    };
    use orbit_core::{
        computer_control::{ControlHistoryEntry, ControlOutcome},
        refusal::EngineCause,
        topics::{TopicData, TopicFrame, TopicId},
    };

    fn history_entry() -> Result<ControlHistoryEntry, &'static str> {
        ControlHistoryEntry::new("op-1", "click", "Fixture", 500, ControlOutcome::Unknown)
            .and_then(|entry| entry.from_channel("local"))
    }

    fn input<'a>(snapshot: &'a AppSnapshot, topics: &'a TopicCache) -> FeatureInput<'a> {
        FeatureInput::new(snapshot, topics)
    }

    fn cache_with_control(data: TopicData) -> TopicCache {
        let mut topics = TopicCache::default();
        topics.observe_at(
            TopicFrame {
                epoch: "synthetic".into(),
                revision: 1,
                data,
            },
            Some(1_000),
        );
        topics
    }

    fn ready_head() -> TopicData {
        let entry = history_entry().unwrap_or_else(|error| panic!("{error}"));
        TopicData::Control(Knowledge::Present {
            value: control::Value {
                engine: Knowledge::Present {
                    value: control::Engine::Ready { tools: 5 },
                    as_of_ms: None,
                },
                retry_at_ms: None,
                action: Some(entry.clone()),
                history: Knowledge::Present {
                    value: control::History {
                        total: 2,
                        recent: vec![entry],
                    },
                    as_of_ms: None,
                },
                folders: Box::new(Knowledge::Unknown {
                    why: orbit_core::topics::Pending::NotObservedYet,
                }),
            },
            as_of_ms: None,
        })
    }

    /// Settings shows each folder with whether it opens, an unreadable list
    /// with its cause, and an unreported one as unknown, never as empty.
    #[test]
    fn the_folders_reach_ajustes_in_their_three_values() {
        use orbit_core::assistant_files::{FolderAccess, FolderCause, FolderView};
        let snapshot = crate::AppSnapshot::empty(1, crate::DaemonConnection::Ready);
        let with = |folders| {
            let TopicData::Control(Knowledge::Present {
                mut value,
                as_of_ms,
            }) = ready_head()
            else {
                panic!("the ready head is present");
            };
            value.folders = Box::new(folders);
            cache_with_control(TopicData::Control(Knowledge::Present { value, as_of_ms }))
        };
        let folder = FolderView {
            id: "folder-1".into(),
            path: "/Users/ejemplo/Proyectos".into(),
            access: FolderAccess::Unavailable {
                cause: FolderCause::NotAFolder,
            },
        };
        let listed = with(Knowledge::Present {
            value: vec![folder.clone()],
            as_of_ms: None,
        });
        let view = project(&input(&snapshot, &listed)).expect("a head must fill the view");
        assert_eq!(view.folders.state, crate::AssistantFoldersState::Known);
        assert_eq!(view.folders.folders, vec![folder]);
        assert_eq!(
            usize::from(view.folders.max),
            orbit_core::assistant_files::MAX_FOLDERS
        );
        let unreadable = with(Knowledge::Absent {
            refusal: Refusal::StorageFailed {
                operation: orbit_core::refusal::Operation::Files,
                cause: Some("fila dañada".to_owned().into()),
            },
        });
        let view = project(&input(&snapshot, &unreadable)).expect("a head must fill the view");
        assert_eq!(view.folders.state, crate::AssistantFoldersState::Unreadable);
        assert!(
            view.folders
                .unreadable
                .is_some_and(|cause| cause.contains("fila dañada"))
        );
        let unknown = with(Knowledge::Unknown {
            why: orbit_core::topics::Pending::NotObservedYet,
        });
        let view = project(&input(&snapshot, &unknown)).expect("a head must fill the view");
        assert_eq!(view.folders.state, crate::AssistantFoldersState::Unknown);
        assert!(view.folders.folders.is_empty());
    }

    #[test]
    fn a_control_topic_arrival_reaches_the_control_frame() {
        let snapshot = crate::AppSnapshot::empty(1, crate::DaemonConnection::Ready);
        let before = TopicCache::default();
        let after = cache_with_control(TopicData::not_observed(TopicId::Control));
        let mut counter = 0;
        let frames = project_changed(
            &input(&snapshot, &before),
            &input(&snapshot, &after),
            || {
                counter += 1;
                FrameRevision { epoch: 1, counter }
            },
        );
        assert!(
            frames
                .iter()
                .any(|frame| frame.feature() == FeatureId::Control),
            "a control topic change must reproject the control function"
        );
    }

    #[test]
    fn the_engine_comes_from_the_topic_without_the_observation_json() {
        // No computer_control view at all in the legacy snapshot: whatever
        // the view says can only come from the topic.
        let snapshot = crate::AppSnapshot::empty(1, crate::DaemonConnection::Ready);
        assert!(snapshot.computer_control.is_none());
        let topics = cache_with_control(ready_head());
        let view = project(&input(&snapshot, &topics)).expect("the topic alone must fill the view");
        assert_eq!(view.engine, EngineSnapshot::Ready { tools: 5 });
        assert_eq!(view.operation_id, "op-1");
        assert_eq!(view.remaining_steps, 0);
        assert_eq!(view.history.len(), 1);
        assert_eq!(view.history[0].outcome, "unknown");
        assert_eq!(view.stage, "", "no stage names on the topic path");
        assert!(view.engine_tools.is_empty());
    }

    #[test]
    fn action_authors_reach_the_app_view_from_the_control_topic() {
        let snapshot = crate::AppSnapshot::empty(1, crate::DaemonConnection::Ready);
        let mut head = ready_head();
        let TopicData::Control(Knowledge::Present { value, .. }) = &mut head else {
            panic!("control fixture");
        };
        let Knowledge::Present { value: history, .. } = &mut value.history else {
            panic!("history fixture");
        };
        history.recent = [
            serde_json::json!({"id":"atlas", "name":"Atlas"}),
            serde_json::json!({"id":"vega", "name":"Vega"}),
        ]
        .into_iter()
        .map(|agent| {
            let mut entry = serde_json::to_value(history_entry().expect("history fixture"))
                .expect("serialized history fixture");
            entry["agent"] = agent;
            serde_json::from_value(entry).expect("attributed history fixture")
        })
        .collect();
        let topics = cache_with_control(head);
        let view = project(&input(&snapshot, &topics)).expect("control view");
        let encoded = serde_json::to_value(view).expect("serialized control view");
        assert_eq!(
            encoded["history"][0]["agent"],
            serde_json::json!({"id":"atlas", "name":"Atlas"})
        );
        assert_eq!(
            encoded["history"][1]["agent"],
            serde_json::json!({"id":"vega", "name":"Vega"})
        );
    }

    #[test]
    fn a_refused_engine_names_its_cause_with_its_permissions() {
        let snapshot = crate::AppSnapshot::empty(1, crate::DaemonConnection::Ready);
        let mut head = ready_head();
        if let TopicData::Control(Knowledge::Present { value, .. }) = &mut head {
            value.engine = Knowledge::Absent {
                refusal: Refusal::PermissionDenied {
                    permission: Permission::Accessibility,
                },
            };
        }
        let topics = cache_with_control(head);
        let view = project(&input(&snapshot, &topics)).expect("a head must fill the view");
        assert_eq!(
            view.engine,
            EngineSnapshot::PermissionMissing {
                accessibility: true,
                screen_recording: false,
            }
        );
    }

    #[test]
    fn an_unknown_head_keeps_the_engine_unsaid() {
        let snapshot = crate::AppSnapshot::empty(1, crate::DaemonConnection::Ready);
        let topics = cache_with_control(TopicData::not_observed(TopicId::Control));
        let view = project(&input(&snapshot, &topics)).expect("a topic cell must fill the view");
        assert_eq!(view.engine, EngineSnapshot::Unknown);
    }

    #[test]
    fn a_refused_head_keeps_its_storage_cause_for_the_reader() {
        let snapshot = crate::AppSnapshot::empty(1, crate::DaemonConnection::Ready);
        let refusal = Refusal::StorageFailed {
            operation: orbit_core::refusal::Operation::Storage,
            cause: None,
        };
        let topics = cache_with_control(TopicData::Control(Knowledge::Absent {
            refusal: refusal.clone(),
        }));
        let view = project(&input(&snapshot, &topics)).expect("a topic cell must fill the view");
        assert_eq!(view.engine, EngineSnapshot::Unknown);
        assert_eq!(view.detail, refusal.to_string());
    }

    #[test]
    fn without_a_topic_the_legacy_snapshot_cannot_claim_the_engine_is_starting() {
        let mut snapshot = crate::AppSnapshot::empty(1, crate::DaemonConnection::Ready);
        snapshot.computer_control = Some(Box::new(ComputerControlSnapshot {
            availability: None,
            operation_id: "legacy".into(),
            stage: "ready".into(),
            detail: "camino viejo".into(),
            remaining_steps: 0,
            expires_in_ms: 0,
            engine: EngineSnapshot::Starting,
            engine_tools: vec!["list_apps".into()],
            engine_tools_error: None,
            history_error: None,
            history: Vec::new(),
            folders: Default::default(),
        }));
        let topics = TopicCache::default();
        let view = project(&input(&snapshot, &topics)).expect("unknown control view");
        assert_eq!(view.engine, EngineSnapshot::Unknown);
        assert!(view.engine_tools.is_empty());
        assert!(matches!(
            view.availability,
            Some(FeatureAvailability::Unknown { .. })
        ));
    }

    #[test]
    fn a_failed_engine_carries_its_cause() {
        let snapshot = crate::AppSnapshot::empty(1, crate::DaemonConnection::Ready);
        let failed = TopicData::Control(Knowledge::Present {
            value: control::Value {
                engine: Knowledge::Absent {
                    refusal: Refusal::EngineUnavailable {
                        cause: EngineCause::Unhealthy,
                    },
                },
                retry_at_ms: Some(5_000),
                action: None,
                history: Knowledge::Present {
                    value: control::History {
                        total: 0,
                        recent: Vec::new(),
                    },
                    as_of_ms: None,
                },
                folders: Box::new(Knowledge::Unknown {
                    why: orbit_core::topics::Pending::NotObservedYet,
                }),
            },
            as_of_ms: None,
        });
        let topics = cache_with_control(failed);
        let view = project(&input(&snapshot, &topics)).expect("a head must fill the view");
        assert_eq!(
            view.engine,
            EngineSnapshot::Failed {
                cause: "unhealthy".into(),
                retry_in_ms: None,
                retry_at_ms: Some(5_000),
            }
        );
        let wire = serde_json::to_value(&view.engine).expect("failed engine serializes");
        assert!(
            wire.get("retry_in_ms").is_none(),
            "the topic publishes only its fixed deadline"
        );
    }

    #[test]
    fn engine_tools_read_reaches_the_control_view() {
        let snapshot = crate::AppSnapshot::empty(1, crate::DaemonConnection::Ready);
        let mut topics = cache_with_control(ready_head());
        let request = orbit_core::reads::ReadRequest::EngineTools { cursor: None };
        assert!(topics.remember_read(
            &request,
            TopicId::Control,
            Knowledge::Present {
                value: orbit_core::topics::Page {
                    items: vec![serde_json::json!("list_apps")],
                    next: None,
                    total: 1,
                    revision: 0,
                    store_version: None,
                    handed_over_from: None,
                },
                as_of_ms: None,
            }
        ));
        let view = project(&input(&snapshot, &topics)).expect("control head must project");
        assert_eq!(view.engine_tools, ["list_apps"]);
    }

    #[test]
    fn broken_history_keeps_a_ready_engine_and_names_only_the_history_failure() {
        let snapshot = crate::AppSnapshot::empty(1, crate::DaemonConnection::Ready);
        let mut head = ready_head();
        if let TopicData::Control(Knowledge::Present { value, .. }) = &mut head {
            value.history = Knowledge::Absent {
                refusal: Refusal::StorageFailed {
                    operation: orbit_core::refusal::Operation::Storage,
                    cause: None,
                },
            };
        }
        let topics = cache_with_control(head);
        let view = project(&input(&snapshot, &topics)).expect("control head must project");
        assert_eq!(view.engine, EngineSnapshot::Ready { tools: 5 });
        assert!(view.history.is_empty());
        assert!(view.history_error.as_deref().is_some_and(|cause| {
            serde_json::from_str::<serde_json::Value>(cause)
                .is_ok_and(|v| v["code"] == "storage_failed")
        }));
        assert!(view.detail.is_empty());
    }

    #[test]
    fn failed_engine_tools_read_keeps_its_cause() {
        let snapshot = crate::AppSnapshot::empty(1, crate::DaemonConnection::Ready);
        let mut topics = cache_with_control(ready_head());
        let request = orbit_core::reads::ReadRequest::EngineTools { cursor: None };
        assert!(topics.remember_read(
            &request,
            TopicId::Control,
            Knowledge::Absent {
                refusal: Refusal::StorageFailed {
                    operation: orbit_core::refusal::Operation::Storage,
                    cause: None,
                },
            }
        ));
        let view = project(&input(&snapshot, &topics)).expect("control head must project");
        assert!(view.engine_tools.is_empty());
        assert!(view.engine_tools_error.as_deref().is_some_and(|cause| {
            serde_json::from_str::<serde_json::Value>(cause)
                .is_ok_and(|v| v["code"] == "storage_failed")
        }));
        assert_eq!(view.engine, EngineSnapshot::Ready { tools: 5 });
    }

    fn projected_engine(
        engine: Knowledge<control::Engine>,
    ) -> Result<EngineSnapshot, &'static str> {
        let mut head = ready_head();
        if let TopicData::Control(Knowledge::Present { value, .. }) = &mut head {
            value.engine = engine;
        }
        let topics = cache_with_control(head);
        let local = AppSnapshot::empty(1, crate::DaemonConnection::Ready);
        project(&FeatureInput::new(&local, &topics))
            .map(|view| view.engine)
            .ok_or("control view missing")
    }

    #[test]
    fn engine_refusals_preserve_the_permission_and_unknown_state()
    -> Result<(), Box<dyn std::error::Error>> {
        assert_eq!(
            projected_engine(Knowledge::Absent {
                refusal: Refusal::PermissionNotGranted {
                    permission: Permission::Accessibility,
                }
            })?,
            EngineSnapshot::PermissionMissing {
                accessibility: true,
                screen_recording: false
            }
        );
        assert_eq!(
            projected_engine(Knowledge::Absent {
                refusal: Refusal::PermissionUnknown {
                    permission: Permission::ScreenRecording,
                }
            })?,
            EngineSnapshot::PermissionUnknown {
                accessibility: false,
                screen_recording: true
            }
        );
        assert_eq!(
            projected_engine(Knowledge::Present {
                value: control::Engine::Ready { tools: 42 },
                as_of_ms: None
            })?,
            EngineSnapshot::Ready { tools: 42 }
        );
        assert_eq!(
            projected_engine(Knowledge::Unknown {
                why: orbit_core::topics::Pending::NotObservedYet
            })?,
            EngineSnapshot::Unknown,
            "an unobserved engine is neither ready nor missing"
        );
        Ok(())
    }
}
