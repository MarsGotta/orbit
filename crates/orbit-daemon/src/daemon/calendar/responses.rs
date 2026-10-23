use crate::daemon::now;
use anyhow::Result;
use orbit_core::calendar::{
    self, CalendarInventoryPage, CalendarReadFacts, CalendarReadPage, CalendarStatus,
    InventoryRead, WindowRead, assemble_view,
};
use orbit_core::wire::native::{
    NativeEventHeader, NativeResponseFields, NativeResponseIdRef, decode_result_body,
};
use serde::Deserialize;

#[derive(Deserialize)]
struct CalendarObservation {
    #[serde(default, deserialize_with = "present_calendar")]
    calendar: CalendarField,
}

#[derive(Deserialize)]
struct CalendarPermissionReply {
    calendars: CalendarAuthorization,
}

#[derive(Default)]
enum CalendarField {
    #[default]
    Missing,
    Present(Value),
}

fn present_calendar<'de, D: serde::Deserializer<'de>>(
    decoder: D,
) -> std::result::Result<CalendarField, D::Error> {
    Value::deserialize(decoder).map(CalendarField::Present)
}

use super::*;

#[path = "result_pages.rs"]
mod pages;

#[path = "observed.rs"]
mod observed;

impl Runtime {
    pub(in crate::daemon) fn calendar_response(&mut self, event: &Value) -> bool {
        let Some(ticket) = NativeResponseIdRef::from_value(event)
            .ok()
            .map(|response| response.id)
            .filter(|id| id.starts_with(PREFIX))
        else {
            return false;
        };
        // Ignore obsolete/unknown responses, including errors: they must not
        // trigger generic fail_closed and stop an unrelated audio recording.
        if self
            .calendar
            .pending
            .as_ref()
            .is_none_or(|pending| pending.id != ticket)
        {
            return true;
        }
        let Some(pending) = self.calendar.pending.take() else {
            return true;
        };
        let failure = self.calendar_pending_failure(&pending, 0);
        let settlement = self
            .calendar
            .reply
            .settle(&pending.ticket, self.state.generation());
        let settlement_failure = settlement_failure(settlement);
        if let Some(failure) = failure.or(settlement_failure) {
            self.calendar_job_failed_detail(pending.job, failure.code(), Some(&failure.refusal()));
            self.retire_native_calendar();
        } else if let Job::Write {
            request_id,
            request,
        } = &pending.job
        {
            self.calendar_write_answered(&pending, request_id, request, event);
        } else {
            self.calendar_read_answered(pending, event);
        }
        self.pump_calendar();
        true
    }

    /// A write's answer: refused with the helper's code, admitted, or an
    /// unreadable reply whose outcome stays unknown.
    fn calendar_write_answered(
        &mut self,
        pending: &Pending,
        request_id: &str,
        request: &WriteRequest,
        event: &Value,
    ) {
        match NativeResponseFields::from_value(event) {
            Ok(response) if !response.ok => {
                let (code, detail) = response
                    .error
                    .as_ref()
                    .map_or(("calendar_write_outcome_unknown", None), |error| {
                        (error.code.as_str(), error.detail.as_ref())
                    });
                self.calendar_write_refused(request_id, request, code, detail);
            }
            Ok(response) => {
                let admitted = response.result.as_ref().map_or_else(
                    || Err("response calendar field=result type=absent".to_owned()),
                    |result| {
                        self.admit_calendar_write(pending, request_id, request, result)
                            .map_err(|problem| problem.to_string())
                    },
                );
                if let Err(problem) = admitted {
                    self.log(
                        "calendar_reply_invalid",
                        &[("command", request.command().0), ("cause", &problem)],
                    );
                    self.calendar_write_failed(request_id, request, "calendar_reply_invalid", true);
                }
            }
            Err(_) => {
                self.calendar_write_failed(request_id, request, "calendar_reply_invalid", true);
            }
        }
    }

    /// A read's answer: the helper's refusal, a result admitted as the
    /// contract says, or an unreadable reply.
    fn calendar_read_answered(&mut self, pending: Pending, event: &Value) {
        if let Ok(response) = NativeResponseFields::from_value(event) {
            if !response.ok {
                let code = response
                    .error
                    .as_ref()
                    .map_or("calendar_read_failed", |error| error.code.as_str())
                    .to_owned();
                self.calendar_job_failed(pending.job, &code);
            } else if let Some(result) = response.result.as_ref() {
                if let Err(problem) = self.admit_calendar_result(&pending, result) {
                    let refusal = problem
                        .downcast_ref::<orbit_core::wire::native::NativeDecodeProblem>()
                        .map(|problem| {
                            orbit_core::refusal::Refusal::helper_reply_invalid(problem.clone())
                        })
                        .unwrap_or_else(|| orbit_core::refusal::Refusal::HelperReplyInvalid {
                            field: "calendar.response".into(),
                            expected: "calendar_contract".into(),
                            received_type: problem.to_string(),
                        });
                    self.calendar_invalid_reply(pending.job, refusal);
                }
            } else {
                self.calendar_invalid_reply(
                    pending.job,
                    orbit_core::refusal::Refusal::HelperReplyInvalid {
                        field: "result".into(),
                        expected: "object".into(),
                        received_type: "absent".into(),
                    },
                );
            }
        } else {
            self.calendar_invalid_reply(
                pending.job,
                orbit_core::refusal::Refusal::HelperReplyInvalid {
                    field: "envelope".into(),
                    expected: "native_v1/calendar".into(),
                    received_type: "invalid".into(),
                },
            );
        }
    }

