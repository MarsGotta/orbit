//! The bridge's one slot, freed when the operation that served it ended
//! outside the helper's own answer.
use super::{BridgeAsk, BridgeReleaseReason};
use crate::daemon::{Connection, Runtime, hub, hub_link};
use orbit_core::notice::{Notice, NoticeCode, NoticeKey};
use orbit_core::refusal::Refusal;
use orbit_core::wire::bridge::DeviceReplyParams;
use serde_json::json;

impl Runtime {
    /// Frees the bridge slot when the operation that served it ended outside
    /// the helper's own answer.
    ///
    /// Every ending listed by the spec calls this: timeout, cancellation,
    /// privacy stop, hub disconnect, generation change. If the hub is still
    /// reachable the caller is told why with the same wire shape the
    /// rejection path already uses; if the hub is gone, the local notice names
    /// the failed delivery. Either way the slot is left as `None` so the next remote
    /// request is admitted rather than told the bridge is busy.
    pub(in crate::daemon) fn release_bridge_request(&mut self, reason: BridgeReleaseReason<'_>) {
        if let Some(mut upload) = self.sources.bridge_upload.take() {
            upload.work.cancel();
        }
        let Some(ask) = self.assistant.bridge_request.take() else {
            return;
        };
        let attempted_action = (ask.capability
            == orbit_core::device_bridge::Capability::PerformAction)
            .then(|| {
                self.computer
                    .active_general
                    .as_ref()
                    .map(|entry| entry.operation_id.clone())
            })
            .flatten();
        if self.release_without_hub(&ask, reason) {
            return;
        }
        let interruption = reason.refusal();
        let attempted = attempted_action.is_some();
        let refusal = if let Some(operation_id) = attempted_action {
            Refusal::OutcomeUnknown { operation_id }
        } else {
            self.cancelled_source_failure(&ask, reason)
                .unwrap_or_else(|| interruption.clone())
        };
        let mut reply = DeviceReplyParams::refused(ask.id.clone(), refusal, None);
        if attempted && let Some(ref mut fields) = reply.refusal {
            fields.interruption = Some(Box::new(interruption));
        }
        let frame = hub::command(&ask.id, "device.reply", json!(reply));
        // `send_or_report` rather than `self.send`: this path runs after
        // `privacy_stopped` is already true, and a refusal is the answer the
        // caller is owed rather than a request that should be rebuffed.
        let result = self.send_frame(frame);
        let subject = if matches!(reason, BridgeReleaseReason::PrivacyStopped) {
            "the remote request's refusal (privacy_stopped)"
        } else {
            "the remote request's refusal"
        };
        self.note_bridge_send(&result, subject);
        if matches!(reason, BridgeReleaseReason::CaptureUploadTimeout { .. }) {
            self.say(
                Notice::new(NoticeCode::BridgeRefused)
                    .with_token(NoticeKey::Code, "capture_upload_timeout"),
            );
        }
    }

    /// The endings with no hub to tell: said locally, and `true` when the
    /// release is complete.
    fn release_without_hub(&mut self, ask: &BridgeAsk, reason: BridgeReleaseReason<'_>) -> bool {
        // «Adjuntar ventana» is answered to the app, not to the hub (its reply
        // is diverted in `send_frame`): without the hub it is told why here.
        if ask.for_attachment && self.state.connection() == Connection::Offline {
            self.assistant
                .refuse_window(if self.state.privacy_stopped() {
                    Refusal::PrivacyStopped { since_ms: None }
                } else {
                    Refusal::HubUnreachable { since_ms: None }
                });
            return true;
        }
        if self.state.privacy_stopped()
            && self.state.connection() == Connection::Offline
            && !self.link.outbound.is_some()
        {
            self.note_bridge_send(
                &hub_link::SendResult::Retired(hub_link::Retirement::PrivacyStop),
                "the remote request's refusal",
            );
            return true;
        }
        // The disconnect handler sets Offline before releasing its request.
        // The user's privacy stop also sets Offline, but after cutting capture
        // it may still deliver this final refusal on the open link before
        // retiring it.
        if self.state.connection() == Connection::Offline
            && !(matches!(reason, BridgeReleaseReason::PrivacyStopped)
                && self.link.outbound.is_some())
        {
            self.note_bridge_send(
                &hub_link::SendResult::Disconnected,
                "the remote request's refusal",
            );
            return true;
        }
        false
    }

    /// A cancelled source read that failed on its own says its own cause.
    fn cancelled_source_failure(
        &self,
        ask: &BridgeAsk,
        reason: BridgeReleaseReason<'_>,
    ) -> Option<Refusal> {
        self.sources
            .last_failure
            .as_ref()
            .filter(|failure| {
                matches!(reason, BridgeReleaseReason::Cancelled)
                    && (ask.source_id.as_deref() == Some(failure.operation_id.as_str())
                        || (ask.source_id.is_none()
                            && self
                                .state
                                .views
                                .source_request
                                .as_ref()
                                .is_some_and(|progress| progress.id == failure.operation_id)))
            })
            .map(|failure| failure.refusal.clone())
    }
}
