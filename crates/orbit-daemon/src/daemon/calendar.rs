//! Reads of Mars's agenda — for the agenda on screen and for Deimos (REF-10) —
//! and the writes she makes from Orbit (P9-21, `writes.rs`).
//!
//! The macOS calendar permission is the only decision. Nothing is listed for
//! choosing, selected, copied or renewed: a read asks the helper for one
//! window of every calendar at a time, page by page, and joins the pages. The
//! helper has one EventKit slot, so the requests are sent one after another:
//! a write first, because Mars is waiting on it and it changes what a read
//! would return; then Deimos's reads, because a question is waiting on them.
//!
//! RAM only. No storage, credentials or hub traffic here, except the answer
//! to Deimos.
use std::{collections::VecDeque, time::Instant};

use orbit_core::{
    calendar::{
        CalendarAuthorization, CalendarReadQuery, CalendarWindow, InventoryRead,
        PERMISSION_LIFETIME_NS, READ_LIFETIME_NS, READ_WINDOW_MS, WRITE_LIFETIME_NS, WindowRead,
    },
    calendar_control::CalendarView,
    wire::native::NativeEventHeader,
};
use serde_json::{Value, json};

use super::{
    Runtime,
    native_host::HostIdentity,
    ticketed::{Settlement, Ticketed, WorkTicket},
};

mod commands;
mod deimos;
mod responses;
mod writes;

use writes::WriteRequest;

pub(super) use deimos::Cursor;

#[cfg(test)]
mod bridge_cases;
#[cfg(test)]
pub(super) mod tests;
#[cfg(test)]
mod write_cases;

const PREFIX: &str = "calendar-local-";

/// How many times one window is read again because the store changed under it.
///
/// Protects: a read from looping forever on a calendar that keeps changing
/// while it is read (a sync storm). Why three: one change during a read is
/// ordinary — iCloud syncing a moment ago — and a fourth in a row means the
/// agenda is not holding still long enough for any coherent answer. It is not
/// a limit on what is read: after it the read says, by name and with the
/// count, that the agenda kept changing.
const WINDOW_RESTARTS: u8 = 3;

/// The narrowest window worth asking for again when EventKit ran out of time.
///
/// A window EventKit cannot enumerate within the helper's read deadline
/// (`READ_LIFETIME_NS`, five seconds) is split in half and asked for again, so
/// a dense stretch is read in smaller pieces instead of failing. Protects: the
/// halving from going on forever. Why one hour: below it a window holds the
/// same few events and a slower read cannot come from their number.
const MIN_WINDOW_MS: i64 = 3_600_000;

/// How long past a request's own deadline the daemon waits for the helper to
/// say it expired before deciding so itself. The helper answers at the
/// deadline; this only covers an answer that was lost.
const DEADLINE_MARGIN_NS: u64 = 2_000_000_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Purpose {
    /// The agenda on screen in the app.
    Screen,
    /// A question Deimos is waiting on.
    Deimos,
    /// P9-11: what is on now, for the event of a call.
    Call,
}

/// P9-11: the agenda around a call, one short window read once.
struct CallRead {
    window: CalendarWindow,
    sent: bool,
    restarts: u8,
}

enum Job {
    Permission,
    Inventory {
        after: u32,
        read: Option<InventoryRead>,
        restarts: u8,
    },
    Window {
        purpose: Purpose,
        window: CalendarWindow,
        after: u32,
        read: Option<WindowRead>,
        epoch: u64,
    },
    /// P9-21: one write Mars made, under her client's identifier.
    Write {
        request_id: String,
        request: WriteRequest,
    },
}

impl Job {
    fn lifetime_ns(&self) -> u64 {
        match self {
            // A permission request waits for a person; everything else waits
            // for EventKit. A write has its own deadline: past it the helper
            // says the outcome is unknown instead of discarding an answer.
            Self::Permission => PERMISSION_LIFETIME_NS,
            Self::Write { .. } => WRITE_LIFETIME_NS,
            Self::Inventory { .. } | Self::Window { .. } => READ_LIFETIME_NS,
        }
    }

