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

impl Runtime {
    /// What the helper says about itself, and about the store changing.
    /// Revocation precedes any fallible native/Store consumer in native_event.
    /// Whatever that made due — a read the permission now allows, a window the
    /// store changed under — is sent at once, not on the next tick.
    pub(in crate::daemon) fn observe_calendar_native(
        &mut self,
        event: &Value,
        update: super::super::native_host::HostUpdate,
    ) {
        self.observe_calendar_event(event, update);
        self.pump_calendar();
    }

    fn observe_calendar_event(
        &mut self,
        event: &Value,
        update: super::super::native_host::HostUpdate,
    ) {
        let name = NativeEventHeader::from_value(event).map_or("", |header| header.event);
        if update.authority_changed || update.stopped() {
            // Each revocation says what happened: her stop, the Mac cutting
            // its capture (not a privacy stop, REF-12), or a new helper epoch.
            let (code, refusal) = if update.privacy_stopped {
                (
                    "privacy_stopped",
                    orbit_core::refusal::Refusal::PrivacyStopped { since_ms: None },
                )
            } else if update.capture_cut {
                (
                    "capture_stopped",
                    Runtime::bridge_refusal_code("capture_stopped"),
                )
            } else {
                (
                    "calendar_read_restarted",
                    self.features.calendar.pending.as_ref().map_or_else(
                        || Runtime::bridge_refusal_code("calendar_read_restarted"),
                        |pending| {
                            CalendarWaitFailure::HelperRestarted {
                                before: pending.host.clone(),
                                after: self.features.native_host.identity().cloned(),
                            }
                            .refusal()
                        },
                    ),
                )
            };
            self.revoke_calendar(code, refusal);
        }
        if name == "calendar_invalidated" {
            // The store changed (a sync, an edit). Pages read before and after
            // are never joined, and what is on screen is read again by itself.
            self.features.calendar.epoch = self.features.calendar.epoch.wrapping_add(1);
            self.features.calendar.complete = None;
            self.features.calendar.inventory_valid = false;
            self.features.calendar.inventory_failure = None;
            self.reread_calendar_on_change();
        }
        if restart_value(event) {
            // The helper restarted or went away: anything in flight is lost and
            // the permission is not known again until it says so.
            self.features.calendar.epoch = self.features.calendar.epoch.wrapping_add(1);
            self.features.calendar.complete = None;
            self.features.calendar.view.authorization = CalendarAuthorization::Unknown;
            self.features.calendar.inventory_valid = false;
            self.features.calendar.inventory_failure = None;
            self.state.set_calendar_count(None);
            if let Some(pending) = self.features.calendar.pending.take() {
                self.features.calendar.reply.cancel();
                let failure = CalendarWaitFailure::HelperRestarted {
                    before: pending.host,
                    after: self.features.native_host.identity().cloned(),
                };
                self.calendar_job_failed_detail(
                    pending.job,
                    failure.code(),
                    Some(&failure.refusal()),
                );
            }
            self.features.calendar.wanted_inventory = true;
            if let Some(range) = self.features.calendar.screen_range {
                self.features.calendar.read_failure = None;
                self.features.calendar.screen = Some(ScreenRead::new(range));
            }
        }
        let response = if name == "response" {
            NativeResponseFields::from_value(event).ok()
        } else {
            None
        };
        let state = match (name, response.as_ref()) {
            ("state", _) => event,
            ("response", Some(response)) if response.ok => {
                let Some(result) = response.result.as_ref() else {
                    return;
                };
                result
            }
            _ => return,
        };
        // Full host state/heartbeat observations report the permission; they
        // never carry agenda. Dedicated results are matched by exact ID below.
        if let Some(CalendarField::Present(value)) = CalendarObservation::deserialize(state)
            .ok()
            .map(|observation| observation.calendar)
        {
            match serde_json::to_vec(&value)
                .ok()
                .and_then(|bytes| calendar::decode_status(&bytes).ok())
            {
                Some(status) => self.calendar_status_observation(status),
                None => {
                    // Unreadable is unknown, never «denied».
                    self.features.calendar.view.authorization = CalendarAuthorization::Unknown;
                }
            }
        }
    }

