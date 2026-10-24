//! What the helper says about the calendar outside a reply: a revocation, the
//! store changing, the helper restarting, and the permission it reports.
use super::*;

impl Runtime {
    /// What the helper says about itself, and about the store changing.
    /// Revocation precedes any fallible native/Store consumer in native_event.
    /// Whatever that made due — a read the permission now allows, a window the
    /// store changed under — is sent at once, not on the next tick.
    pub(in crate::daemon) fn observe_calendar_native(
        &mut self,
        event: &Value,
        update: super::super::super::native_host::HostUpdate,
    ) {
        self.observe_calendar_event(event, update);
        self.pump_calendar();
    }

    pub(super) fn observe_calendar_event(
        &mut self,
        event: &Value,
        update: super::super::super::native_host::HostUpdate,
    ) {
        let name = NativeEventHeader::from_value(event).map_or("", |header| header.event);
        if update.authority_changed || update.stopped() {
            let (code, refusal) = self.calendar_revocation(update);
            self.revoke_calendar(code, refusal);
        }
        if name == "calendar_invalidated" {
            self.calendar_store_changed();
        }
        if restart_value(event) {
            self.calendar_helper_restarted();
        }
        self.observe_calendar_permission(name, event);
    }

    /// Each revocation says what happened: the user's stop, the Mac cutting
    /// its capture (not a privacy stop), or a new helper epoch.
    pub(super) fn calendar_revocation(
        &self,
        update: super::super::super::native_host::HostUpdate,
    ) -> (&'static str, orbit_core::refusal::Refusal) {
        if update.privacy_stopped {
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
                self.calendar.pending.as_ref().map_or_else(
                    || Runtime::bridge_refusal_code("calendar_read_restarted"),
                    |pending| {
                        CalendarWaitFailure::HelperRestarted {
                            before: pending.host.clone(),
                            after: self.helper.identity().cloned(),
                        }
                        .refusal()
                    },
                ),
            )
        }
    }

    /// The store changed (a sync, an edit). Pages read before and after are
    /// never joined, and what is on screen is read again by itself.
    pub(super) fn calendar_store_changed(&mut self) {
        self.calendar.epoch = self.calendar.epoch.wrapping_add(1);
        self.calendar.complete = None;
        self.calendar.inventory_valid = false;
        self.calendar.inventory_failure = None;
        self.reread_calendar_on_change();
    }

    /// The helper restarted or went away: anything in flight is lost and the
    /// permission is not known again until it says so.
    pub(super) fn calendar_helper_restarted(&mut self) {
        self.calendar.epoch = self.calendar.epoch.wrapping_add(1);
        self.calendar.complete = None;
        self.calendar.view.authorization = CalendarAuthorization::Unknown;
        self.calendar.inventory_valid = false;
        self.calendar.inventory_failure = None;
        self.state.set_calendar_count(None);
        if let Some(pending) = self.calendar.pending.take() {
            self.calendar.reply.cancel();
            let failure = CalendarWaitFailure::HelperRestarted {
                before: pending.host,
                after: self.helper.identity().cloned(),
            };
            self.calendar_job_failed_detail(pending.job, failure.code(), Some(&failure.refusal()));
        }
        self.calendar.wanted_inventory = true;
        if let Some(range) = self.calendar.screen_range {
            self.calendar.read_failure = None;
            self.calendar.screen = Some(ScreenRead::new(range));
        }
    }

    /// Full host state/heartbeat observations report the permission; they
    /// never carry agenda. Dedicated results are matched by exact ID.
    pub(super) fn observe_calendar_permission(&mut self, name: &str, event: &Value) {
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
                    self.calendar.view.authorization = CalendarAuthorization::Unknown;
                }
            }
        }
    }

    pub(super) fn calendar_status_observation(&mut self, status: CalendarStatus) {
        let before = self.calendar.view.authorization;
        self.calendar.view.authorization = status.authorization;
        if before == CalendarAuthorization::FullAccess
            && status.authorization != CalendarAuthorization::FullAccess
        {
            // Withdrawing the permission cuts the agenda off here too. What
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
            // The moment the user grants it, the agenda arrives by itself.
            self.calendar.wanted_inventory = true;
            self.calendar.read_failure = None;
            if let Some(range) = self.calendar.screen_range {
                self.calendar.screen = Some(ScreenRead::new(range));
            }
        }
    }

    /// The store changed: what is on screen is read again, and so is the list
    /// of calendars. Nothing is asked of anyone.
    pub(super) fn reread_calendar_on_change(&mut self) {
        let calendar = &mut self.calendar;
        calendar.wanted_inventory = true;
        self.restart_screen_read();
    }

    /// The agenda on screen is read again from its first window. A
    /// read already under way starts over too — the windows it had joined
    /// were read before the change, and keeping them would leave old days on
    /// screen (a write between two windows of a long range). Restarts count
    /// for the whole read, not per window, so a store that never holds still
    /// ends the read by name (`WINDOW_RESTARTS`) instead of starting it over
    /// for ever. A restart is counted here when it throws away windows already
    /// joined; a window still in flight is counted when its stale answer
    /// arrives (`calendar_window_changed`), never twice.
    pub(in crate::daemon) fn restart_screen_read(&mut self) {
        let in_flight = self.calendar.pending.as_ref().is_some_and(|pending| {
            matches!(
                pending.job,
                Job::Window {
                    purpose: Purpose::Screen,
                    ..
                }
            )
        });
        let calendar = &mut self.calendar;
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
}
