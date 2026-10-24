//! The calendar's one order at a time: the next job the queue allows, sent to
//! the helper with its deadline, or answered at once when it cannot be sent.
use super::*;

impl Runtime {
    /// Sends the next helper request if the EventKit slot is free.
    pub(in crate::daemon) fn pump_calendar(&mut self) {
        if self.calendar.pending.is_some() || !self.calendar_backend_ready() {
            return;
        }
        // Reads wait while privacy is stopped and while macOS is known not to
        // grant the calendar; an unknown permission is asked of the helper,
        // which knows.
        let reading_allowed = !self.state.privacy_stopped()
            && self.helper.identity().is_some_and(|host| !host.stopped)
            && orbit_core::device_bridge::CalendarPermission::from_authorization(
                self.calendar.view.authorization,
            ) != orbit_core::device_bridge::CalendarPermission::NotGranted;
        let Some(job) = self.next_calendar_job(reading_allowed) else {
            return;
        };
        self.send_calendar_job(job);
    }

    pub(super) fn next_calendar_job(&mut self, reading_allowed: bool) -> Option<Job> {
        let calendar = &mut self.calendar;
        if let Some(job) = calendar.parked.take()
            && (reading_allowed || !job.reads())
        {
            return Some(job);
        }
        if std::mem::take(&mut calendar.wanted_permission) {
            return Some(Job::Permission);
        }
        // A write waits only for the stop and for a helper that is there; the
        // permission is the helper's to report, with EventKit's real state.
        if !self.state.privacy_stopped()
            && self.helper.identity().is_some_and(|host| !host.stopped)
            && let Some((request_id, request)) = self.calendar.write_queue.pop_front()
        {
            return Some(Job::Write {
                request_id,
                request,
            });
        }
        let calendar = &mut self.calendar;
        if !reading_allowed {
            return None;
        }
        let epoch = calendar.epoch;
        if let Some(window) = calendar
            .deimos
            .as_ref()
            .and_then(deimos::Walk::wanted_window)
        {
            return Some(Job::Window {
                purpose: Purpose::Deimos,
                window,
                after: 0,
                read: None,
                epoch,
            });
        }
        // A call waits on this one: it names the event while the call is
        // being confirmed, so it goes before the screen's range.
        if let Some(call) = calendar.call.as_mut().filter(|call| !call.sent) {
            call.sent = true;
            return Some(Job::Window {
                purpose: Purpose::Call,
                window: call.window,
                after: 0,
                read: None,
                epoch,
            });
        }
        if let Some(window) = calendar.screen.as_ref().and_then(ScreenRead::next_window) {
            return Some(Job::Window {
                purpose: Purpose::Screen,
                window,
                after: 0,
                read: None,
                epoch,
            });
        }
        if std::mem::take(&mut calendar.wanted_inventory) {
            return Some(Job::Inventory {
                after: 0,
                read: None,
                restarts: 0,
            });
        }
        None
    }

    pub(super) fn send_calendar_job(&mut self, job: Job) {
        let Some(host) = self.helper.identity().cloned() else {
            self.calendar.parked = Some(job);
            return;
        };
        let (name, mut fields) = match &job {
            Job::Permission => (
                "permissions",
                json!({"request":[crate::permissions::CALENDARS]}),
            ),
            Job::Inventory { after, .. } => (
                "calendar_sources",
                json!({"calendar_page": {"after_index": after}}),
            ),
            Job::Window { window, after, .. } => (
                "calendar_snapshot",
                json!({"calendar_query": CalendarReadQuery {
                    start_unix_ms: window.start_unix_ms,
                    end_unix_ms: window.end_unix_ms,
                    after_index: *after,
                }}),
            ),
            Job::Write { request, .. } => request.command(),
        };
        let Some(permit) = self.helper.reserve() else {
            // The channel to the helper is full for a moment. Nothing was sent;
            // the same request goes first on the next turn.
            self.calendar.parked = Some(job);
            return;
        };
        let ticket = format!("{PREFIX}{}", super::super::id());
        let Some(object) = fields.as_object_mut() else {
            self.calendar_job_unsent(job, "calendar_command_invalid");
            return;
        };
        object.insert("id".into(), json!(ticket));
        let command = match crate::native::command(name, fields) {
            Ok(command) => command,
            Err(error) => {
                self.log("calendar_command_invalid", &[("cause", &error.to_string())]);
                self.calendar_job_unsent(job, "calendar_command_invalid");
                return;
            }
        };
        let Some(work_ticket) =
            self.calendar
                .reply
                .start_external(self.state.generation(), self.turn.op.clone(), ())
        else {
            self.calendar_job_unsent(job, "calendar_busy");
            return;
        };
        if let Job::Write { request_id, .. } = &job {
            self.calendar.write_sent(request_id);
        }
        self.calendar.pending = Some(Pending {
            id: ticket,
            host,
            ticket: work_ticket,
            started: Instant::now(),
            job,
        });
        permit.send(command);
    }

    /// A job that ended before it reached the helper. A write is then known
    /// not to have been applied; every other job fails as it always did.
    pub(super) fn calendar_job_unsent(&mut self, job: Job, code: &str) {
        match job {
            Job::Write {
                request_id,
                request,
            } => self.calendar_write_failed(&request_id, &request, code, false),
            other @ (Job::Permission | Job::Inventory { .. } | Job::Window { .. }) => {
                self.calendar_job_failed(other, code);
            }
        }
    }
}
