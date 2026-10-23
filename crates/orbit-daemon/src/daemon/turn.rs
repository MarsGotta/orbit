//! The op of the actor's current turn and the daemon's one report function.
//!
//! Every turn of the actor answers one thing: an order from the app, a frame
//! from the hub, a helper event, a worker's result, the once-a-second upkeep.
//! The turn carries that thing's [`Op`]; whatever the turn says, sends to the
//! hub or writes to `daemon.log` carries it too. Domain notices inherit that
//! op on insertion; local notices use [`Runtime::scoped`]. Reporting advances
//! its own cursor without changing the published records. [`Runtime::report_line`]
//! writes the local line and hands it to the error-report client.
use super::*;
use orbit_core::notice::{Notice, NoticeCode, Op, Report, ReportSource};

pub(super) struct Turn {
    pub(super) op: Op,
    pub(super) source: ReportSource,
    pub(super) reported_sequence: u64,
}

impl Turn {
    pub(super) fn fresh(source: ReportSource) -> Self {
        Self {
            op: orbit_ipc::new_op(),
            source,
            reported_sequence: 0,
        }
    }
}

impl Runtime {
    /// Starts a turn: what was said before it is reported under the turn
    /// that said it, and everything from here belongs to `op`.
    pub(super) fn begin_turn(&mut self, op: Op, source: ReportSource) {
        self.report_notices();
        self.state.durable.notice_log.begin_turn(op.clone());
        self.turn = Turn {
            op,
            source,
            reported_sequence: self.turn.reported_sequence,
        };
    }

    /// Starts a turn for something that began here: a helper event, the
    /// upkeep, a hub event that names no request.
    pub(super) fn begin_fresh_turn(&mut self, source: ReportSource) {
        self.begin_turn(orbit_ipc::new_op(), source);
    }

    pub(super) fn op(&self) -> &Op {
        &self.turn.op
    }

    /// Names the producing turn before the notice enters a state or report.
    pub(super) fn scoped(&self, notice: impl Into<Notice>) -> Notice {
        let mut notice = notice.into();
        notice.op.get_or_insert_with(|| self.turn.op.clone());
        notice
    }

    /// The daemon's one report function: the line in the local report log,
    /// and the same line to Sentry if the user allowed it.
    pub(super) fn report_line(&self, report: &Report) {
        if let Some(log) = &self.report_log {
            log.report(report);
        }
        self.error_reports.report(report, std::time::Instant::now());
    }

    /// Records a notice nobody needs to see (what the helper reports): its
    /// report line, and Sentry if the user allowed it, without
    /// touching the notices the interface shows.
    pub(super) fn record(&self, notice: impl Into<Notice>) {
        let notice = self.scoped(notice);
        let op = notice.op.clone().unwrap_or_else(|| self.turn.op.clone());
        self.report_line(&Report::of_notice(&notice, op, self.turn.source));
    }

    /// Says a notice of this turn.
    pub(super) fn say(&mut self, notice: impl Into<Notice>) {
        self.state.set_notice(self.scoped(notice));
        self.report_notices();
    }

    /// Says a failure the person cannot fix, keeping its cause, which may be
    /// a sentence, only in `daemon.log` under the same op.
    pub(super) fn say_failure(&mut self, notice: impl Into<Notice>, cause: &dyn std::fmt::Display) {
        let mut notice = notice.into();
        // A seal that failed says why: the cause its seal recorded.
        if notice.code == orbit_core::notice::NoticeCode::RecordingSealFailed
            && !notice
                .data
                .contains_key(&orbit_core::notice::NoticeKey::Cause)
            && let Some(seal) = self.capture.seal_cause.take()
        {
            notice = notice.with_token(orbit_core::notice::NoticeKey::Cause, seal);
        }
        let code = serde_json::to_value(notice.code).unwrap_or_default();
        self.log(
            "notice_cause",
            &[
                ("code", code.as_str().unwrap_or("unknown")),
                ("cause", &cause.to_string()),
            ],
        );
        self.say(notice);
    }

