#![allow(clippy::unwrap_used)]
use orbit_app_core::{AppCommand, DaemonClient, ReceiptDelivery};
use orbit_core::protocol::Response;
use orbit_ipc::Paths;
use std::error::Error;
#[path = "support/fake_daemon.rs"]
mod fake_daemon;

#[tokio::test]
async fn local_order_receipt_names_the_frame_it_just_published() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let paths = Paths::resolve(Some(directory.path().to_path_buf()))?;
    let client = DaemonClient::connect(paths, fake_daemon::idle());
    let before = client.bootstrap().revision;
    let receipt = client
        .send(AppCommand::Local(
            orbit_app_core::features::local::LocalCommand::OnboardingDone,
        ))
        .await?;
    assert!(matches!(receipt.delivery, Some(ReceiptDelivery::Reflected)));
    assert_eq!(receipt.revision.epoch, before.epoch);
    assert!(
        receipt.revision.counter > before.counter,
        "the local order changed the snapshot but its receipt still named the old frame"
    );
    assert!(client.take_frames().iter().any(|frame| matches!(frame,
        orbit_app_core::features::FeatureFrame::Local { view, revision, .. }
            if view.onboarding_done && *revision == receipt.revision
    )));
    Ok(())
}

#[tokio::test]
async fn applied_order_reports_the_missing_topic_revision() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let paths = Paths::resolve(Some(directory.path().to_path_buf()))?;
    let (calls, mut incoming) = tokio::sync::mpsc::channel::<orbit_ipc::Call>(32);
    let endpoint = fake_daemon::channel(calls, orbit_ipc::TopicChannels::new());
    let daemon = tokio::spawn(async move {
        while let Some(call) = incoming.recv().await {
            let _ = call.reply.send(Response {
                v: 1,
                id: call.request.id,
                ok: true,
                status: None,
                error: None,
                refusal: None,
                epoch: Some("expected-epoch".into()),
                revisions: Some([(orbit_core::topics::TopicId::Status, 4)].into()),
            });
        }
    });
    let client = DaemonClient::connect(paths, endpoint.client.clone());
    let receipt = client
        .send(AppCommand::Daemon(Box::new(
            orbit_core::protocol::Command::PrivacyStop,
        )))
        .await?;
    assert!(matches!(
        receipt.delivery,
        Some(ReceiptDelivery::FrameNotObserved {
            topic: orbit_core::topics::TopicId::Status,
            expected_revision: 4,
            observed_revision: None,
            expected_epoch,
            observed_epoch: None,
            ..
        }) if expected_epoch == "expected-epoch"
    ));
    daemon.abort();
    Ok(())
}

#[tokio::test]
async fn typed_daemon_refusal_reaches_the_command_error() -> Result<(), Box<dyn Error>> {
    use orbit_app_core::CommandError;
    use orbit_core::refusal::Refusal;
    let directory = tempfile::tempdir()?;
    let paths = Paths::resolve(Some(directory.path().to_path_buf()))?;
    let (calls, mut incoming) = tokio::sync::mpsc::channel::<orbit_ipc::Call>(32);
    let endpoint = fake_daemon::channel(calls, orbit_ipc::TopicChannels::new());
    let daemon = tokio::spawn(async move {
        while let Some(call) = incoming.recv().await {
            let refusal = Refusal::NotPaired;
            let _ = call.reply.send(Response {
                v: 1,
                id: call.request.id,
                ok: false,
                status: None,
                error: Some(refusal.to_string()),
                refusal: Some(refusal),
                epoch: None,
                revisions: Some(Default::default()),
            });
        }
    });
    let client = DaemonClient::connect(paths, endpoint.client.clone());
    let command = AppCommand::Daemon(Box::new(orbit_core::protocol::Command::PrivacyStop));
    let error = client.send(command.clone()).await.unwrap_err();
    let error = CommandError::from_app_error(command, error);
    daemon.abort();
    assert_eq!(*error.refusal, Refusal::NotPaired);
    Ok(())
}

