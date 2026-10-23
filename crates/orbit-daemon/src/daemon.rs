//! Single state owner. Native, network and IPC feed facts into this actor.
use crate::{
    config::{Paths, Settings, get_secret, storage_key},
    hub::{self, HubEvent},
    ipc::{self, Call},
    native,
};
use anyhow::{Context, Result, bail};
use orbit_core::{
    AccountSummary, Approval, CapturePhase, Connection, DurableState, Effect, Event, State, decide,
    failure::Outcome,
    protocol::{Command, DaemonStatus, Response, TopicResponse, VoiceNoteAudioResponse},
    refusal::{Refusal, Service},
};
use orbit_store::{Job, JobState, Store};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    time::Duration,
};
use tokio::{
    sync::{mpsc, watch},
    task::JoinHandle,
};

#[cfg(test)]
mod subscription_tests {
    use super::*;
    use anyhow::bail;
    use orbit_core::{RecoveryStatus, topics::TopicId};
    use std::time::Duration;

    fn fixture() -> Result<(tempfile::TempDir, Paths)> {
        let directory = tempfile::tempdir()?;
        let paths = Paths::resolve(Some(directory.path().join("orbit")))?;
        paths.save_settings(&Settings {
            hub_url: Some("http://127.0.0.1:0".into()),
            ..Settings::default()
        })?;
        Ok((directory, paths))
    }

