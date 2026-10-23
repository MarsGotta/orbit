//! What the loop answers the app with, and the reads a privacy stop cuts.
use super::*;

pub(super) fn cancel_remote_reads(reads: &mut Vec<JoinHandle<()>>) {
    for task in reads.drain(..) {
        task.abort();
    }
}

pub(super) fn privacy_stopped_read()
-> orbit_core::topics::Knowledge<orbit_core::topics::Page<serde_json::Value>> {
    orbit_core::topics::Knowledge::Absent {
        refusal: Refusal::PrivacyStopped { since_ms: None },
    }
}

pub(super) async fn cancellable_remote_read(
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

pub(super) fn publish_privacy_epoch(
    stopped: bool,
    was_stopped: &mut bool,
    epoch: &watch::Sender<u64>,
) {
    if stopped && !*was_stopped {
        epoch.send_modify(|value| *value = value.wrapping_add(1));
    }
    *was_stopped = stopped;
}

pub(super) fn refusal_from_error(error: &anyhow::Error) -> Refusal {
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

pub(super) fn ipc_command_name(command: &Command) -> String {
    serde_json::to_value(command)
        .ok()
        .and_then(|value| value["command"].as_str().map(str::to_owned))
        .unwrap_or_else(|| "command_serialization_failed".into())
}

pub(super) fn command_response(
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

pub(super) fn topic_response(
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

pub(super) fn record_ipc_reply_loss<T>(
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