    /// The op a hub event belongs to: the one the frame names, or the op of
    /// the request it answers (by id), so a notice built from the hub's
    /// error keeps the op of the request. Anything else is new.
    pub(super) fn hub_event_op(&self, event: &crate::hub::HubEvent) -> Op {
        use crate::hub::HubEvent;
        let named = match event {
            HubEvent::Frame(frame) => frame
                .get("op")
                .and_then(Value::as_str)
                .and_then(Op::parse)
                .or_else(|| {
                    frame
                        .get("id")
                        .and_then(Value::as_str)
                        .and_then(|id| self.hub.op_for(id))
                }),
            HubEvent::Decoded(frame) => frame.id.as_deref().and_then(|id| self.hub.op_for(id)),
            HubEvent::Connected
            | HubEvent::Disconnected
            | HubEvent::InvalidFrame { .. }
            | HubEvent::Problem(_)
            | HubEvent::Revoked => None,
        };
        named.unwrap_or_else(orbit_ipc::new_op)
    }

    /// Sends one frame to the hub under this turn's op.
    pub(super) fn send_frame(&mut self, frame: anyhow::Result<Value>) -> hub_link::SendResult {
        // «Adjuntar ventana»'s capture answers the app, never the hub.
        if let Ok(frame) = &frame
            && self.divert_window_reply(frame)
        {
            return hub_link::SendResult::Queued;
        }
        self.hub.send_or_report(frame, &self.turn.op)
    }

    /// Reports new records without changing anything already published.
    pub(super) fn report_notices(&mut self) {
        for notice in self.state.durable.notice_log.records() {
            if notice
                .sequence
                .is_some_and(|sequence| sequence > self.turn.reported_sequence)
                && let Some(op) = notice.op.clone()
            {
                self.report_line(&Report::of_notice(notice, op, self.turn.source));
            }
        }
        self.turn.reported_sequence = self.state.durable.notice_log.last_sequence();
    }
}

/// The op a durable job was queued under. Read like a field with
/// `#[serde(default)]`: a job saved before has none.
pub(super) fn job_op(payload: &Value) -> Option<Op> {
    payload
        .get("op")
        .and_then(Value::as_str)
        .and_then(Op::parse)
}

/// What an order refused by the core says when no caller is waiting for the
/// refusal (a shortcut, a call the Mac detected): the refusals the user can fix
/// keep their own notice; any other is `otherwise`.
pub(super) fn refusal_notice(error: &anyhow::Error, otherwise: NoticeCode) -> Notice {
    use orbit_core::{refusal::Permission, refusal::Refusal};
    match error.downcast_ref::<Refusal>() {
        Some(Refusal::PrivacyStopped { .. }) => Notice::new(NoticeCode::PrivacyStopActive),
        Some(Refusal::HelperNotRunning) => Notice::new(NoticeCode::HelperNotRunning),
        Some(
            Refusal::PermissionDenied {
                permission: Permission::Microphone,
            }
            | Refusal::PermissionNotGranted {
                permission: Permission::Microphone,
            }
            | Refusal::PermissionUnknown {
                permission: Permission::Microphone,
            },
        ) => Notice::new(NoticeCode::MicrophonePermissionMissing),
        Some(
            Refusal::PermissionDenied {
                permission: Permission::ScreenRecording,
            }
            | Refusal::PermissionNotGranted {
                permission: Permission::ScreenRecording,
            }
            | Refusal::PermissionUnknown {
                permission: Permission::ScreenRecording,
            },
        ) => Notice::new(NoticeCode::ScreenPermissionMissing),
        _ => Notice::new(otherwise),
    }
}

/// What tests read of the notices a state holds: codes, and the
/// tokens their data carries, instead of sentences.
#[cfg(test)]
pub(crate) mod said {
    use orbit_core::State;
    use orbit_core::notice::{DataValue, NoticeCode};

    /// Every code in the recent sequence, oldest first, then the current one.
    pub(crate) fn codes(state: &State) -> Vec<NoticeCode> {
        let mut codes: Vec<NoticeCode> = state
            .durable
            .notice_log
            .records()
            .iter()
            .map(|notice| notice.code)
            .collect();
        if let Some(current) = state.notice_code()
            && codes.last() != Some(&current)
        {
            codes.push(current);
        }
        codes
    }