    fn calendar_status_observation(&mut self, status: CalendarStatus) {
        let before = self.features.calendar.view.authorization;
        self.features.calendar.view.authorization = status.authorization;
        if before == CalendarAuthorization::FullAccess
            && status.authorization != CalendarAuthorization::FullAccess
        {
            // A9: withdrawing the permission cuts the agenda off here too. What
            // was on screen goes, and Deimos's next question is refused.
            let refusal =
                if let orbit_core::topics::Knowledge::Absent { refusal } = self.calendar_access() {
                    refusal
                } else {
                    orbit_core::refusal::Refusal::PermissionUnknown {
                        permission: orbit_core::refusal::Permission::Calendar,
                    }
                };
            self.revoke_calendar("calendar_permission_required", refusal);
        } else if before != CalendarAuthorization::FullAccess
            && status.authorization == CalendarAuthorization::FullAccess
        {
            // A1: the moment Mars grants it, the agenda arrives by itself.
            self.features.calendar.wanted_inventory = true;
            self.features.calendar.read_failure = None;
            if let Some(range) = self.features.calendar.screen_range {
                self.features.calendar.screen = Some(ScreenRead::new(range));
            }
        }
    }

    /// The store changed: what is on screen is read again, and so is the list
    /// of calendars. Nothing is asked of anyone.
    fn reread_calendar_on_change(&mut self) {
        let calendar = &mut self.features.calendar;
        calendar.wanted_inventory = true;
        self.restart_screen_read();
    }