    async fn ready(paths: &Paths, daemon: &mut JoinHandle<Result<()>>) -> Result<()> {
        loop {
            if paths.socket.exists()
                && let Ok(response) = ipc::request(paths, Command::Status).await
                && response
                    .status
                    .is_some_and(|state| matches!(state.recovery, RecoveryStatus::Ready))
            {
                return Ok(());
            }
            tokio::select! {
                result = &mut *daemon => {
                    result??;
                    bail!("daemon stopped before it became ready")
                }
                () = tokio::time::sleep(Duration::from_millis(10)) => {}
            }
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn shutdown_records_numeric_turn_counts_without_turn_content() -> Result<()> {
        tokio::task::LocalSet::new()
            .run_until(async {
                let (_directory, paths) = fixture()?;
                let daemon_paths = paths.clone();
                let mut daemon = tokio::task::spawn_local(async move {
                    run_with_helper_inner(daemon_paths, true, None, None).await
                });
                ready(&paths, &mut daemon).await?;
                ipc::request(&paths, Command::Shutdown).await?;
                daemon.await??;
                let log = std::fs::read_to_string(paths.root.join("logs/daemon.log"))?;
                let summary = log
                    .lines()
                    .filter_map(|line| serde_json::from_str::<Value>(line).ok())
                    .find(|line| line["event"] == "loop_turns")
                    .context("shutdown omitted loop branch counts")?;
                let fields = summary["fields"].as_object().context("counts missing")?;
                let names = [
                    "signals",
                    "commands",
                    "native",
                    "hub",
                    "work",
                    "notes",
                    "topic_due",
                    "tick",
                    "capture_tick",
                    "elapsed_ms",
                ];
                assert_eq!(fields.len(), names.len());
                for name in names {
                    let count = fields
                        .get(name)
                        .and_then(Value::as_str)
                        .and_then(|value| value.parse::<u64>().ok());
                    assert!(count.is_some(), "{name} must contain only a count");
                }
                assert!(
                    fields["commands"]
                        .as_str()
                        .context("commands")?
                        .parse::<u64>()?
                        >= 2
                );
                assert_eq!(fields["signals"], "0");
                assert_eq!(fields["native"], "0");
                Ok(())
            })
            .await
    }

    #[tokio::test(flavor = "current_thread")]
    async fn command_receipt_revisions_are_already_published_by_the_actor() -> Result<()> {
        tokio::task::LocalSet::new()
            .run_until(async {
                let (_directory, paths) = fixture()?;
                let daemon_paths = paths.clone();
                let mut daemon = tokio::task::spawn_local(async move {
                    run_with_helper_inner(daemon_paths, true, None, None).await
                });
                ready(&paths, &mut daemon).await?;
                let (_initial, mut subscription) =
                    ipc::subscribe_topics(&paths, vec![TopicId::Status, TopicId::Control]).await?;
                for _ in 0..2 {
                    let _initial_frame = subscription.next_topic().await?;
                }
                let receipt = ipc::request(&paths, Command::PrivacyStop).await?;
                assert!(receipt.ok);
                let revisions = receipt
                    .revisions
                    .context("command reply omitted revisions")?;
                let mut seen = std::collections::BTreeSet::new();
                for _ in 0..2 {
                    let frame = subscription.next_topic().await?;
                    let id = frame.data.id();
                    assert_eq!(Some(&frame.revision), revisions.get(&id));
                    let published = serde_json::to_value(&frame.data)?;
                    if id == TopicId::Status {
                        assert_eq!(published["knowledge"]["value"]["privacy_stopped"], true);
                    } else if id == TopicId::Control {
                        assert_eq!(
                            published["knowledge"]["value"]["engine"]["refusal"]["code"],
                            "privacy_stopped"
                        );
                    } else {
                        bail!("unexpected topic in command subscription: {id:?}");
                    }
                    seen.insert(id);
                }
                assert_eq!(
                    seen,
                    [TopicId::Status, TopicId::Control].into_iter().collect()
                );
                ipc::request(&paths, Command::Shutdown).await?;
                daemon.await??;
                Ok(())
            })
            .await
    }
}

#[cfg(test)]
mod privacy_read_tests {
    use super::*;
    use orbit_core::topics::Knowledge;

    #[tokio::test]
    async fn privacy_stop_replies_to_a_read_in_flight() -> Result<(), Box<dyn std::error::Error>> {
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = tokio::sync::oneshot::channel::<()>();
        let (epoch_tx, epoch_rx) = watch::channel(0_u64);
        let task = tokio::spawn(async move {
            let _ = started_tx.send(());
            cancellable_remote_read(
                async {
                    let _ = release_rx.await;
                    orbit_core::topics::Knowledge::Unknown {
                        why: orbit_core::topics::Pending::NotObservedYet,
                    }
                },
                epoch_rx,
                0,
            )
            .await
        });
        assert!(started_rx.await.is_ok());
        let mut was_stopped = false;
        publish_privacy_epoch(true, &mut was_stopped, &epoch_tx);
        let _ = release_tx.send(());
        assert!(
            matches!(
                task.await?,
                Knowledge::Absent {
                    refusal: Refusal::PrivacyStopped { .. }
                }
            ),
            "an in-flight read must answer with the privacy refusal"
        );
        Ok(())
    }

    #[tokio::test]
    async fn privacy_stop_refuses_a_local_read_while_its_worker_keeps_the_slot()
    -> Result<(), Box<dyn std::error::Error>> {
        let slots = std::sync::Arc::new(tokio::sync::Semaphore::new(1));
        let permit = slots.clone().try_acquire_owned()?;
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = std::sync::mpsc::sync_channel::<()>(0);
        let (epoch_tx, epoch_rx) = watch::channel(0_u64);
        let task = tokio::spawn(async move {
            cancellable_remote_read(
                async move {
                    match tokio::task::spawn_blocking(move || {
                        let _permit = permit;
                        if started_tx.send(()).is_err() {
                            return;
                        }
                        if let Err(error) = release_rx.recv() {
                            eprintln!("local note test worker lost its release: {error}");
                        }
                    })
                    .await
                    {
                        Ok(()) => Knowledge::Unknown {
                            why: orbit_core::topics::Pending::NotObservedYet,
                        },
                        Err(error) => Knowledge::Absent {
                            refusal: Refusal::StorageFailed {
                                operation: orbit_core::refusal::Operation::Storage,
                                cause: Some(error.to_string().into()),
                            },
                        },
                    }
                },
                epoch_rx,
                0,
            )
            .await
        });
        started_rx.await?;
        let mut was_stopped = false;
        publish_privacy_epoch(true, &mut was_stopped, &epoch_tx);
        let answer = task.await?;
        let held_after_privacy_reply = slots.available_permits() == 0;
        release_tx.send(())?;
        let released =
            tokio::time::timeout(std::time::Duration::from_secs(5), slots.acquire_owned())
                .await??;
        drop(released);
        assert!(
            held_after_privacy_reply,
            "the local worker released its slot before finishing"
        );
        assert!(matches!(
            answer,
            Knowledge::Absent {
                refusal: Refusal::PrivacyStopped { .. }
            }
        ));
        Ok(())
    }
}

#[cfg(test)]
mod ipc_tests {
    use super::*;

    #[test]
    fn abandoned_reply_records_command_once_without_conversation() -> anyhow::Result<()> {
        let directory = tempfile::tempdir()?;
        let diagnostics = crate::diagnostics::DiagnosticLog::in_root(directory.path());
        let (sender, receiver) = tokio::sync::oneshot::channel::<()>();
        drop(receiver);
        record_ipc_reply_loss(
            sender.send(()),
            &Command::Chat {
                attachments: Vec::new(),
                text: "SECRET_FIXTURE_CONVERSATION".into(),
                primary: false,
            },
            Some(&diagnostics),
        );
        let log = std::fs::read_to_string(diagnostics.path())?;
        assert_eq!(log.matches("ipc_reply_receiver_gone").count(), 1, "{log}");
        assert!(log.contains("\"command\":\"chat\""), "{log}");
        assert!(!log.contains("SECRET_FIXTURE_CONVERSATION"), "{log}");
        Ok(())
    }
}

#[cfg(test)]
mod refusal_tests {
    use super::*;
    use orbit_core::refusal::Permission;

    #[test]
    fn typed_cause_survives_anyhow_context_and_unknown_names_its_internal_cause() {
        use orbit_core::refusal::Refusal as Core;
        for permission in [Permission::Microphone, Permission::ScreenRecording] {
            for (cause, expected) in [
                (
                    Core::PermissionDenied { permission },
                    Refusal::PermissionDenied { permission },
                ),
                (
                    Core::PermissionNotGranted { permission },
                    Refusal::PermissionNotGranted { permission },
                ),
                (
                    Core::PermissionUnknown { permission },
                    Refusal::PermissionUnknown { permission },
                ),
            ] {
                assert_eq!(refusal_from_error(&cause.into()), expected);
            }
        }

        let error = anyhow::Error::new(orbit_core::refusal::Refusal::PermissionUnknown {
            permission: Permission::Microphone,
        })
        .context("orden rechazada");
        assert_eq!(
            refusal_from_error(&error),
            Refusal::PermissionUnknown {
                permission: orbit_core::refusal::Permission::Microphone
            }
        );
        let upstream = anyhow::Error::new(crate::retry::Refused(403)).context("send");
        assert_eq!(
            refusal_from_error(&upstream),
            Refusal::UpstreamFailed {
                service: Service::Hub,
                status: Some(403)
            }
        );
        let missing = anyhow::Error::new(orbit_store::Error::NotFound).context("lectura");
        assert!(matches!(
            refusal_from_error(&missing),
            Refusal::StorageFailed {
                operation: orbit_core::refusal::Operation::Storage,
                ..
            }
        ));
        let unknown = anyhow::anyhow!("synthetic cause");
        assert_eq!(
            refusal_from_error(&unknown),
            Refusal::Internal {
                cause: orbit_core::refusal::InternalCause::DaemonOperation {
                    error: "synthetic cause".into()
                }
            }
        );
        use orbit_core::refusal::{InternalCause, Operation};
        let hub = Refusal::HubUnreachable { since_ms: None };
        assert_eq!(refusal_from_error(&anyhow::Error::new(hub.clone())), hub);
        for error in [
            orbit_store::Error::NotFound,
            orbit_store::Error::Invalid("synthetic"),
        ] {
            let cause = error.to_string();
            assert_eq!(
                refusal_from_error(&error.into()),
                Refusal::StorageFailed {
                    operation: Operation::Storage,
                    cause: Some(cause.into())
                }
            );
        }
        let error = anyhow::anyhow!("synthetic error");
        assert_eq!(
            refusal_from_error(&error),
            Refusal::Internal {
                cause: InternalCause::DaemonOperation {
                    error: error.to_string()
                }
            }
        );
    }
}

mod actions;
mod ipc_replies;
mod main_loop;
mod run;
mod runtime;
mod runtime_build;
use ipc_replies::*;
#[cfg(test)]
use run::run_with_helper_inner;
pub use run::{run, run_embedded, run_with_helper};
pub(crate) use runtime::MAX_PENDING_NATIVE;
use runtime::*;
mod agent_config;
mod assistant_files;
mod attachments;
mod audio;
mod audio_sources;
mod bridge;
mod calendar;
mod call;
mod call_detection;
mod call_recovery;
mod capture_lane;
mod capture_link;
mod capture_record;
mod commands;
mod computer_control;
mod cut_recovery;
mod finalize;
mod hub_account;
mod hub_events;
mod hub_link;
mod interactions;
mod kernel;
mod links;
mod loop_lag;
mod meeting_question;
mod meeting_runtime;
mod native_events;
mod native_host;
mod read_dispatch;
mod reads;
mod recording_notes;
mod retire;
mod sealing;
mod shutdown;
use bridge::{BridgeAsk, BridgeReleaseReason};
use kernel::DurableSnapshot;
pub use shutdown::{SHUTDOWN_BUDGET, SNAPSHOT_RECEIPT_GRACE};
mod helper_waits;
mod sources;
mod speech;
mod startup;
mod stops;
mod store_actor;
pub(crate) mod ticketed;
mod timed_reader;
mod topics;
mod track_gaps;
mod track_signal;
mod transfers;
mod turn;
mod turn_watch;
mod upload_outcome;
mod voice_dispatch;
mod voice_notes;
mod window_attachment;
use turn::refusal_notice;
mod work;

#[cfg(test)]
mod agent_config_tests;
#[cfg(test)]
mod agents_tests;
#[cfg(test)]
mod attachments_tests;
#[cfg(test)]
mod capability_notice_tests;
#[cfg(test)]
mod commands_tests;
#[cfg(test)]
mod helper_liveness_tests;
#[cfg(test)]
mod pairing_session_tests;
#[cfg(test)]
mod state_observation_tests;
#[cfg(test)]
mod track_failure_tests;
#[cfg(test)]
mod voice_note_tests;

#[cfg(test)]
mod hub_backpressure_tests;

#[cfg(test)]
mod native_bridge_tests;

#[cfg(test)]
mod capture_isolation_tests;
#[cfg(test)]
mod chunk_delivery_tests;
#[cfg(test)]
mod meeting_continuity_tests;
#[cfg(test)]
mod offline_voice_note_tests;
#[cfg(test)]
mod restart_closure_tests;
#[cfg(test)]
mod seal_cause_tests;

#[cfg(test)]
mod privacy_capture_tests;

#[cfg(test)]
mod scheduling_tests;

#[cfg(test)]
mod error_report_tests;

#[cfg(test)]
pub(crate) mod authority_fixture;
#[cfg(test)]
mod authority_retirement_tests;
#[cfg(test)]
mod store_actor_tests;
#[cfg(test)]
mod turn_watch_tests;

#[cfg(test)]
mod test_store;

fn now() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn account_summary(settings: &Settings) -> AccountSummary {
    AccountSummary {
        hub_url: settings.hub_url.clone().map(Into::into),
        device_name: Some(settings.device_name.clone().into()),
        device_id_short: settings
            .device_id
            .as_deref()
            .map(|device_id| device_id.chars().take(8).collect::<String>().into()),
    }
}

/// The applications the Mac attached to a meeting hotkey press.
///
/// Anything malformed is dropped rather than defaulted: a row with a missing
/// name or a zero PID would travel on as a real candidate and could end up
/// chosen as the thing to record.
fn meeting_applications(event: &Value) -> Vec<orbit_core::meeting_audio::RunningApplication> {
    event["applications"]
        .as_array()
        .map(|rows| {
            rows.iter()
                .filter_map(|row| {
                    let application_pid = i32::try_from(row["application_pid"].as_i64()?).ok()?;
                    let bundle_id = row["bundle_id"].as_str()?;
                    let name = row["name"].as_str()?;
                    if application_pid <= 0 || bundle_id.is_empty() || name.is_empty() {
                        return None;
                    }
                    Some(orbit_core::meeting_audio::RunningApplication {
                        application_pid,
                        bundle_id: bundle_id.to_owned(),
                        name: name.to_owned(),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

async fn recover_store(store: &mut Store) -> Result<orbit_store::RecoveryReport> {
    let mut scan = store.begin_recovery()?;
    let mut total = orbit_store::RecoveryReport::default();
    loop {
        let batch = store.recover_next(&mut scan, 500)?;
        total.inspected += batch.inspected;
        total.recovered += batch.recovered;
        total.invalid += batch.invalid;
        total.missing += batch.missing;
        total.incomplete_temporary += batch.incomplete_temporary;
        if !batch.limit_reached {
            return Ok(total);
        }
        // No capture, transfer or position restoration before this reaches EOF.
        tokio::task::yield_now().await;
    }
}
fn id() -> String {
    uuid::Uuid::new_v4().to_string()
}

#[cfg(test)]
mod meeting_application_tests {
    //! Fixture setup only; production code above never panics.
    #![allow(clippy::expect_used)]
    use super::*;
    use serde_json::json;

    #[test]
    fn a_hotkey_press_carries_what_the_mac_had_running() {
        let event = json!({
            "event": "hotkey",
            "action": "meeting_toggle",
            "applications": [
                {"application_pid": 42, "bundle_id": "us.zoom.xos", "name": "zoom.us"}
            ]
        });
        let applications = meeting_applications(&event);
        assert_eq!(applications.len(), 1);
        assert_eq!(applications[0].application_pid, 42);
    }

    #[test]
    fn a_press_without_applications_chooses_nothing_instead_of_failing() {
        // Every earlier release sent exactly this, and an older Orbit.app paired
        // with a newer daemon still will.
        let event = json!({"event": "hotkey", "action": "meeting_toggle"});
        assert!(meeting_applications(&event).is_empty());
        assert_eq!(
            orbit_core::meeting_audio::choose(&meeting_applications(&event)),
            orbit_core::meeting_audio::AudioChoice::NoCandidate
        );
    }

    #[test]
    fn a_malformed_row_is_dropped_rather_than_recorded_by_accident() {
        // A blank name or a zero PID would travel on as a real candidate and
        // could end up being the thing an hour of audio is captured from.
        let event = json!({
            "applications": [
                {"application_pid": 0, "bundle_id": "us.zoom.xos", "name": "zoom.us"},
                {"application_pid": 7, "bundle_id": "", "name": "sin bundle"},
                {"application_pid": 8, "bundle_id": "us.zoom.xos", "name": ""},
                {"application_pid": "nueve", "bundle_id": "us.zoom.xos", "name": "zoom.us"},
                {"bundle_id": "us.zoom.xos", "name": "zoom.us"}
            ]
        });
        assert!(meeting_applications(&event).is_empty());
    }
}

#[cfg(test)]
#[allow(clippy::expect_used)]
fn topic(runtime: &Runtime, id: orbit_core::topics::TopicId) -> orbit_core::topics::TopicData {
    let mut board = topics::TopicBoard::new("fixture".into());
    let receiver = board
        .channels()
        .get(&id)
        .expect("registered topic")
        .subscribe();
    board.publish_command(&topics::TopicSource {
        state: &runtime.state,
        runtime: Some(runtime),
        now_ms: 0,
    });
    receiver.borrow().data.clone()
}

#[cfg(test)]
mod response_budget_tests {
    use super::*;

    #[test]
    fn oversized_page_is_absent_before_any_transport_receives_it() -> Result<()> {
        use orbit_core::{
            reads::ReadId,
            refusal::{Refusal, Resource},
            topics::{Knowledge, Page, TOPIC_ENVELOPE_BYTES},
        };
        let answer = Knowledge::Present {
            value: Page {
                items: vec![serde_json::json!({"text": "\\".repeat(650 * 1024)})],
                next: None,
                total: 1,
                revision: 3,
                store_version: None,
            },
            as_of_ms: None,
        };
        let original = TopicResponse {
            v: 1,
            id: "page".into(),
            ok: true,
            revisions: BTreeMap::new(),
            read: Some(ReadId::NotesPage),
            answer: Some(answer.clone()),
            refusal: None,
        };
        let bytes = serde_json::to_vec(&original)?.len() + 1;
        let response = topic_response(
            "page".into(),
            BTreeMap::new(),
            Some(ReadId::NotesPage),
            Some(answer),
        );
        assert!(
            response.answer
                == Some(Knowledge::Absent {
                    refusal: Refusal::TooLarge {
                        resource: Resource::IpcResponse,
                        bytes: bytes as u64,
                        max_bytes: (ReadId::NotesPage.max_bytes() + TOPIC_ENVELOPE_BYTES) as u64,
                    }
                })
        );
        Ok(())
    }
}