    /// Whether any notice in the sequence or the current one has `code`.
    pub(crate) fn has(state: &State, code: NoticeCode) -> bool {
        codes(state).contains(&code)
    }

    /// Makes the runtime keep `daemon.log` in `directory`, so a test can read
    /// the causes the daemon logs under each op.
    pub(crate) fn trace(runtime: &mut crate::daemon::Runtime, directory: &std::path::Path) {
        runtime.diagnostics = Some(crate::diagnostics::DiagnosticLog::in_root(directory));
    }

    /// Everything told about what happened: each notice as JSON (its code
    /// and data) and each `daemon.log` line (the causes, with their op).
    pub(crate) fn told(runtime: &crate::daemon::Runtime) -> String {
        let mut told: Vec<String> = runtime
            .state
            .durable
            .notice_log
            .records()
            .iter()
            .chain(runtime.state.durable.notice.iter())
            .map(|notice| serde_json::to_string(notice).unwrap_or_default())
            .collect();
        if let Some(log) = &runtime.diagnostics
            && let Ok(text) = std::fs::read_to_string(log.path())
        {
            told.push(text);
        }
        told.join("\n")
    }

    /// Whether any notice carries `token` (a helper or hub code) in its data.
    pub(crate) fn mentions(state: &State, token: &str) -> bool {
        state
            .durable
            .notice_log
            .records()
            .iter()
            .chain(state.durable.notice.iter())
            .flat_map(|notice| notice.data.values())
            .any(|value| matches!(value, DataValue::Token(said) if said.as_str() == token))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::hub::HubEvent;
    use orbit_core::notice::{NoticeCode, Op, ReportSource};
    use serde_json::json;

    /// The contract with the hub: a notice the daemon builds from
    /// the hub's error keeps the op of the request it answers, found by the
    /// frame's id or named by the frame itself.
    #[tokio::test]
    async fn a_notice_from_the_hubs_error_keeps_the_op_of_the_request() {
        let directory = tempfile::tempdir().unwrap();
        let (mut runtime, _helper) =
            crate::daemon::audio::tests::fixture(directory.path()).unwrap();
        let mut frames = runtime.hub.open(4);
        let op = Op::parse("0123abcd-request").unwrap();
        runtime.begin_turn(op.clone(), ReportSource::Daemon);
        let sent = runtime.send_frame(crate::hub::command("request-7", "ping", json!({})));
        assert!(sent.delivered_to_queue());
        assert_eq!(frames.try_recv().unwrap()["op"], "0123abcd-request");

        let error = HubEvent::Frame(json!({"v":1,"event":"error","id":"request-7",
            "data":{"message":"el hub no pudo","code":"bad_request"}}));
        let answered = runtime.hub_event_op(&error);
        assert_eq!(answered, op);
        runtime.begin_turn(answered, ReportSource::Hub);
        runtime.hub_event(error).await.unwrap();
        let notice = runtime.state.durable.notice.clone().unwrap();
        assert_eq!(notice.code, NoticeCode::HubError);
        assert_eq!(notice.op, Some(op));

        let named = HubEvent::Frame(json!({"v":1,"event":"error","op":"fedcba98-named",
            "data":{"message":"x"}}));
        assert_eq!(runtime.hub_event_op(&named).as_str(), "fedcba98-named");
        let unrelated = HubEvent::Frame(json!({"v":1,"event":"error","id":"never-sent"}));
        assert_ne!(
            runtime.hub_event_op(&unrelated).as_str(),
            "0123abcd-request"
        );
    }

    /// A job saved before typed ops still loads (the op reads as
    /// absent), and an op that is not one reads as absent too.
    #[test]
    fn a_durable_job_without_a_valid_op_reads_as_absent() {
        assert_eq!(job_op(&json!({"recording_id": "recording"})), None);
        assert_eq!(job_op(&json!({"op": "con espacio"})), None);
    }
}

#[cfg(test)]
#[path = "permission_notice_tests.rs"]
mod permission_tests;