    /// The agenda on screen is read again from its first window. P9-21: a
    /// read already under way starts over too — the windows it had joined
    /// were read before the change, and keeping them would leave old days on
    /// screen (a write between two windows of a long range). Restarts count
    /// for the whole read, not per window, so a store that never holds still
    /// ends the read by name (`WINDOW_RESTARTS`) instead of starting it over
    /// for ever. A restart is counted here when it throws away windows already
    /// joined; a window still in flight is counted when its stale answer
    /// arrives (`calendar_window_changed`), never twice.
    pub(super) fn restart_screen_read(&mut self) {
        let in_flight = self
            .features
            .calendar
            .pending
            .as_ref()
            .is_some_and(|pending| {
                matches!(
                    pending.job,
                    Job::Window {
                        purpose: Purpose::Screen,
                        ..
                    }
                )
            });
        let calendar = &mut self.features.calendar;
        calendar.complete = None;
        calendar.read_failure = None;
        let Some(range) = calendar.screen_range else {
            return;
        };
        let (mut restarts, discarded) = calendar.screen.as_ref().map_or((0, false), |screen| {
            (screen.restarts, !screen.windows.is_empty())
        });
        if discarded && !in_flight {
            restarts = restarts.saturating_add(1);
        }
        let mut screen = ScreenRead::new(range);
        screen.restarts = restarts;
        calendar.screen = Some(screen);
        if restarts > WINDOW_RESTARTS {
            self.calendar_purpose_failed(Purpose::Screen, "calendar_invalidated");
        }
    }

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
            .features
            .calendar
            .pending
            .as_ref()
            .is_none_or(|pending| pending.id != ticket)
        {
            return true;
        }
        let Some(pending) = self.features.calendar.pending.take() else {
            return true;
        };
        let failure = self.calendar_pending_failure(&pending, 0);
        let settlement = self
            .features
            .calendar
            .reply
            .settle(&pending.ticket, self.state.generation());
        let settlement_failure = match settlement {
            Settlement::Current(()) => None,
            Settlement::Foreign => Some(CalendarWaitFailure::ReplyReplaced),
            Settlement::Expired {
                started_generation,
                current_generation,
            } => Some(CalendarWaitFailure::GenerationChanged {
                started: started_generation,
                current: current_generation,
            }),
        };
        if let Some(failure) = failure.or(settlement_failure) {
            self.calendar_job_failed_detail(pending.job, failure.code(), Some(&failure.refusal()));
            self.retire_native_calendar();
        } else if let Job::Write {
            request_id,
            request,
        } = &pending.job
        {
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
                            self.admit_calendar_write(&pending, request_id, request, result)
                                .map_err(|problem| problem.to_string())
                        },
                    );
                    if let Err(problem) = admitted {
                        self.log(
                            "calendar_reply_invalid",
                            &[("command", request.command().0), ("cause", &problem)],
                        );
                        self.calendar_write_failed(
                            request_id,
                            request,
                            "calendar_reply_invalid",
                            true,
                        );
                    }
                }
                Err(_) => {
                    self.calendar_write_failed(request_id, request, "calendar_reply_invalid", true);
                }
            }
        } else if let Ok(response) = NativeResponseFields::from_value(event) {
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
        self.pump_calendar();
        true
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
            self.features.calendar.view.authorization,
            self.state.privacy_stopped() || pending.host.stopped,
        )
    }

    fn admit_calendar_result(&mut self, pending: &Pending, value: &Value) -> Result<()> {
        let (instance, generation, authorization, stopped) = self.calendar_facts(pending);
        let facts = CalendarReadFacts {
            native_instance_id: &instance,
            privacy_generation: generation,
            // The helper already checked the permission for this read; a
            // status still marked unknown here does not make its answer false.
            authorization: if authorization == CalendarAuthorization::Unknown {
                CalendarAuthorization::FullAccess
            } else {
                authorization
            },
            privacy_stopped: stopped,
            elapsed_since_request_ns: elapsed(Instant::now(), pending.started),
        };
        match &pending.job {
            Job::Permission => {
                // Swift permissions returns this field alongside unrelated OS
                // permissions. Only the exact request's calendar value is used.
                let authorization =
                    decode_result_body::<CalendarPermissionReply>("permissions", value)?.calendars;
                self.state = orbit_core::decide(
                    &self.state,
                    orbit_core::Event::CalendarAuthorization(authorization),
                    now(),
                )?
                .state;
                self.calendar_status_observation(CalendarStatus {
                    authorization,
                    physical_read_pending: false,
                    failure: None,
                });
                self.features.calendar.read_failure = None;
            }
            Job::Inventory {
                after,
                read,
                restarts,
            } => {
                let page: CalendarInventoryPage = decode_result_body("calendar_sources", value)?;
                page.validate()?;
                page.validate_response(*after, &facts)?;
                let joined = match read.clone() {
                    None => InventoryRead::start(page),
                    Some(mut joined) => joined.push(page).map(|()| joined),
                };
                match joined {
                    Ok(joined) => match joined.next_after_index() {
                        Some(next) => {
                            self.send_calendar_job(Job::Inventory {
                                after: next,
                                read: Some(joined),
                                restarts: *restarts,
                            });
                        }
                        None => {
                            let count = u32::try_from(joined.calendars.len()).unwrap_or(u32::MAX);
                            self.state.set_calendar_count(Some(count));
                            self.features.calendar.view.calendars = Some(joined.calendars);
                            self.features.calendar.inventory_valid = true;
                            self.features.calendar.inventory_failure = None;
                        }
                    },
                    // A calendar was added or removed between two pages: list
                    // again from the start, a bounded number of times.
                    Err(_) if *restarts < WINDOW_RESTARTS => {
                        self.send_calendar_job(Job::Inventory {
                            after: 0,
                            read: None,
                            restarts: restarts.saturating_add(1),
                        });
                    }
                    Err(error) => return Err(error.into()),
                }
            }
            // Answered in `calendar_response`, before this is reached.
            Job::Write { .. } => {}
            Job::Window {
                purpose,
                window,
                after,
                read,
                epoch,
            } => {
                let page: CalendarReadPage = decode_result_body("calendar_snapshot", value)?;
                page.validate()?;
                let query = CalendarReadQuery {
                    start_unix_ms: window.start_unix_ms,
                    end_unix_ms: window.end_unix_ms,
                    after_index: *after,
                };
                page.validate_response(&query, &facts)?;
                if *epoch != self.features.calendar.epoch {
                    // The store changed since this window's first page.
                    self.calendar_window_changed(*purpose);
                    return Ok(());
                }
                let joined = match read.clone() {
                    None => WindowRead::start(page),
                    Some(mut joined) => joined.push(page).map(|()| joined),
                };
                let Ok(joined) = joined else {
                    self.calendar_window_changed(*purpose);
                    return Ok(());
                };
                match joined.next_after_index() {
                    Some(next) => self.send_calendar_job(Job::Window {
                        purpose: *purpose,
                        window: *window,
                        after: next,
                        read: Some(joined),
                        epoch: *epoch,
                    }),
                    None => self.calendar_window_read(*purpose, joined)?,
                }
            }
        }
        Ok(())
    }

    /// One window read whole, handed to whoever asked for it.
    fn calendar_window_read(&mut self, purpose: Purpose, read: WindowRead) -> Result<()> {
        if read.events.iter().any(|event| {
            self.features
                .calendar
                .view
                .calendars
                .as_ref()
                .is_none_or(|known| !known.iter().any(|entry| entry.reference == event.calendar))
        }) {
            // An event from a calendar the list does not name yet: list again,
            // so the screen and a meeting association can name it.
            self.features.calendar.wanted_inventory = true;
        }
        match purpose {
            Purpose::Deimos => self.deimos_window_read(read),
            Purpose::Call => {
                self.features.calendar.call = None;
                self.call_calendar_read(read.events);
                Ok(())
            }
            Purpose::Screen => {
                let Some(screen) = self.features.calendar.screen.as_mut() else {
                    return Ok(());
                };
                if read.window.start_unix_ms != screen.next_from {
                    // A window asked for before the read started over: not the
                    // one this read needs next, so it is not joined.
                    return Ok(());
                }
                // Restarts are not reset here: they count for the whole read.
                screen.next_from = read.window.end_unix_ms;
                screen.windows.push(read);
                if screen.next_window().is_none()
                    && let Some(screen) = self.features.calendar.screen.take()
                {
                    self.features.calendar.complete =
                        Some(Box::new(assemble_view(screen.windows)?));
                    self.features.calendar.read_failure = None;
                }
                Ok(())
            }
        }
    }

    /// The store changed under a window being read in pages: read it again
    /// from its first page, a bounded number of times, then say so.
    fn calendar_window_changed(&mut self, purpose: Purpose) {
        let restarts = match purpose {
            Purpose::Screen => self.features.calendar.screen.as_mut().map(|screen| {
                screen.restarts = screen.restarts.saturating_add(1);
                screen.restarts
            }),
            Purpose::Deimos => self
                .features
                .calendar
                .deimos
                .as_mut()
                .map(|walk| walk.note_restart()),
            Purpose::Call => self.features.calendar.call.as_mut().map(|call| {
                call.sent = false;
                call.restarts = call.restarts.saturating_add(1);
                call.restarts
            }),
        };
        // Within the bound `next_calendar_job` asks for the same window again,
        // from its first page; past it the read says the agenda kept changing.
        if let Some(count) = restarts.filter(|count| *count > WINDOW_RESTARTS) {
            let refusal = orbit_core::refusal::Refusal::ReadInterrupted {
                cause: orbit_core::refusal::ReadInterruption::CalendarChanged {
                    changes: u64::from(count),
                    max_restarts: u64::from(WINDOW_RESTARTS),
                },
            };
            self.calendar_purpose_failed_detail(purpose, "calendar_invalidated", Some(&refusal));
        }
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
                self.features.calendar.read_failure = Some(
                    detail
                        .cloned()
                        .unwrap_or_else(|| Runtime::bridge_refusal_code(code)),
                );
            }
            Job::Inventory { .. } => {
                self.features.calendar.inventory_valid = false;
                self.features.calendar.inventory_failure = Some(
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
                        Purpose::Screen => self.features.calendar.screen.as_mut().map(|screen| {
                            screen.window_ms = (span / 2).max(MIN_WINDOW_MS);
                            span > MIN_WINDOW_MS
                        }),
                        Purpose::Deimos => self
                            .features
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
                    self.features.calendar.view.authorization = CalendarAuthorization::Denied;
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
                self.features.calendar.screen = None;
                self.features.calendar.complete = None;
                self.features.calendar.read_failure = Some(
                    detail
                        .cloned()
                        .unwrap_or_else(|| Runtime::bridge_refusal_code(code)),
                );
            }
            Purpose::Deimos => self.deimos_failed_refusal(refusal),
            Purpose::Call => {
                // The call is offered and recorded without its event; the
                // cause is logged, never content.
                self.features.calendar.call = None;
                self.log("call_calendar_read_failed", &[("code", failure_code(code))]);
            }
        }
    }
}