    fn reads(&self) -> bool {
        matches!(self, Self::Inventory { .. } | Self::Window { .. })
    }
}

struct Pending {
    id: String,
    host: HostIdentity,
    ticket: WorkTicket,
    started: Instant,
    job: Job,
}

enum CalendarWaitFailure {
    PrivacyStopped,
    GenerationChanged {
        started: u64,
        current: u64,
    },
    HelperRestarted {
        before: HostIdentity,
        after: Option<HostIdentity>,
    },
    BackendUnavailable,
    HelperStopped,
    ReplyReplaced,
    ClockUnavailable,
    ReplyTimeout {
        waited_ms: u64,
        deadline_ms: u64,
    },
}

impl CalendarWaitFailure {
    fn code(&self) -> &'static str {
        match self {
            Self::PrivacyStopped => "privacy_stopped",
            Self::GenerationChanged { .. } => "calendar_generation_changed",
            Self::HelperRestarted { .. } => "calendar_read_restarted",
            Self::BackendUnavailable => "calendar_backend_unavailable",
            Self::HelperStopped => "calendar_helper_stopped",
            Self::ReplyReplaced => "calendar_reply_replaced",
            Self::ClockUnavailable => "calendar_clock_unavailable",
            Self::ReplyTimeout { .. } => "calendar_reply_timeout",
        }
    }

    fn refusal(&self) -> orbit_core::refusal::Refusal {
        use orbit_core::refusal::{Operation, ReadInterruption, Refusal};
        match self {
            Self::PrivacyStopped => Refusal::PrivacyStopped { since_ms: None },
            Self::GenerationChanged { started, current } => Refusal::ReadInterrupted {
                cause: ReadInterruption::GenerationChanged {
                    started_generation: *started,
                    current_generation: *current,
                },
            },
            Self::HelperRestarted { before, after } => Refusal::ReadInterrupted {
                cause: ReadInterruption::HelperRestarted {
                    previous_generation: Some(before.generation),
                    current_generation: after.as_ref().map(|host| host.generation),
                    previous_instance: Some(before.id.clone()),
                    current_instance: after.as_ref().map(|host| host.id.clone()),
                },
            },
            Self::BackendUnavailable => Refusal::CalendarBackendUnavailable,
            Self::HelperStopped => Refusal::ReadInterrupted {
                cause: ReadInterruption::CalendarHelperStopped,
            },
            Self::ReplyReplaced => Refusal::ReadInterrupted {
                cause: ReadInterruption::CalendarReplyReplaced,
            },
            Self::ClockUnavailable => Refusal::HelperReplyInvalid {
                field: "monotonic_clock".into(),
                expected: "monotonic_read".into(),
                received_type: "unavailable".into(),
            },
            Self::ReplyTimeout {
                waited_ms,
                deadline_ms,
            } => Refusal::Timeout {
                operation: Operation::Calendar,
                waited_ms: *waited_ms,
                deadline_ms: *deadline_ms,
            },
        }
    }
}

/// The agenda on screen: the range the app asked for, read window by window.
struct ScreenRead {
    range: CalendarWindow,
    next_from: i64,
    windows: Vec<WindowRead>,
    window_ms: i64,
    restarts: u8,
}

impl ScreenRead {
    fn new(range: CalendarWindow) -> Self {
        Self {
            range,
            next_from: range.start_unix_ms,
            windows: Vec::new(),
            window_ms: READ_WINDOW_MS,
            restarts: 0,
        }
    }

    fn next_window(&self) -> Option<CalendarWindow> {
        (self.next_from < self.range.end_unix_ms).then(|| CalendarWindow {
            start_unix_ms: self.next_from,
            end_unix_ms: self
                .next_from
                .saturating_add(self.window_ms)
                .min(self.range.end_unix_ms),
        })
    }
}

