//! The control topic's projection: the engine with its cause,
//! the action in flight and the recent history, read from the runtime at the
//! end of the turn. The whole record and the tool catalogue are reads.
use super::{ProjectTopic, TopicSource};
use orbit_core::topics::{
    TopicData, TopicId,
    control::{self, engine_knowledge_with_permissions},
};

pub(super) struct Projector;

impl ProjectTopic for Projector {
    fn project(&self, source: &TopicSource<'_>) -> TopicData {
        let Some(runtime) = source.runtime else {
            return TopicData::not_observed(TopicId::Control);
        };
        let status = runtime.engine_status();
        let retry_at_ms = if matches!(
            &status,
            orbit_core::computer_control::engine::Status::Failed { .. }
        ) {
            runtime.features.computer.engine_retry_at_ms
        } else {
            None
        };
        let engine = engine_knowledge_with_permissions(
            status,
            runtime.engine_failure_cause(),
            runtime.state.capabilities().accessibility_state(),
            runtime.state.capabilities().screen_state(),
        );
        // The durable record is read on change. A failed read is unknown
        // history, not an empty record.
        let history = match runtime.control_history() {
            Ok(history) => orbit_core::topics::Knowledge::Present {
                value: control::History {
                    total: u64::try_from(history.len()).unwrap_or(u64::MAX),
                    recent: history.into_iter().take(control::RECENT_HISTORY).collect(),
                },
                as_of_ms: None,
            },
            Err(error) => orbit_core::topics::Knowledge::Absent {
                refusal: orbit_core::refusal::Refusal::StorageFailed {
                    operation: orbit_core::refusal::Operation::Storage,
                    cause: Some(error.to_string().into()),
                },
            },
        };
        TopicData::Control(orbit_core::topics::Knowledge::Present {
            value: control::Value {
                engine,
                retry_at_ms,
                action: runtime.active_control_action().cloned(),
                history,
                folders: Box::new(runtime.folders_knowledge()),
            },
            as_of_ms: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::audio::tests::fixture;
    use orbit_core::{
        computer_control::{ControlHistoryEntry, ControlOutcome},
        refusal::{Permission, Refusal},
        topics::Knowledge,
    };
    fn projected(runtime: &crate::daemon::Runtime) -> TopicData {
        let source = TopicSource {
            state: &runtime.state,
            runtime: Some(runtime),
            now_ms: 0,
        };
        Projector.project(&source)
    }

    #[test]
    fn unreported_permissions_are_published_as_unknown_not_as_denied() -> anyhow::Result<()> {
        let directory = tempfile::tempdir()?;
        let (mut runtime, _) = fixture(directory.path())?;
        {
            let fixture_value = serde_json::json!(orbit_core::PermissionState::Unknown);
            crate::daemon::authority_fixture::set(
                &mut runtime.state,
                "capabilities.accessibility_state",
                fixture_value,
            );
        }
        {
            let fixture_value = serde_json::json!(orbit_core::PermissionState::Unknown);
            crate::daemon::authority_fixture::set(
                &mut runtime.state,
                "capabilities.screen_state",
                fixture_value,
            );
        }
        // The fixture helper is connected and has not reported either
        // permission: the engine's cause is «unknown», which is neither a
        // macOS «no» nor «nothing observed yet».
        assert!(runtime.state.capabilities().native_helper());
        assert_eq!(
            engine_of(&runtime),
            Knowledge::Absent {
                refusal: Refusal::PermissionUnknown {
                    permission: Permission::ScreenAndAccessibility,
                },
            },
        );
        Ok(())
    }

    /// The topic-level knowledge is the head — the daemon always has one —
    /// and the three values live on the engine inside it.
    fn engine_of(runtime: &crate::daemon::Runtime) -> Knowledge<control::Engine> {
        let TopicData::Control(Knowledge::Present { value, .. }) = projected(runtime) else {
            panic!("the daemon always has a control head");
        };
        value.engine
    }

    #[test]
    fn a_permission_macos_is_known_to_deny_names_it() -> anyhow::Result<()> {
        let directory = tempfile::tempdir()?;
        let (mut runtime, _) = fixture(directory.path())?;
        {
            let fixture_value = serde_json::json!(orbit_core::PermissionState::Denied);
            crate::daemon::authority_fixture::set(
                &mut runtime.state,
                "capabilities.accessibility_state",
                fixture_value,
            );
        }
        {
            let fixture_value = serde_json::json!(orbit_core::PermissionState::Granted);
            crate::daemon::authority_fixture::set(
                &mut runtime.state,
                "capabilities.screen_state",
                fixture_value,
            );
        }
        assert_eq!(
            engine_of(&runtime),
            Knowledge::Absent {
                refusal: Refusal::PermissionDenied {
                    permission: Permission::Accessibility,
                },
            },
        );
        Ok(())
    }

    #[test]
    fn not_granted_is_not_called_denied() -> anyhow::Result<()> {
        let directory = tempfile::tempdir()?;
        let (mut runtime, _) = fixture(directory.path())?;
        {
            let fixture_value = serde_json::json!(orbit_core::PermissionState::NotGranted);
            crate::daemon::authority_fixture::set(
                &mut runtime.state,
                "capabilities.accessibility_state",
                fixture_value,
            );
        }
        {
            let fixture_value = serde_json::json!(orbit_core::PermissionState::Granted);
            crate::daemon::authority_fixture::set(
                &mut runtime.state,
                "capabilities.screen_state",
                fixture_value,
            );
        }
        assert_eq!(
            engine_of(&runtime),
            Knowledge::Absent {
                refusal: Refusal::PermissionNotGranted {
                    permission: Permission::Accessibility,
                },
            },
        );
        Ok(())
    }

    #[tokio::test]
    async fn a_ready_engine_publishes_its_tools_its_action_and_the_recent_record()
    -> anyhow::Result<()> {
        let directory = tempfile::tempdir()?;
        let (mut runtime, _) = fixture(directory.path())?;
        {
            let fixture_value = serde_json::json!(orbit_core::PermissionState::Granted);
            crate::daemon::authority_fixture::set(
                &mut runtime.state,
                "capabilities.accessibility_state",
                fixture_value,
            );
        }
        {
            let fixture_value = serde_json::json!(orbit_core::PermissionState::Granted);
            crate::daemon::authority_fixture::set(
                &mut runtime.state,
                "capabilities.screen_state",
                fixture_value,
            );
        }
        runtime.features.computer.declared_tools = Some(7);
        // Not connecting: a worker holding a counted engine is Ready.
        let generation = runtime.state.generation();
        runtime
            .features
            .computer
            .work
            .attach_test(generation, (), tokio::spawn(async {}));
        // Newest first, as `upsert_control_history` records them.
        let history: Vec<serde_json::Value> = (0..25)
            .rev()
            .map(|index| {
                serde_json::json!({
                    "operation_id": format!("op-{index:03}"),
                    "requested": "click",
                    "application": "Fixture",
                    "channel": "local",
                    "at_ms": 1_000 + index,
                    "outcome": "performed",
                })
            })
            .collect();
        std::fs::write(
            runtime.paths.root.join("computer-control.json"),
            serde_json::to_vec(&serde_json::json!({"schema": 1, "history": history}))?,
        )?;
        let action = ControlHistoryEntry::new(
            "op-live",
            "type_text",
            "Fixture",
            1_234,
            ControlOutcome::Unknown,
        )
        .and_then(|entry| entry.from_channel("local"))
        .map_err(|error| anyhow::anyhow!("{error}"))?;
        runtime.features.computer.active_general = Some(action);
        let TopicData::Control(Knowledge::Present { value, .. }) = projected(&runtime) else {
            anyhow::bail!("a ready engine must publish a present head");
        };
        assert_eq!(
            value.engine,
            Knowledge::Present {
                value: control::Engine::Ready { tools: 7 },
                as_of_ms: None,
            }
        );
        let Knowledge::Present { value: history, .. } = value.history else {
            anyhow::bail!("the valid record must be present");
        };
        assert_eq!(history.total, 25);
        assert_eq!(history.recent.len(), control::RECENT_HISTORY);
        assert_eq!(history.recent[0].operation_id, "op-024");
        assert_eq!(
            value
                .action
                .as_ref()
                .map(|action| action.operation_id.as_str()),
            Some("op-live"),
        );
        Ok(())
    }

    #[test]
    fn an_unreadable_history_is_not_reported_as_empty() -> anyhow::Result<()> {
        let directory = tempfile::tempdir()?;
        let (runtime, _) = fixture(directory.path())?;
        std::fs::write(runtime.paths.root.join("computer-control.json"), b"broken")?;
        let TopicData::Control(Knowledge::Present { value, .. }) = projected(&runtime) else {
            anyhow::bail!("a broken record must not hide the engine");
        };
        assert!(matches!(value.engine, Knowledge::Absent { .. }));
        let encoded = serde_json::to_value(value)?;
        assert_eq!(encoded["history"]["state"], "absent");
        assert_eq!(encoded["history"]["refusal"]["code"], "storage_failed");
        Ok(())
    }

    #[tokio::test]
    async fn a_remote_action_is_published_while_in_flight() -> anyhow::Result<()> {
        let directory = tempfile::tempdir()?;
        let (mut runtime, _) = fixture(directory.path())?;
        let mut board = super::super::TopicBoard::new("control-remote".into());
        let receiver = board.channels()[&TopicId::Control].subscribe();
        board.publish(&TopicSource {
            state: &runtime.state,
            runtime: Some(&runtime),
            now_ms: 0,
        });
        let action = ControlHistoryEntry::new(
            "remote-1",
            "launch_app",
            "Fixture",
            1_000,
            ControlOutcome::Unknown,
        )
        .map_err(|error| anyhow::anyhow!("{error}"))?
        .from_channel("deimos")
        .map_err(|error| anyhow::anyhow!("{error}"))?;
        runtime.features.computer.active_general = Some(action);
        board.state_changed();
        board.publish(&TopicSource {
            state: &runtime.state,
            runtime: Some(&runtime),
            now_ms: 0,
        });
        let TopicData::Control(Knowledge::Present { value, .. }) = &receiver.borrow().data else {
            anyhow::bail!("control head did not publish");
        };
        assert_eq!(
            value
                .action
                .as_ref()
                .map(|action| action.operation_id.as_str()),
            Some("remote-1")
        );
        Ok(())
    }

    #[tokio::test]
    async fn an_unbundled_engine_failure_is_published_from_the_maintenance_tick()
    -> anyhow::Result<()> {
        let directory = tempfile::tempdir()?;
        let (mut runtime, _) = fixture(directory.path())?;
        let before_ms = crate::daemon::now();
        {
            let fixture_value = serde_json::json!(orbit_core::PermissionState::Granted);
            crate::daemon::authority_fixture::set(
                &mut runtime.state,
                "capabilities.accessibility_state",
                fixture_value,
            );
        }
        {
            let fixture_value = serde_json::json!(orbit_core::PermissionState::Granted);
            crate::daemon::authority_fixture::set(
                &mut runtime.state,
                "capabilities.screen_state",
                fixture_value,
            );
        }
        let (work, mut events) = tokio::sync::mpsc::channel(4);
        runtime.work.work_events = work;
        let mut board = super::super::TopicBoard::new("control-unbundled".into());
        let receiver = board.channels()[&TopicId::Control].subscribe();
        board.publish(&TopicSource {
            state: &runtime.state,
            runtime: Some(&runtime),
            now_ms: 0,
        });
        let revision = receiver.borrow().revision;
        runtime.maintain_engine().await;
        assert!(events.try_recv().is_err(), "no process may start");
        board.state_changed();
        board.publish(&TopicSource {
            state: &runtime.state,
            runtime: Some(&runtime),
            now_ms: 1,
        });
        assert!(receiver.borrow().revision > revision);
        assert_eq!(
            engine_of(&runtime),
            Knowledge::Absent {
                refusal: Refusal::EngineUnavailable {
                    cause: orbit_core::refusal::EngineCause::InitializationFailed,
                }
            }
        );
        let TopicData::Control(Knowledge::Present { value, .. }) = projected(&runtime) else {
            anyhow::bail!("failed engine head disappeared");
        };
        // The retry is a later instant, not merely present.
        assert!(value.retry_at_ms.is_some_and(|at| at > before_ms));
        Ok(())
    }

    #[tokio::test]
    async fn a_missing_packaged_engine_reports_initialization_failed_after_its_work_result()
    -> anyhow::Result<()> {
        let directory = tempfile::tempdir()?;
        let (mut runtime, _) = fixture(directory.path())?;
        let bundle = directory.path().join(orbit_core::orbit_bundle::NAME);
        let daemon =
            orbit_core::orbit_bundle::member_path(&bundle, orbit_core::orbit_bundle::Member::App)
                .ok_or_else(|| anyhow::anyhow!("synthetic daemon route"))?;
        std::fs::create_dir_all(
            daemon
                .parent()
                .ok_or_else(|| anyhow::anyhow!("daemon directory"))?,
        )?;
        std::fs::write(&daemon, b"synthetic daemon")?;
        runtime.features.computer.daemon_executable = Some(daemon.clone());
        let missing_driver = crate::cua::resolve_driver_path("", &daemon)?;
        assert!(
            !missing_driver.exists(),
            "fixture cannot launch a host driver"
        );
        let before_ms = crate::daemon::now();
        {
            let fixture_value = serde_json::json!(orbit_core::PermissionState::Granted);
            crate::daemon::authority_fixture::set(
                &mut runtime.state,
                "capabilities.accessibility_state",
                fixture_value,
            );
        }
        {
            let fixture_value = serde_json::json!(orbit_core::PermissionState::Granted);
            crate::daemon::authority_fixture::set(
                &mut runtime.state,
                "capabilities.screen_state",
                fixture_value,
            );
        }
        let (work, mut events) = tokio::sync::mpsc::channel(4);
        runtime.work.work_events = work;
        runtime.maintain_engine().await;
        assert!(matches!(
            engine_of(&runtime),
            Knowledge::Present {
                value: control::Engine::Starting,
                ..
            }
        ));
        let event = tokio::time::timeout(std::time::Duration::from_secs(5), events.recv())
            .await?
            .ok_or_else(|| anyhow::anyhow!("missing engine result"))?;
        runtime.work_event(event).await?;
        assert_eq!(
            engine_of(&runtime),
            Knowledge::Absent {
                refusal: Refusal::EngineUnavailable {
                    cause: orbit_core::refusal::EngineCause::InitializationFailed
                }
            }
        );
        let TopicData::Control(Knowledge::Present { value, .. }) = projected(&runtime) else {
            anyhow::bail!("failed engine head disappeared");
        };
        assert!(value.retry_at_ms.is_some_and(|at| at > before_ms));
        Ok(())
    }
}