async fn refused_path(command: Option<AppCommand>, calendar: bool) -> Result<(), Box<dyn Error>> {
    use orbit_app_core::CommandError;
    use orbit_core::refusal::{Refusal, Resource};
    let refusal = Refusal::TooMany {
        resource: Resource::Subscriptions,
        count: 5,
        max: 4,
    };
    let directory = tempfile::tempdir()?;
    let paths = Paths::resolve(Some(directory.path().to_path_buf()))?;
    let (calls, mut incoming) = tokio::sync::mpsc::channel::<orbit_ipc::Call>(32);
    let endpoint = fake_daemon::channel(calls, orbit_ipc::TopicChannels::new());
    let sent = refusal.clone();
    let daemon = tokio::spawn(async move {
        while let Some(call) = incoming.recv().await {
            let refusal = sent.clone();
            let _ = call.reply.send(Response {
                v: 1,
                id: call.request.id,
                ok: false,
                status: None,
                error: Some(refusal.to_string()),
                refusal: Some(refusal),
                epoch: None,
                revisions: Some(Default::default()),
            });
        }
    });
    let client = DaemonClient::connect(paths, endpoint.client.clone());
    let result = match command.clone() {
        Some(command) => client.send(command).await.map(|_| ()),
        None => client.read_sessions_page(None).await.map(|_| ()),
    };
    daemon.abort();
    let error = result.unwrap_err();
    assert_eq!(
        error.code,
        if calendar {
            "calendar_error"
        } else {
            "daemon_error"
        }
    );
    assert_eq!(
        error.message,
        format!(
            "{}: {}",
            if calendar {
                "calendar_error"
            } else {
                "daemon_error"
            },
            refusal
        )
    );
    let error = CommandError::from_app_error(
        command.unwrap_or(AppCommand::Daemon(Box::new(
            orbit_core::protocol::Command::PrivacyStop,
        ))),
        error,
    );
    assert_eq!(*error.refusal, refusal);
    Ok(())
}

#[tokio::test]
async fn calendar_commands_preserve_typed_refusal_and_calendar_context()
-> Result<(), Box<dyn Error>> {
    refused_path(
        Some(AppCommand::Daemon(Box::new(
            orbit_core::protocol::Command::Calendar {
                request: orbit_core::calendar_control::CalendarCommand::RequestPermission,
            },
        ))),
        true,
    )
    .await?;
    refused_path(
        Some(AppCommand::Daemon(Box::new(
            orbit_core::protocol::Command::Calendar {
                request: orbit_core::calendar_control::CalendarCommand::Delete {
                    request_id: "delete-refused".into(),
                    delete: orbit_core::calendar::CalendarDelete {
                        calendar_item_id: "event".into(),
                        occurrence_start_ms: 1_792_058_400_000,
                        span: orbit_app_core::CalendarSpan::This,
                    },
                },
            },
        ))),
        true,
    )
    .await
}

#[tokio::test]
async fn calendar_read_preserves_typed_refusal_and_calendar_context() -> Result<(), Box<dyn Error>>
{
    refused_path(
        Some(AppCommand::Agenda(
            orbit_app_core::AgendaCommand::CalendarRefresh,
        )),
        true,
    )
    .await
}

#[tokio::test]
async fn voice_audio_preserves_typed_refusal() -> Result<(), Box<dyn Error>> {
    refused_path(
        Some(AppCommand::Assistant(
            orbit_app_core::AssistantCommand::VoiceNoteAudio {
                message_id: "voice".into(),
            },
        )),
        false,
    )
    .await
}

#[tokio::test]
async fn channel_refusals_preserve_recovery_legacy_and_size_fields() -> Result<(), Box<dyn Error>> {
    use orbit_core::refusal::{Refusal, Resource};
    let directory = tempfile::tempdir()?;
    let paths = Paths::resolve(Some(directory.path().to_owned()))?;
    let cases = [
        Refusal::Recovering {
            step: orbit_core::RecoveryStep::Snapshot,
            elapsed_ms: 123,
        },
        Refusal::Legacy {
            text: "El almacén se está recuperando: mensaje heredado".into(),
        },
        Refusal::TooLarge {
            resource: Resource::IpcResponse,
            bytes: 659457,
            max_bytes: 659456,
        },
    ];
    let (calls, mut incoming) = tokio::sync::mpsc::channel::<orbit_ipc::Call>(32);
    let endpoint = fake_daemon::channel(calls, orbit_ipc::TopicChannels::new());
    let sent = cases.clone();
    let actor = tokio::spawn(async move {
        for refusal in sent {
            let call = incoming.recv().await.unwrap();
            call.reply
                .send(Response {
                    v: 1,
                    id: call.request.id,
                    ok: false,
                    status: None,
                    error: Some(refusal.to_string()),
                    refusal: Some(refusal),
                    epoch: None,
                    revisions: None,
                })
                .unwrap();
        }
    });
    let client = DaemonClient::connect(paths, endpoint.client.clone());
    for refusal in cases {
        let error = client.read_sessions_page(None).await.unwrap_err();
        assert_eq!(error.refusal.as_deref(), Some(&refusal));
        assert_eq!(error.message, format!("daemon_error: {refusal}"));
    }
    actor.await?;
    Ok(())
}
