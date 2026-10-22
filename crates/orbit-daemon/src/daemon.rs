//! Single state owner. Native, network and IPC feed facts into this actor.
use crate::{
    capture::CaptureBuffer,
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

fn cancel_remote_reads(reads: &mut Vec<JoinHandle<()>>) {
    for task in reads.drain(..) {
        task.abort();
    }
}

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

fn privacy_stopped_read()
-> orbit_core::topics::Knowledge<orbit_core::topics::Page<serde_json::Value>> {
    orbit_core::topics::Knowledge::Absent {
        refusal: Refusal::PrivacyStopped { since_ms: None },
    }
}

async fn cancellable_remote_read(
    read: impl std::future::Future<
        Output = orbit_core::topics::Knowledge<orbit_core::topics::Page<serde_json::Value>>,
    >,
    mut privacy_epoch: watch::Receiver<u64>,
    started_at_epoch: u64,
) -> orbit_core::topics::Knowledge<orbit_core::topics::Page<serde_json::Value>> {
    if *privacy_epoch.borrow() != started_at_epoch {
        return privacy_stopped_read();
    }
    tokio::select! {
        _ = privacy_epoch.changed() => privacy_stopped_read(),
        answer = read => {
            if *privacy_epoch.borrow() != started_at_epoch { privacy_stopped_read() } else { answer }
        }
    }
}

fn publish_privacy_epoch(stopped: bool, was_stopped: &mut bool, epoch: &watch::Sender<u64>) {
    if stopped && !*was_stopped {
        epoch.send_modify(|value| *value = value.wrapping_add(1));
    }
    *was_stopped = stopped;
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

fn refusal_from_error(error: &anyhow::Error) -> Refusal {
    for cause in error.chain() {
        if let Some(refusal) = cause.downcast_ref::<Refusal>() {
            return refusal.clone();
        }
        if let Some(refusal) = cause.downcast_ref::<crate::retry::Refused>() {
            return Refusal::UpstreamFailed {
                service: Service::Hub,
                status: Some(refusal.0),
            };
        }
    }
    if error.chain().any(|cause| cause.is::<orbit_store::Error>()) {
        return Refusal::StorageFailed {
            operation: orbit_core::refusal::Operation::Storage,
            cause: Some(error.to_string().into()),
        };
    }
    Refusal::Internal {
        cause: orbit_core::refusal::InternalCause::DaemonOperation {
            error: error.to_string(),
        },
    }
}

fn ipc_command_name(command: &Command) -> String {
    serde_json::to_value(command)
        .ok()
        .and_then(|value| value["command"].as_str().map(str::to_owned))
        .unwrap_or_else(|| "command_serialization_failed".into())
}

fn command_response(
    id: String,
    status: Option<DaemonStatus>,
    refusal: Option<orbit_core::refusal::Refusal>,
    epoch: Option<String>,
    revisions: BTreeMap<orbit_core::topics::TopicId, u64>,
) -> Response {
    Response {
        v: 1,
        id,
        ok: refusal.is_none(),
        status,
        error: refusal.as_ref().map(ToString::to_string),
        refusal,
        epoch,
        revisions: Some(revisions),
    }
}

fn topic_response(
    id: String,
    revisions: BTreeMap<orbit_core::topics::TopicId, u64>,
    read: Option<orbit_core::reads::ReadId>,
    answer: Option<orbit_core::topics::Knowledge<orbit_core::topics::Page<Value>>>,
) -> TopicResponse {
    ipc::bound_topic_response(TopicResponse {
        v: 1,
        id,
        ok: true,
        revisions,
        read,
        answer,
        refusal: None,
    })
}

fn record_ipc_reply_loss<T>(
    result: std::result::Result<(), T>,
    command: &Command,
    diagnostics: Option<&crate::diagnostics::DiagnosticLog>,
) {
    if result.is_err()
        && let Some(diagnostics) = diagnostics
    {
        diagnostics.record(
            "ipc_reply_receiver_gone",
            &[("command", &ipc_command_name(command))],
        );
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
                text: "SECRET_FIXTURE_CONVERSATION".into(),
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

/// How many in-flight helper command IDs the daemon and bridge retain. This
/// protects the daemon and bridge's memory from a helper that stops replying
/// or leaks correlation IDs. In the bridge it only decides whether a reply is
/// labelled with its command name (`_bridge_command`); every reply is still
/// forwarded. In the daemon it also decides whether a late reply that is not
/// a capture reply (those are named by the capture registry) still has its
/// effect or is logged as unpaired. The helper runs 1 command at a time and
/// queues up to 32, answering anything beyond that at once, so 64 slots cover
/// its whole queue plus a burst. `playback`, `playback_clip` and `audio_stop`
/// skip that queue (`NativeCommandRouter.swift`, ~:84-87), so they can be in
/// flight beyond it; `audio_stop` is still named by the capture registry.
pub(crate) const MAX_PENDING_NATIVE: usize = 64;

/// Wake-up notices the store actor and the upload workers may have waiting for
/// the daemon's select loop while it is busy with another event. What it
/// protects: the loop's own memory and its freedom to come back — a notice
/// only has to exist for the loop to be woken, so 16 slots are slack for the
/// work results of one busy turn (seal receipts, upload settlements, vertical
/// completions), not a throughput size. What it is not: it is not an audio
/// queue (PCM travels in the store actor's own ordered lanes) and not a
/// durability bound — a `Sealed` wake-up that does not fit is dropped while
/// its receipt stays recorded in the actor, whose collector applies it within
/// the 1 s maintenance tick, or within `SEAL_RETRY_DELAY` after a failed seal;
/// a `StoreProblem` notice never takes that drop path (it waits for room on
/// the writer). The writer never blocks on this channel,
/// so nothing may treat it as backpressure either.
const WORK_EVENTS_CAPACITY: usize = 16;

mod actions;
mod audio;
mod audio_sources;
mod bridge;
mod calendar;
mod call;
mod call_detection;
mod call_recovery;
mod commands;
mod computer_control;
mod finalize;
mod hub_account;
mod hub_events;
mod hub_link;
mod interactions;
mod kernel;
mod links;
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
pub use shutdown::{SHUTDOWN_BUDGET, SNAPSHOT_RECEIPT_GRACE};
mod sources;
mod speech;
mod startup;
mod stops;
mod store_actor;
pub(crate) mod ticketed;
mod topics;
mod track_signal;
mod turn;
use turn::refusal_notice;
mod work;

#[cfg(test)]
mod agents_tests;
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
mod voice_note_tests;

#[cfg(test)]
mod hub_backpressure_tests;

#[cfg(test)]
mod native_bridge_tests;

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
mod test_store;

enum WorkResult {
    Control(ControlWork),
    Capture(CaptureWork),
    Store(StoreWork),
    Problem {
        ticket: ticketed::WorkTicket,
        message: String,
    },
    SpeechAvailability {
        ticket: ticketed::WorkTicket,
        available: bool,
    },
    BridgeCaptureUpload {
        ticket: ticketed::WorkTicket,
        request_id: String,
        capture: bridge::PendingCapture,
        outcome: bridge::CaptureUploadOutcome,
    },
}

impl WorkResult {
    /// The op of the turn that started this work; a result with no
    /// ticket gets a new one.
    fn op(&self) -> orbit_core::notice::Op {
        let ticket = match self {
            Self::Control(ControlWork::Computer { ticket, .. })
            | Self::Capture(
                CaptureWork::VoiceProblem { ticket, .. } | CaptureWork::Transcribed { ticket, .. },
            )
            | Self::Store(
                StoreWork::Uploaded { ticket, .. }
                | StoreWork::UploadSettled { ticket, .. }
                | StoreWork::ChunkPruned { ticket, .. },
            )
            | Self::Problem { ticket, .. }
            | Self::SpeechAvailability { ticket, .. }
            | Self::BridgeCaptureUpload { ticket, .. } => Some(ticket),
            Self::Store(StoreWork::Sealed { .. } | StoreWork::StoreProblem { .. }) => None,
        };
        ticket.map_or_else(orbit_ipc::new_op, |ticket| ticket.op().clone())
    }
}

async fn deliver_work(sender: &mpsc::Sender<WorkResult>, result: WorkResult) {
    let kind = match &result {
        WorkResult::Control(ControlWork::Computer { .. }) => "computer",
        WorkResult::Capture(CaptureWork::VoiceProblem { .. }) => "voice_problem",
        WorkResult::Capture(CaptureWork::Transcribed { .. }) => "transcription",
        WorkResult::Store(StoreWork::Uploaded { .. }) => "upload",
        WorkResult::Store(StoreWork::UploadSettled { .. }) => "upload_settlement",
        WorkResult::Store(StoreWork::ChunkPruned { .. }) => "chunk_cleanup",
        WorkResult::Store(StoreWork::Sealed { .. }) => "seal",
        WorkResult::Store(StoreWork::StoreProblem { .. }) => "store_problem",
        WorkResult::Problem { .. } => "problem",
        WorkResult::SpeechAvailability { .. } => "speech_availability",
        WorkResult::BridgeCaptureUpload { .. } => "bridge_capture_upload",
    };
    if sender.send(result).await.is_err() {
        eprintln!("work_result_receiver_closed kind={kind}");
    }
}

enum ControlWork {
    Computer {
        ticket: ticketed::WorkTicket,
        event: Box<Result<computer_control::ComputerCompletion>>,
    },
}

enum CaptureWork {
    VoiceProblem {
        ticket: ticketed::WorkTicket,
        operation_id: String,
        failure: orbit_core::VoiceTranscriptionFailure,
    },
    Transcribed {
        ticket: ticketed::WorkTicket,
        source: audio::VoiceSource,
        text: String,
    },
}

enum StoreWork {
    ChunkPruned {
        ticket: ticketed::WorkTicket,
        result: Result<()>,
    },
    Uploaded {
        ticket: ticketed::WorkTicket,
        job_id: String,
        result: Result<()>,
    },
    UploadSettled {
        ticket: ticketed::WorkTicket,
        succeeded: bool,
        error: Option<String>,
        result: Result<()>,
    },
    Sealed {
        recording: String,
        sequence: u64,
    },
    StoreProblem {
        message: String,
    },
}

struct Runtime {
    closing: bool,
    /// A timed-out privacy retirement keeps its durable tail owed to the
    /// next maintenance turn after the writer recovers.
    privacy_retirement_owed: bool,
    paths: Paths,
    settings: Settings,
    development: bool,
    state: State,
    reader: orbit_store::ReadOnlyStore,
    /// Reads voice-note audio off the actor; see `reads::VoiceAudioReader`.
    voice_audio_reader: reads::VoiceAudioReader,
    store_actor: store_actor::StoreActor,
    /// `None` only in fixtures that assert nothing about the trail.
    diagnostics: Option<crate::diagnostics::DiagnosticLog>,
    /// The local report log; `None` in fixtures, like `diagnostics`.
    report_log: Option<orbit_ipc::report::ReportLog>,
    /// The Mac's one error-report client: what reaches the local report log
    /// may also leave for Sentry, if the user allowed it.
    error_reports: crate::error_reports::ErrorReports,
    /// The op of the turn being handled and who it answers.
    turn: turn::Turn,
    native: mpsc::Sender<Value>,
    hub: hub_link::HubLink,
    hub_protocol_version: Option<u64>,
    unrecognized_hub_events: BTreeSet<String>,
    hub_task: Option<JoinHandle<Result<()>>>,
    snapshot_revision: Option<u64>,
    hub_events: mpsc::Sender<(u64, HubEvent)>,
    /// Helper command ids still waiting for a reply, with the name of the
    /// command each one carried. Bounded by [`MAX_PENDING_NATIVE`]; entries
    /// are removed when the matching response arrives, so a late refusal can
    /// be classified without parsing names out of the id.
    pending_native: BTreeMap<String, String>,
    token: Option<String>,
    work: WorkRuntime,
    conversation: ConversationRuntime,
    capture: CaptureRuntime,
    features: FeatureRuntime,
    reads: InFlightReads,
    #[cfg(test)]
    fixture_store: Store,
}

struct InFlightReads {
    tasks: Vec<JoinHandle<()>>,
    privacy_epoch_tx: watch::Sender<u64>,
    privacy_epoch_rx: watch::Receiver<u64>,
    was_privacy_stopped: bool,
}

impl Default for InFlightReads {
    fn default() -> Self {
        let (privacy_epoch_tx, privacy_epoch_rx) = watch::channel(0);
        Self {
            tasks: Vec::new(),
            privacy_epoch_tx,
            privacy_epoch_rx,
            was_privacy_stopped: false,
        }
    }
}

impl InFlightReads {
    fn retire(&mut self, change: orbit_core::authority::AuthorityChange) {
        use orbit_core::authority::AuthorityChange;
        match change {
            AuthorityChange::UserStop | AuthorityChange::Revoked => {
                publish_privacy_epoch(true, &mut self.was_privacy_stopped, &self.privacy_epoch_tx);
            }
            AuthorityChange::Closing => {
                cancel_remote_reads(&mut self.tasks);
            }
            AuthorityChange::Device => {}
            AuthorityChange::Rearm | AuthorityChange::RearmWithoutStop => {
                publish_privacy_epoch(false, &mut self.was_privacy_stopped, &self.privacy_epoch_tx);
            }
            AuthorityChange::HubLink
            | AuthorityChange::HubTransportReady
            | AuthorityChange::HubLinkOffline
            | AuthorityChange::HubDisconnected
            | AuthorityChange::Conversation
            | AuthorityChange::Interrupt => {}
        }
    }
}

struct WorkRuntime {
    /// The one durable transfer worker (upload or finalize). A second job
    /// waits for its turn; this is not a queue limit.
    worker: ticketed::Ticketed<()>,
    retry_gate: crate::retry::RetryGate,
    upload_cursor: Option<String>,
    finalize_cursor: Option<String>,
    prefer_finalize: bool,
    work_events: mpsc::Sender<WorkResult>,
    durable_snapshot: DurableSnapshot,
    force_snapshot_once: bool,
    finalize_ready: BTreeMap<String, bool>,
}

struct ConversationRuntime {
    bridge_request: Option<BridgeAsk>,
    interaction_response: Option<interactions::PendingResponse>,
    interrupted_turns: interactions::InterruptedTurns,
    /// One voice turn worker; a second turn cannot start while one runs.
    voice_task: ticketed::Ticketed<()>,
    voice_operation: Option<String>,
    /// One playback at a time; a replay replaces the current one.
    speech_task: ticketed::Ticketed<()>,
    speech_available: Option<bool>,
    speak_next: bool,
    speech_notice_call_id: Option<String>,
    /// The answer the user asked to hear («Escuchar»), while it plays.
    read_aloud: Option<speech::ReadAloud>,
}

struct CaptureRuntime {
    buffer: CaptureBuffer,
    pending_seals: sealing::PendingSeals,
    reserve_exhausted: bool,
    seal_retry_at: Option<std::time::Instant>,
    audio: audio::AudioLifecycle,
    call_runtime: Option<call::CallRuntime>,
    mic_stream: Option<String>,
    remote_stream: Option<String>,
    pending_starts: Vec<String>,
    retired_call_starts: BTreeMap<String, String>,
    retired_stops: BTreeMap<String, audio::RetiredStreamStop>,
    interruption: Option<orbit_core::RecordingInterruption>,
}

struct FeatureRuntime {
    sources: sources::SourceRuntime,
    calendar: calendar::CalendarRuntime,
    computer: computer_control::ComputerRuntime,
    meeting: meeting_runtime::MeetingRuntime,
    /// The call detection, its preference and the agenda around a call.
    call: call_detection::CallRuntime,
    /// The question to Deimos during a meeting and the «Oye Deimos»
    /// detector's state.
    question: meeting_question::QuestionRuntime,
    native_host: native_host::NativeHost,
    notes: recording_notes::NotesRuntime,
}

use bridge::{BridgeAsk, BridgeReleaseReason};
use kernel::DurableSnapshot;

pub async fn run(paths: Paths, development: bool) -> Result<()> {
    run_with_helper(paths, development, None).await
}

/// The whole daemon, until its closure ends. Orbit.app runs it on a thread of
/// its own (`apps/desktop/src-tauri/src/daemon_thread.rs`); the
/// development binary `orbit` runs it on its main thread.
pub async fn run_with_helper(
    paths: Paths,
    development: bool,
    helper_override: Option<std::path::PathBuf>,
) -> Result<()> {
    // The startup future measured 113,328 bytes on the caller's stack. Keep
    // that future on the heap so ordinary 2 MiB test threads can run it.
    Box::pin(run_with_helper_inner(
        paths,
        development,
        helper_override,
        None,
    ))
    .await
}

/// The embedded daemon exposes its existing call queue and topic watches.
/// The thread owner sends its final result after dropping the runtime; a
/// dropped end sender reports a panic instead of inventing a successful quit.
pub async fn run_embedded(
    paths: Paths,
    development: bool,
    helper_override: Option<std::path::PathBuf>,
    connected: tokio::sync::oneshot::Sender<ipc::LocalClient>,
    ended: watch::Receiver<Option<ipc::DaemonEnded>>,
) -> Result<()> {
    Box::pin(run_with_helper_inner(
        paths,
        development,
        helper_override,
        Some((connected, ended)),
    ))
    .await
}

async fn run_with_helper_inner(
    paths: Paths,
    development: bool,
    helper_override: Option<std::path::PathBuf>,
    local: Option<(
        tokio::sync::oneshot::Sender<ipc::LocalClient>,
        watch::Receiver<Option<ipc::DaemonEnded>>,
    )>,
) -> Result<()> {
    if helper_override
        .as_ref()
        .is_some_and(|path| !path.is_absolute())
    {
        bail!("El helper debe tener una ruta absoluta");
    }
    // Before anything starts, so SIGTERM is never the default kill.
    // Process-wide: inside the app they are the app's too, and the app
    // installs none of its own, so a SIGTERM to Orbit closes the daemon in
    // order and the app follows it.
    let mut signals = shutdown::Signals::install()?;
    paths.initialize()?;
    let _instance = ipc::lock_instance(&paths)?;
    let socket_enabled = local.is_none() || orbit_core::orbit_bundle::LOCAL_SOCKET;
    if !socket_enabled {
        ipc::remove_owned_socket(&paths)?;
    }
    if socket_enabled && tokio::net::UnixStream::connect(&paths.socket).await.is_ok() {
        bail!("Ya hay un servicio Orbit activo");
    }
    let settings = paths.load_settings()?;
    // Before anything else runs, so a panic from here on is reported if the
    // user allowed it.
    let error_reports = crate::error_reports::ErrorReports::start(
        crate::error_reports::DSN,
        settings.error_reports,
    );
    error_reports.report_panics();
    let recovery_storage_key = storage_key(&paths, development)?;
    let store = store_actor::StoreActor::open_bootstrap(&paths.store, recovery_storage_key)?;
    let saved_state = store.load_snapshot::<DurableState>()?;
    let restart_interrupted_live_capture = saved_state
        .as_ref()
        .and_then(|saved| saved.recording.as_ref())
        .is_some_and(|recording| recording.phase.interrupted_on_recovery());
    let startup_op = orbit_ipc::new_op();
    let reported_sequence = saved_state
        .as_ref()
        .map(|state| state.notice_log.last_sequence())
        .unwrap_or(0);
    let mut state = saved_state
        .map(|mut state| {
            state.notice_log.begin_turn(startup_op.clone());
            state.recovered()
        })
        .map(State::from)
        .unwrap_or_default();
    state.durable.notice_log.begin_turn(startup_op);
    state.views.account = Some(account_summary(&settings));
    let (calls_tx, mut calls) = mpsc::channel::<Call>(32);
    let mut topic_board = topics::TopicBoard::new(uuid::Uuid::new_v4().to_string());
    let topic_channels = topic_board.channels();
    let mut listener = None;
    let mut listener_cleanup = None;
    if socket_enabled {
        let listener_paths = paths.clone();
        let listener_calls = calls_tx.clone();
        let listener_topics = topic_channels.clone();
        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
        let listener_started = std::time::Instant::now();
        let task = tokio::spawn(async move {
            ipc::listen(
                &listener_paths,
                listener_calls,
                listener_topics,
                Some(ready_tx),
            )
            .await
        });
        let mut cleanup = startup::ListenerCleanup::new(&task, paths.socket.clone());
        if ready_rx.await.is_err() {
            let elapsed_ms = listener_started
                .elapsed()
                .as_millis()
                .min(u128::from(u64::MAX)) as u64;
            return Err(match task.await {
                Ok(Err(source)) => startup::StartupError::IpcBind { elapsed_ms, source },
                Ok(Ok(())) => startup::StartupError::IpcEndedBeforeBind { elapsed_ms },
                Err(source) => startup::StartupError::IpcTaskEndedBeforeBind { elapsed_ms, source },
            }
            .into());
        }
        cleanup.socket_owned();
        listener_cleanup = Some(cleanup);
        listener = Some(task);
    }
    if let Some((connected, ended)) = local {
        let _ = connected.send(ipc::LocalClient::new(calls_tx, &topic_channels, ended));
    }
    let mut recovery_io = startup::RecoveryIo {
        log_root: paths.root.clone(),
        calls: &mut calls,
        signals: &mut signals,
        listener: &mut listener,
    };
    let snapshot_gate = std::sync::Arc::new(tokio::sync::Mutex::new(()));
    let (worker_done, _) = tokio::sync::watch::channel(false);
    let direct_stop = startup::StopStore::Direct {
        path: paths.store.clone(),
        key: recovery_storage_key,
        snapshot_gate: snapshot_gate.clone(),
        worker_done,
    };
    let (store, state, startup_started, persisted_state) = startup::recover(
        store,
        state,
        direct_stop,
        snapshot_gate.clone(),
        recovery_io.reborrow(),
    )
    .await?;
    let checkpoint = startup::RecoveryCheckpoint {
        state,
        started: startup_started,
    };
    let (work_events, mut work_rx) = mpsc::channel(WORK_EVENTS_CAPACITY);
    let store_actor = store_actor::StoreActor::spawn_with_diagnostics(
        store,
        work_events.clone(),
        crate::diagnostics::DiagnosticLog::in_root(&paths.root),
    )?;
    let stop_store = startup::StopStore::Actor(store_actor.startup_stop_writer()?);
    let (native_tx, native_commands) = mpsc::channel(64);
    // Sized for ten seconds of two-track audio (≈47 ev/s × 2 × 10 = 940,
    // rounded to 1000) so a stalled consumer pays in discarded frames, not
    // in the helper's life.
    let (native_events_tx, mut native_events) =
        mpsc::channel(crate::native::NATIVE_EVENTS_CAPACITY);
    let (hub_events, mut hub_rx) = mpsc::channel(128);
    let (notes_events, mut notes_rx) = mpsc::channel(recording_notes::NOTE_EVENTS);
    let preparation = tokio::task::spawn_blocking({
        let paths = paths.clone();
        move || startup::prepare_environment(paths, development, helper_override)
    });
    let (checkpoint, prepared) = startup::serve_step(
        checkpoint,
        stop_store.clone(),
        recovery_io.reborrow(),
        orbit_core::RecoveryStep::Environment,
        async {
            preparation
                .await
                .context("Terminó la preparación del servicio")?
        },
    )
    .await?;
    let checkpoint = startup::persist_runtime_recovery(
        checkpoint,
        stop_store.clone(),
        recovery_io.reborrow(),
        &store_actor,
        persisted_state,
        snapshot_gate,
    )
    .await?;
    let startup::PreparedEnvironment {
        helper_path,
        token,
        diagnostics,
    } = prepared;
    let native_available = helper_path.is_some();
    let mut state = checkpoint.state.clone();
    state.views.recovery = orbit_core::RecoveryStatus::Unknown;
    let durable_snapshot = DurableSnapshot::from(&state);
    #[cfg(test)]
    let fixture_store = test_store::open_existing(&paths.store, recovery_storage_key)?;
    let store_reader = store_actor.runtime_reader(&paths.store, recovery_storage_key)?;
    let voice_audio_reader = std::sync::Arc::new(std::sync::Mutex::new(
        store_actor.runtime_reader(&paths.store, recovery_storage_key)?,
    ));
    let report_log = orbit_ipc::report::ReportLog::daemon(&paths.root);
    let mut runtime = Runtime {
        closing: false,
        privacy_retirement_owed: false,
        paths,
        settings,
        development,
        state,
        reader: store_reader,
        voice_audio_reader,
        #[cfg(test)]
        fixture_store,
        store_actor,
        diagnostics: Some(diagnostics),
        report_log: Some(report_log),
        error_reports,
        turn: turn::Turn::fresh(orbit_core::notice::ReportSource::Daemon),
        native: native_tx,
        hub: hub_link::HubLink::default(),
        hub_protocol_version: None,
        unrecognized_hub_events: BTreeSet::new(),
        hub_task: None,
        snapshot_revision: None,
        hub_events,
        pending_native: BTreeMap::new(),
        token,
        work: WorkRuntime {
            worker: ticketed::Ticketed::logical(),
            retry_gate: crate::retry::RetryGate::default(),
            upload_cursor: None,
            finalize_cursor: None,
            prefer_finalize: true,
            work_events,
            durable_snapshot,
            force_snapshot_once: false,
            finalize_ready: BTreeMap::new(),
        },
        conversation: ConversationRuntime {
            bridge_request: None,
            interaction_response: None,
            interrupted_turns: Default::default(),
            voice_task: ticketed::Ticketed::logical(),
            voice_operation: None,
            speech_task: ticketed::Ticketed::logical(),
            speech_available: None,
            speak_next: false,
            read_aloud: None,
            speech_notice_call_id: None,
        },
        capture: CaptureRuntime {
            buffer: CaptureBuffer::default(),
            pending_seals: Default::default(),
            reserve_exhausted: false,
            seal_retry_at: None,
            audio: audio::AudioLifecycle::default(),
            call_runtime: None,
            mic_stream: None,
            remote_stream: None,
            pending_starts: Vec::new(),
            retired_call_starts: BTreeMap::new(),
            retired_stops: BTreeMap::new(),
            interruption: None,
        },
        features: FeatureRuntime {
            sources: sources::SourceRuntime::default(),
            calendar: calendar::CalendarRuntime::default(),
            computer: computer_control::ComputerRuntime::default(),
            meeting: meeting_runtime::MeetingRuntime::default(),
            call: call_detection::CallRuntime::default(),
            question: meeting_question::QuestionRuntime::default(),
            native_host: native_host::NativeHost::default(),
            notes: recording_notes::NotesRuntime::new(notes_events),
        },
        reads: InFlightReads::default(),
    };
    // Restored records were already reported; recovery notices are new entries.
    runtime.turn.reported_sequence = reported_sequence;
    runtime.begin_fresh_turn(orbit_core::notice::ReportSource::Daemon);
    let (runtime_progress_tx, runtime_progress) =
        tokio::sync::watch::channel(orbit_core::RecoveryStep::Operations);
    let _progress_sender = runtime_progress_tx.clone();
    let runtime_recovery = async {
        runtime
            .close_empty_interrupted(orbit_core::RecordingInterruption {
                cause: if restart_interrupted_live_capture {
                    orbit_core::RecordingInterruptionCause::Restart
                } else {
                    orbit_core::RecordingInterruptionCause::CauseNotSaved
                },
                detail: None,
                audio_available: false,
            })
            .await?;
        runtime.recover_call_closures(now()).await?;
        runtime.recover_recording_notes()?;
        runtime_progress_tx.send_replace(orbit_core::RecoveryStep::CallPreference);
        runtime.recover_call_preference();
        Ok(())
    };
    let (checkpoint, ()) = startup::serve_runtime_recovery(
        checkpoint,
        stop_store.clone(),
        recovery_io.reborrow(),
        runtime_progress,
        runtime_recovery,
    )
    .await?;
    if startup::needs_runtime_stop(&checkpoint.state, &runtime.state) {
        let (_after_stop, ()) = startup::serve_step(
            checkpoint,
            stop_store.clone(),
            recovery_io.reborrow(),
            orbit_core::RecoveryStep::Snapshot,
            async {
                runtime
                    .privacy_stop(orbit_core::privacy_stop::UserStop::Order)
                    .await;
                Ok(())
            },
        )
        .await?;
    }
    runtime
        .store_actor
        .critical_snapshot(&runtime.state)
        .await?;
    runtime.store_actor.finish_startup().await?;
    runtime.state.views.recovery = orbit_core::RecoveryStatus::Ready;
    // Only now, with nothing left that can fail the startup: a helper launched
    // earlier could be left behind by a startup error.
    let helper = native_available
        .then(|| native::Supervisor::spawn(helper_path, native_commands, native_events_tx));
    runtime.connect();
    let topics_started = std::time::Instant::now();
    let mut loop_turns = [0_u64; 8];
    if let Some(cleanup) = &mut listener_cleanup {
        cleanup.disarm();
    }
    let mut tick = tokio::time::interval(Duration::from_secs(1));
    let mut last_tick = Duration::ZERO;
    let mut closing = shutdown::Shutdown::default();
    let outcome: Result<()> = async {
        loop {
            closing.observe_listener(listener.as_ref().is_some_and(JoinHandle::is_finished), &mut runtime).await;
            if closing.ready(&runtime) { break; }
            // The bounded budget is checked on every iteration, not only on
            // the tick, so a closure that has been waiting on a stuck vertical
            // still leaves on time.
            if closing.exit_now(std::time::Instant::now()) {
                let _ = runtime.declare_bounded_exit().await;
                break;
            }
            let topic_due = topic_board.next_due_ms().and_then(|due_ms| {
                topics_started
                    .checked_add(Duration::from_millis(due_ms))
                    .map(tokio::time::Instant::from_std)
            });
            tokio::select! {
                name = signals.next() => {
                    loop_turns[0] += 1;
                    runtime.begin_fresh_turn(orbit_core::notice::ReportSource::Daemon);
                    // SIGTERM (launchd at logout, `kill`) and SIGINT (a
                    // terminal) are one request: the complete closure the IPC
                    // order runs. A second signal only means "now".
                    let first = !closing.requested;
                    closing.signal();
                    runtime.log("shutdown_signal", &[("signal", name)]);
                    if closing.signals > 1 {
                        let _ = runtime
                            .declare_bounded_exit_for(shutdown::BoundedExitCause::SecondSignal(name))
                            .await;
                        break;
                    }
                    if first {
                        match closing.close_until_exit(&mut runtime, signals.next()).await {
                            shutdown::CloseWait::Completed => {}
                            shutdown::CloseWait::BudgetExpired => {
                                let _ = runtime.declare_bounded_exit().await;
                                break;
                            }
                            shutdown::CloseWait::SecondSignal(signal) => {
                                closing.signal();
                                runtime.log("shutdown_signal", &[("signal", signal)]);
                                let _ = runtime
                                    .declare_bounded_exit_for(shutdown::BoundedExitCause::SecondSignal(signal))
                                    .await;
                                break;
                            }
                        }
                    }
                },
                Some(call) = calls.recv() => {
                    loop_turns[1] += 1;
                    // The order's op, or a new one for a client that sent
                    // none: everything this turn says carries it.
                    runtime.begin_turn(
                        call.request.op.clone().unwrap_or_else(orbit_ipc::new_op),
                        orbit_core::notice::ReportSource::Daemon,
                    );
                    let command = call.request.body;
                    let topic_command = matches!(&command, Command::SubscribeTopics { .. } | Command::Read { .. });
                    if !topic_command {
                        topic_board.state_changed();
                    }
                    let result = if topic_command {
                        Ok(())
                    } else {
                        closing.command(command.clone(), &mut runtime).await
                    };
                    // The transport has already attached the subscription's
                    // receivers. Refresh their dirty heads before waking the
                    // caller, which otherwise can accept an older turn as current.
                    if !topic_command || matches!(&command, Command::SubscribeTopics { .. }) {
                        runtime.report_notices();
                        let source = topics::TopicSource {
                            state: &runtime.state,
                            runtime: Some(&runtime),
                            now_ms: u64::try_from(topics_started.elapsed().as_millis()).unwrap_or(u64::MAX),
                        };
                        topic_board.publish_command(&source);
                    }
                    if let Command::Read { read } = &command {
                        runtime.serve_read(read.clone(), call.request.id, call.reply, topic_board.revisions());
                    } else if topic_command {
                        record_ipc_reply_loss(call.reply.send(topic_response(call.request.id, topic_board.revisions(), None, None)), &command, runtime.diagnostics.as_ref());
                    } else if let Command::VoiceNoteAudio { message_id } = &command {
                        let audio = result.and_then(|()| runtime.voice_note_audio(message_id));
                        let response = VoiceNoteAudioResponse {
                            v: 1,
                            id: call.request.id,
                            ok: audio.is_ok(),
                            error: audio.as_ref().err().map(ToString::to_string),
                            refusal: audio.as_ref().err().map(refusal_from_error),
                            audio: audio.ok(),
                        };
                        record_ipc_reply_loss(
                            call.reply.send(response),
                            &command,
                            runtime.diagnostics.as_ref(),
                        );
                    } else {
                        let refusal = result.err().map(|error| refusal_from_error(&error));
                        let status = matches!(command, Command::Status | Command::PrivacyStop).then(|| DaemonStatus::from(&runtime.state));
                        let response = command_response(call.request.id, status, refusal, Some(topic_board.epoch().to_owned()), topic_board.revisions());
                        record_ipc_reply_loss(
                            call.reply.send(response),
                            &command,
                            runtime.diagnostics.as_ref(),
                        );
                    }
                }
                Some(event) = native_events.recv() => {
                    loop_turns[2] += 1;
                    runtime.begin_fresh_turn(orbit_core::notice::ReportSource::Daemon);
                    if closing.allows_native_event(&event) {
                        topic_board.state_changed();
                        runtime.handle_native_event(event).await;
                    }
                }
                Some((generation, event)) = hub_rx.recv() => {
                    loop_turns[3] += 1;
                    let op = runtime.hub_event_op(&event);
                    runtime.begin_turn(op, orbit_core::notice::ReportSource::Hub);
                    if generation == runtime.state.generation() {
                        topic_board.state_changed();
                        if let Err(error) = runtime.hub_event(event).await {
                            runtime.say_failure(orbit_core::notice::NoticeCode::HubEventFailed, &error);
                        }
                    }
                }
                Some(event) = work_rx.recv() => {
                    loop_turns[4] += 1;
                    let op = event.op();
                    runtime.begin_turn(op, orbit_core::notice::ReportSource::Daemon);
                    topic_board.state_changed();
                    if let Err(error) = runtime.work_event(event).await {
                        runtime.say_failure(orbit_core::notice::NoticeCode::WorkFailed, &error);
                    }
                },
                Some(event) = notes_rx.recv() => {
                    loop_turns[5] += 1;
                    runtime.begin_fresh_turn(orbit_core::notice::ReportSource::Daemon);
                    topic_board.state_changed();
                    if let Err(error) = runtime.note_answer(event).await {
                        runtime.say_failure(orbit_core::notice::NoticeCode::WorkFailed, &error);
                    }
                },
                _ = async {
                    if let Some(due) = topic_due {
                        tokio::time::sleep_until(due).await;
                    }
                }, if topic_due.is_some() => { loop_turns[6] += 1; },
                _ = tick.tick() => {
                    loop_turns[7] += 1;
                    let tick_started = std::time::Instant::now();
                    runtime.begin_fresh_turn(orbit_core::notice::ReportSource::Daemon);
                    // The call's health is judged with what the loop has not
                    // read yet: after a held loop, frames waiting here are
                    // audio that arrived, not silence.
                    runtime.note_loop_tick(native_events.len(), last_tick);
                    topic_board.state_changed();
                    runtime.handle_audio_timeout().await;
                    runtime.run_maintenance().await;
                    runtime.error_reports.send_due(std::time::Instant::now());
                    last_tick = tick_started.elapsed();
                }
            }
            runtime.reads.tasks.retain(|task| !task.is_finished());
            runtime.report_notices();
            let source = topics::TopicSource {
                state: &runtime.state,
                runtime: Some(&runtime),
                now_ms: u64::try_from(topics_started.elapsed().as_millis()).unwrap_or(u64::MAX),
            };
            topic_board.publish(&source);
            if closing.exit_now(std::time::Instant::now()) {
                let _ = runtime.declare_bounded_exit().await;
                break;
            }
            if closing.ready(&runtime) {
                break;
            }
        }
        if closing.listener_failed {
            bail!("El servidor IPC se detuvo; el audio pendiente ya está guardado. Revisa la ruta de datos y otra instancia");
        }
        Ok(())
    }.await;
    // One aggregate at closure: no event payload, command, op or conversation.
    if let Some(diagnostics) = &runtime.diagnostics {
        diagnostics.record(
            "loop_turns",
            &[
                ("signals", &loop_turns[0].to_string()),
                ("commands", &loop_turns[1].to_string()),
                ("native", &loop_turns[2].to_string()),
                ("hub", &loop_turns[3].to_string()),
                ("work", &loop_turns[4].to_string()),
                ("notes", &loop_turns[5].to_string()),
                ("topic_due", &loop_turns[6].to_string()),
                ("tick", &loop_turns[7].to_string()),
                (
                    "elapsed_ms",
                    &topics_started.elapsed().as_millis().to_string(),
                ),
            ],
        );
    }
    cancel_remote_reads(&mut runtime.reads.tasks);
    // Every exit has already stopped producers and crossed the same PCM
    // durability boundary. Do not start a second unobserved drain during cleanup.
    if !runtime.store_actor.bounded_exit_declared() {
        let _ = runtime.store_actor.snapshot(&runtime.state).await;
    }
    if let Some(task) = runtime.hub_task.take() {
        task.abort();
    }
    runtime.work.worker.cancel();
    if let Some(helper) = helper {
        // `shutdown`, its input closed, its grace, and only then a kill.
        runtime.log_helper_end(helper.stop(&mut native_events).await);
    }
    if let Some(listener) = listener {
        listener.abort();
        let _ = std::fs::remove_file(&runtime.paths.socket);
    }
    runtime
        .error_reports
        .flush(crate::error_reports::EXIT_FLUSH);
    outcome
}

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