#[derive(Default)]
pub(super) struct CalendarRuntime {
    pub view: CalendarView,
    /// Failure of the latest screen read, kept until that range is read again.
    pub(super) read_failure: Option<orbit_core::refusal::Refusal>,
    pub(super) inventory_valid: bool,
    /// A failed refresh cannot turn the last successful listing into a new
    /// fact. The legacy view keeps its prior colors; keyed reads report this
    /// specific failed attempt until a complete native listing replaces it.
    pub(super) inventory_failure: Option<orbit_core::refusal::Refusal>,
    /// The latest complete native read, delivered in bounded keyed read pages.
    pub(super) complete: Option<Box<orbit_core::calendar::CalendarSnapshot>>,
    pending: Option<Pending>,
    reply: Ticketed<()>,
    /// A job that could not be sent because the helper channel was full,
    /// tried again first on the next turn so a half-read window is not lost.
    /// A parked write was never sent: sending it then is its first send.
    parked: Option<Job>,
    /// P9-21: writes accepted and not yet sent, oldest first. Their records
    /// are in `view.writes`.
    write_queue: VecDeque<(String, WriteRequest)>,
    /// Bumped whenever the store or the helper changed underneath
    /// (`calendar_invalidated`, a helper restart). Pages of one window read on
    /// both sides of a bump are never joined.
    epoch: u64,
    wanted_permission: bool,
    wanted_inventory: bool,
    /// The range the app last asked to see, read again when the agenda
    /// changes or the permission arrives, without anyone having to ask.
    screen_range: Option<CalendarWindow>,
    screen: Option<ScreenRead>,
    deimos: Option<deimos::Walk>,
    /// P9-11: the window around a call, while it is wanted.
    call: Option<CallRead>,
    /// The exact walk retired by a Mac capture cut, so its next cursor can
    /// name the cut rather than falsely claiming the reader restarted.
    cut_deimos: Option<String>,
}

impl CalendarRuntime {
    pub(super) fn inventory_refusal(&self) -> Option<orbit_core::refusal::Refusal> {
        self.inventory_failure.clone()
    }

    pub(super) fn topic_invalidation(&self) -> u64 {
        self.epoch
    }

    pub(super) fn topic_window(&self) -> Option<CalendarWindow> {
        self.screen_range
    }
}

impl Runtime {
    pub(super) fn calendar_access(&self) -> orbit_core::topics::Knowledge<()> {
        let authorization =
            if self.features.calendar.view.authorization == CalendarAuthorization::Unavailable {
                CalendarAuthorization::Unavailable
            } else if self.features.native_host.identity().is_none() {
                CalendarAuthorization::Unknown
            } else if self.calendar_backend_ready() {
                self.features.calendar.view.authorization
            } else {
                CalendarAuthorization::Unavailable
            };
        orbit_core::topics::agenda::access(authorization, self.state.privacy_stopped())
    }

    /// Privacy stop, a change of authority or a withdrawn permission: every
    /// read in progress is dropped, what is on screen goes, and the helper is
    /// told to retire a reply it may still be preparing. Idempotent: with
    /// nothing to retire it changes nothing and sends nothing.
    pub(super) fn revoke_calendar(
        &mut self,
        code: &'static str,
        refusal: orbit_core::refusal::Refusal,
    ) {
        self.revoke_calendar_writes(code);
        self.features.calendar.inventory_valid = false;
        self.features.calendar.inventory_failure = None;
        let calendar = &self.features.calendar;
        let in_flight = calendar.pending.is_some();
        let had_content = in_flight
            || calendar.parked.is_some()
            || calendar.screen.is_some()
            || calendar.deimos.is_some()
            || calendar.call.is_some()
            || calendar.complete.is_some();
        if !had_content {
            return;
        }
        let calendar = &mut self.features.calendar;
        // The physical slot stays marked busy until the helper answers; that
        // answer then finds nothing waiting for it.
        calendar.screen = None;
        calendar.call = None;
        calendar.complete = None;
        calendar.parked = None;
        calendar.read_failure = Some(refusal.clone());
        self.drop_deimos_walk(code, refusal);
        if in_flight {
            self.retire_native_calendar();
        }
    }

