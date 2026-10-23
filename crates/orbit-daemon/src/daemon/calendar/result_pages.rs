//! A read's result admitted page by page: the permission, the calendars' list
//! and a window's events, each joined only with pages of the same store.
use super::*;

impl Runtime {
    pub(super) fn admit_calendar_result(&mut self, pending: &Pending, value: &Value) -> Result<()> {
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
            Job::Permission => self.admit_calendar_permission(value)?,
            Job::Inventory {
                after,
                read,
                restarts,
            } => self.admit_inventory_page(*after, read, *restarts, value, &facts)?,
            // Answered in `calendar_response`, before this is reached.
            Job::Write { .. } => {}
            Job::Window {
                purpose,
                window,
                after,
                read,
                epoch,
            } => self.admit_window_page(*purpose, *window, *after, read, *epoch, value, &facts)?,
        }
        Ok(())
    }

    /// Swift permissions returns this field alongside unrelated OS
    /// permissions. Only the exact request's calendar value is used.
    pub(super) fn admit_calendar_permission(&mut self, value: &Value) -> Result<()> {
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
        self.calendar.read_failure = None;
        Ok(())
    }

    /// One page of the calendars' list: the next one asked for, the whole
    /// list kept, or the list read again from the start when a calendar
    /// came or went between two pages, a bounded number of times.
    pub(super) fn admit_inventory_page(
        &mut self,
        after: u32,
        read: &Option<InventoryRead>,
        restarts: u8,
        value: &Value,
        facts: &CalendarReadFacts<'_>,
    ) -> Result<()> {
        let page: CalendarInventoryPage = decode_result_body("calendar_sources", value)?;
        page.validate()?;
        page.validate_response(after, facts)?;
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
                        restarts,
                    });
                }
                None => {
                    let count = u32::try_from(joined.calendars.len()).unwrap_or(u32::MAX);
                    self.state.set_calendar_count(Some(count));
                    self.calendar.view.calendars = Some(joined.calendars);
                    self.calendar.inventory_valid = true;
                    self.calendar.inventory_failure = None;
                }
            },
            // A calendar was added or removed between two pages: list
            // again from the start, a bounded number of times.
            Err(_) if restarts < WINDOW_RESTARTS => {
                self.send_calendar_job(Job::Inventory {
                    after: 0,
                    read: None,
                    restarts: restarts.saturating_add(1),
                });
            }
            Err(error) => return Err(error.into()),
        }
        Ok(())
    }

    /// One page of a window's events: the next one asked for, or the
    /// window read whole; a store that changed meanwhile reads it again.
    #[allow(
        clippy::too_many_arguments,
        reason = "the window job's own fields, as the job carries them"
    )]
    pub(super) fn admit_window_page(
        &mut self,
        purpose: Purpose,
        window: CalendarWindow,
        after: u32,
        read: &Option<WindowRead>,
        epoch: u64,
        value: &Value,
        facts: &CalendarReadFacts<'_>,
    ) -> Result<()> {
        let page: CalendarReadPage = decode_result_body("calendar_snapshot", value)?;
        page.validate()?;
        let query = CalendarReadQuery {
            start_unix_ms: window.start_unix_ms,
            end_unix_ms: window.end_unix_ms,
            after_index: after,
        };
        page.validate_response(&query, facts)?;
        if epoch != self.calendar.epoch {
            // The store changed since this window's first page.
            self.calendar_window_changed(purpose);
            return Ok(());
        }
        let joined = match read.clone() {
            None => WindowRead::start(page),
            Some(mut joined) => joined.push(page).map(|()| joined),
        };
        let Ok(joined) = joined else {
            self.calendar_window_changed(purpose);
            return Ok(());
        };
        match joined.next_after_index() {
            Some(next) => self.send_calendar_job(Job::Window {
                purpose,
                window,
                after: next,
                read: Some(joined),
                epoch,
            }),
            None => self.calendar_window_read(purpose, joined)?,
        }
        Ok(())
    }

    /// One window read whole, handed to whoever asked for it.
    pub(super) fn calendar_window_read(
        &mut self,
        purpose: Purpose,
        read: WindowRead,
    ) -> Result<()> {
        if read.events.iter().any(|event| {
            self.calendar
                .view
                .calendars
                .as_ref()
                .is_none_or(|known| !known.iter().any(|entry| entry.reference == event.calendar))
        }) {
            // An event from a calendar the list does not name yet: list again,
            // so the screen and a meeting association can name it.
            self.calendar.wanted_inventory = true;
        }
        match purpose {
            Purpose::Deimos => self.deimos_window_read(read),
            Purpose::Call => {
                self.calendar.call = None;
                self.calls.call_calendar_read(read.events);
                Ok(())
            }
            Purpose::Screen => {
                let Some(screen) = self.calendar.screen.as_mut() else {
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
                    && let Some(screen) = self.calendar.screen.take()
                {
                    self.calendar.complete = Some(Box::new(assemble_view(screen.windows)?));
                    self.calendar.read_failure = None;
                }
                Ok(())
            }
        }
    }

    /// The store changed under a window being read in pages: read it again
    /// from its first page, a bounded number of times, then say so.
    pub(super) fn calendar_window_changed(&mut self, purpose: Purpose) {
        let restarts = match purpose {
            Purpose::Screen => self.calendar.screen.as_mut().map(|screen| {
                screen.restarts = screen.restarts.saturating_add(1);
                screen.restarts
            }),
            Purpose::Deimos => self
                .calendar
                .deimos
                .as_mut()
                .map(|walk| walk.note_restart()),
            Purpose::Call => self.calendar.call.as_mut().map(|call| {
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
}