    fn calendar_invalid_reply(&mut self, job: Job, refusal: orbit_core::refusal::Refusal) {
        if let Job::Write {
            request_id,
            request,
        } = &job
        {
            self.calendar_write_failed(request_id, request, "calendar_reply_invalid", true);
            return;
        }
        self.calendar_job_failed_detail(job, "calendar_reply_invalid", Some(&refusal));
    }

    fn calendar_facts(&self, pending: &Pending) -> (String, u64, CalendarAuthorization, bool) {
        (
            pending.host.id.clone(),
            pending.host.generation,
            self.calendar.view.authorization,
            self.state.privacy_stopped() || pending.host.stopped,
        )
    }

    /// A request to the helper ended without an answer to use.
    pub(super) fn calendar_job_failed(&mut self, job: Job, code: &str) {
        self.calendar_job_failed_detail(job, code, None);
    }

    pub(super) fn calendar_job_failed_detail(
        &mut self,
        job: Job,
        code: &str,
        detail: Option<&orbit_core::refusal::Refusal>,
    ) {
        if let Job::Write {
            request_id,
            request,
        } = &job
        {
            // Reached here, a write was with the helper: lost, never resent.
            self.calendar_write_failed(request_id, request, code, true);
            return;
        }
        match job {
            // Ended above.
            Job::Write { .. } => {}
            Job::Permission => {
                self.calendar.read_failure = Some(
                    detail
                        .cloned()
                        .unwrap_or_else(|| Runtime::bridge_refusal_code(code)),
                );
            }
            Job::Inventory { .. } => {
                self.calendar.inventory_valid = false;
                self.calendar.inventory_failure = Some(
                    detail
                        .cloned()
                        .unwrap_or_else(|| Runtime::bridge_refusal_code(code)),
                );
            }
            Job::Window {
                purpose, window, ..
            } => match code {
                "calendar_invalidated" => self.calendar_window_changed(purpose),
                "calendar_read_expired" | "calendar_reply_timeout" => {
                    // EventKit or its helper ran out of time on this window:
                    // read it in halves rather than turning absence into failure.
                    let span = window.end_unix_ms.saturating_sub(window.start_unix_ms);
                    let halved = match purpose {
                        Purpose::Screen => self.calendar.screen.as_mut().map(|screen| {
                            screen.window_ms = (span / 2).max(MIN_WINDOW_MS);
                            span > MIN_WINDOW_MS
                        }),
                        Purpose::Deimos => self
                            .calendar
                            .deimos
                            .as_mut()
                            .map(|walk| walk.halve_window(span)),
                        // Ten minutes are never split: the call goes without
                        // its event.
                        Purpose::Call => None,
                    };
                    if halved != Some(true) {
                        self.calendar_purpose_failed_detail(purpose, code, detail);
                    }
                }
                "calendar_permission_required" => {
                    self.calendar.view.authorization = CalendarAuthorization::Denied;
                    self.calendar_purpose_failed_detail(purpose, code, detail);
                }
                _ => self.calendar_purpose_failed_detail(purpose, code, detail),
            },
        }
    }

    /// A read for the screen or for Deimos ended, by name.
    fn calendar_purpose_failed(&mut self, purpose: Purpose, code: &str) {
        self.calendar_purpose_failed_detail(purpose, code, None);
    }

    fn calendar_purpose_failed_detail(
        &mut self,
        purpose: Purpose,
        code: &str,
        detail: Option<&orbit_core::refusal::Refusal>,
    ) {
        let refusal = detail
            .cloned()
            .unwrap_or_else(|| Runtime::bridge_refusal_code(code));
        match purpose {
            Purpose::Screen => {
                self.calendar.screen = None;
                self.calendar.complete = None;
                self.calendar.read_failure = Some(
                    detail
                        .cloned()
                        .unwrap_or_else(|| Runtime::bridge_refusal_code(code)),
                );
            }
            Purpose::Deimos => self.deimos_failed_refusal(refusal),
            Purpose::Call => {
                // The call is offered and recorded without its event; the
                // cause is logged, never content.
                self.calendar.call = None;
                self.log("call_calendar_read_failed", &[("code", failure_code(code))]);
            }
        }
    }
}

/// What a settled reply says about its wait: current, replaced or from
/// another generation.
fn settlement_failure(settlement: Settlement<()>) -> Option<CalendarWaitFailure> {
    match settlement {
        Settlement::Current(()) => None,
        Settlement::Foreign => Some(CalendarWaitFailure::ReplyReplaced),
        Settlement::Expired {
            started_generation,
            current_generation,
        } => Some(CalendarWaitFailure::GenerationChanged {
            started: started_generation,
            current: current_generation,
        }),
    }
}