    /// Upkeep on the daemon's tick: a request whose answer never came is
    /// ended by name, and whatever waits is sent.
    pub(super) fn maintain_calendar(&mut self) {
        let failure = self
            .features
            .calendar
            .pending
            .as_ref()
            .and_then(|pending| self.calendar_pending_failure(pending, DEADLINE_MARGIN_NS));
        if let Some(failure) = failure
            && let Some(pending) = self.features.calendar.pending.take()
        {
            self.features.calendar.reply.cancel();
            self.calendar_job_failed_detail(pending.job, failure.code(), Some(&failure.refusal()));
            self.retire_native_calendar();
        }
        self.pump_calendar();
    }

    fn calendar_pending_failure(
        &self,
        pending: &Pending,
        margin_ns: u64,
    ) -> Option<CalendarWaitFailure> {
        if pending.job.reads() && self.state.privacy_stopped() {
            return Some(CalendarWaitFailure::PrivacyStopped);
        }
        if let Some(started) = self
            .features
            .calendar
            .reply
            .expired_generation(&pending.ticket, self.state.generation())
        {
            return Some(CalendarWaitFailure::GenerationChanged {
                started,
                current: self.state.generation(),
            });
        }
        if self.features.calendar.reply.ticket() != Some(&pending.ticket) {
            return Some(CalendarWaitFailure::ReplyReplaced);
        }
        let host = self.features.native_host.identity();
        if host.is_none_or(|host| {
            host.id != pending.host.id || host.generation != pending.host.generation
        }) {
            return Some(CalendarWaitFailure::HelperRestarted {
                before: pending.host.clone(),
                after: host.cloned(),
            });
        }
        if !self.features.native_host.calendar_backend_available() {
            return Some(CalendarWaitFailure::BackendUnavailable);
        }
        if pending.job.reads() && host.is_some_and(|host| host.stopped) {
            return Some(CalendarWaitFailure::HelperStopped);
        }
        let Some(age_ns) = elapsed(Instant::now(), pending.started) else {
            return Some(CalendarWaitFailure::ClockUnavailable);
        };
        let ceiling_ns = pending.job.lifetime_ns().saturating_add(margin_ns);
        (age_ns >= ceiling_ns).then_some(CalendarWaitFailure::ReplyTimeout {
            waited_ms: age_ns / 1_000_000,
            deadline_ms: ceiling_ns / 1_000_000,
        })
    }

    fn calendar_backend_ready(&self) -> bool {
        self.state.capabilities().native_helper()
            && self.features.native_host.calendar_backend_available()
            && self.features.native_host.identity().is_some()
    }

    /// P9-11: reads `window` (now until the lookahead) for the event of a
    /// call. A newer call replaces a read not answered yet.
    pub(super) fn read_calendar_for_call(&mut self, window: CalendarWindow) {
        self.features.calendar.call = Some(CallRead {
            window,
            sent: false,
            restarts: 0,
        });
        self.pump_calendar();
    }

    /// Sends the next helper request if the EventKit slot is free.
    pub(super) fn pump_calendar(&mut self) {
        if self.features.calendar.pending.is_some() || !self.calendar_backend_ready() {
            return;
        }
        // Reads wait while privacy is stopped and while macOS is known not to
        // grant the calendar; an unknown permission is asked of the helper,
        // which knows.
        let reading_allowed = !self.state.privacy_stopped()
            && self
                .features
                .native_host
                .identity()
                .is_some_and(|host| !host.stopped)
            && orbit_core::device_bridge::CalendarPermission::from_authorization(
                self.features.calendar.view.authorization,
            ) != orbit_core::device_bridge::CalendarPermission::NotGranted;
        let Some(job) = self.next_calendar_job(reading_allowed) else {
            return;
        };
        self.send_calendar_job(job);
    }

    fn next_calendar_job(&mut self, reading_allowed: bool) -> Option<Job> {
        let calendar = &mut self.features.calendar;
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
            && self
                .features
                .native_host
                .identity()
                .is_some_and(|host| !host.stopped)
            && let Some((request_id, request)) = self.features.calendar.write_queue.pop_front()
        {
            return Some(Job::Write {
                request_id,
                request,
            });
        }
        let calendar = &mut self.features.calendar;
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

    fn send_calendar_job(&mut self, job: Job) {
        let Some(host) = self.features.native_host.identity().cloned() else {
            self.features.calendar.parked = Some(job);
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
        let sender = self.native.clone();
        let Ok(permit) = sender.try_reserve() else {
            // The channel to the helper is full for a moment. Nothing was sent;
            // the same request goes first on the next turn.
            self.features.calendar.parked = Some(job);
            return;
        };
        let ticket = format!("{PREFIX}{}", super::id());
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
        let Some(work_ticket) = self.features.calendar.reply.start_external(
            self.state.generation(),
            self.turn.op.clone(),
            (),
        ) else {
            self.calendar_job_unsent(job, "calendar_busy");
            return;
        };
        if let Job::Write { request_id, .. } = &job {
            self.write_sent(request_id);
        }
        self.features.calendar.pending = Some(Pending {
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
    fn calendar_job_unsent(&mut self, job: Job, code: &str) {
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

    /// Best effort: tells the helper to retire a reply nobody will read. No
    /// data request, retry or audio stop.
    fn retire_native_calendar(&self) {
        if self.state.capabilities().native_helper()
            && self.features.native_host.calendar_backend_available()
        {
            let command = match crate::native::command(
                "calendar_disconnect",
                json!({"id":format!("{PREFIX}retire-{}", super::id())}),
            ) {
                Ok(command) => command,
                Err(error) => {
                    self.log(
                        "calendar_disconnect_invalid",
                        &[("cause", &error.to_string())],
                    );
                    return;
                }
            };
            match self.native.try_send(command) {
                Ok(()) => {}
                Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {
                    let capacity = self.native.max_capacity().to_string();
                    self.log(
                        "calendar_disconnect_undelivered",
                        &[("cause", "full"), ("capacity", &capacity)],
                    );
                }
                Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {
                    self.log("calendar_disconnect_undelivered", &[("cause", "closed")]);
                }
            }
        }
    }
}

fn elapsed(now: Instant, started: Instant) -> Option<u64> {
    now.checked_duration_since(started)
        .and_then(|value| u64::try_from(value.as_nanos()).ok())
}

/// The bridge code a helper failure is reported under. Only codes the
/// translator knows by name; a store that kept changing is its own code.
fn failure_code(code: &str) -> &'static str {
    match code {
        "calendar_permission_required" => "calendar_permission_required",
        "privacy_stopped" => "privacy_stopped",
        "calendar_busy" => "calendar_busy",
        "calendar_event_too_large" => "calendar_event_too_large",
        "calendar_limit_exceeded" => "calendar_limit_exceeded",
        "calendar_read_expired" => "calendar_read_expired",
        "calendar_invalidated" => "calendar_changed",
        "calendar_backend_unavailable" => "calendar_backend_unavailable",
        "calendar_helper_stopped" => "calendar_helper_stopped",
        "calendar_read_restarted" => "calendar_read_restarted",
        "calendar_generation_changed" => "calendar_generation_changed",
        "calendar_reply_timeout" => "calendar_reply_timeout",
        "calendar_reply_replaced" => "calendar_reply_replaced",
        "calendar_clock_unavailable" => "calendar_clock_unavailable",
        "calendar_reply_invalid" => "calendar_reply_invalid",
        "calendar_command_invalid" => "calendar_command_invalid",
        "capture_stopped" => "capture_stopped",
        _ => "calendar_read_failed",
    }
}

fn restart_value(value: &Value) -> bool {
    NativeEventHeader::from_value(value)
        .is_ok_and(|header| matches!(header.event, "ready" | "helper_disconnected"))
}
