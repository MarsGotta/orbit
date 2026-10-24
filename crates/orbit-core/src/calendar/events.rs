use std::collections::HashSet;

use super::{
    validation::{ascii_control, identifier, joined_interval},
    *,
};

impl CalendarSnapshot {
    /// Structure of a read the daemon assembled, or one a meeting association
    /// kept. Nothing here bounds how many events or calendars a read holds:
    /// the read was already paged to fit every message it crossed.
    pub fn validate(&self) -> Result<(), CalendarError> {
        self.scope.validate()?;
        self.window.validate()?;
        joined_interval(
            &self.clock_domain,
            self.capture_started_monotonic_ns,
            self.capture_completed_monotonic_ns,
        )?;
        if !crate::wire::canonical_uuid(&self.snapshot_id.to_lowercase())
            || self.window.scope_id != self.scope.scope_id
            || !crate::wire::unix_ms(self.observed_at_unix_ms)
            || self.system_timezone.is_empty()
            || self.system_timezone.len() > 128
            || self.provider_sync_status != "unknown"
            || self.enumeration != "bounded_local_read"
        {
            return Err(CalendarError::InvalidValue);
        }
        let mut unreadable = HashSet::new();
        for failure in &self.unreadable_calendars {
            failure.calendar.validate()?;
            if !self.scope.calendars.contains(&failure.calendar)
                || !matches!(
                    failure.reason,
                    CalendarFailure::SelectionUnavailable | CalendarFailure::ReadFailed
                )
                || !unreadable.insert(&failure.calendar.calendar_id)
            {
                return Err(CalendarError::InvalidValue);
            }
        }
        for omission in &self.omitted {
            omission.validate()?;
        }
        let window = CalendarWindow {
            start_unix_ms: self.window.start_unix_ms,
            end_unix_ms: self.window.end_unix_ms,
        };
        let mut observations = HashSet::new();
        for event in &self.events {
            if !self.scope.calendars.contains(&event.calendar) {
                return Err(CalendarError::ScopeChanged);
            }
            event.validate(
                &window,
                &self.system_timezone,
                self.scope.include_invitees,
                self.scope.include_event_url,
            )?;
            if unreadable.contains(&event.calendar.calendar_id)
                || !observations.insert(&event.observation_id)
            {
                return Err(CalendarError::InvalidValue);
            }
        }
        Ok(())
    }
}

impl CalendarOmission {
    pub(super) fn validate(&self) -> Result<(), CalendarError> {
        if self.reason.is_empty()
            || self.reason.len() > 64
            || ascii_control(&self.reason)
            || self.count == 0
        {
            return Err(CalendarError::InvalidValue);
        }
        Ok(())
    }
}

/// The fields a `CalendarOversizedEvent` may name.
pub const OVERSIZED_FIELDS: [&str; 11] = [
    "title",
    "location",
    "event_url",
    "event_timezone",
    "interpreted_timezone",
    "calendar_item_id",
    "event_id",
    "invitee_name",
    "invitee_url",
    "invitees",
    "event",
];

impl CalendarOversizedEvent {
    pub(super) fn validate(&self) -> Result<(), CalendarError> {
        self.calendar.validate()?;
        if self
            .calendar_item_id
            .as_deref()
            .is_some_and(|id| !identifier(id))
            || (self.calendar_item_id.is_none() && self.field != "calendar_item_id")
            || !crate::wire::unix_ms(self.start_unix_ms)
            || !crate::wire::unix_ms(self.end_unix_ms)
            || self.end_unix_ms < self.start_unix_ms
            || !OVERSIZED_FIELDS.contains(&self.field.as_str())
            || self.bytes <= self.limit
        {
            return Err(CalendarError::InvalidValue);
        }
        Ok(())
    }
}

impl CalendarOccurrence {
    /// One event on its own terms, wherever it lies: what a write answers
    /// with. Invitees and the URL are always read.
    pub(super) fn validate_alone(&self, timezone: &str) -> Result<(), CalendarError> {
        self.validate_fields(timezone, true, true)
    }

    /// One event on its own terms: identity, dates, zone, the window it was
    /// read for and the optional categories that read asked for.
    pub(super) fn validate(
        &self,
        window: &CalendarWindow,
        timezone: &str,
        include_invitees: bool,
        include_event_url: bool,
    ) -> Result<(), CalendarError> {
        self.validate_fields(timezone, include_invitees, include_event_url)?;
        // Match the EventKit reader's half-open overlap rule, including point
        // events. Never clip or expand an occurrence into the requested window.
        let overlaps = self.start_unix_ms < window.end_unix_ms
            && if self.start_unix_ms == self.end_unix_ms {
                self.start_unix_ms >= window.start_unix_ms
            } else {
                self.end_unix_ms > window.start_unix_ms
            };
        if !overlaps {
            return Err(CalendarError::InvalidValue);
        }
        Ok(())
    }

    fn validate_fields(
        &self,
        timezone: &str,
        include_invitees: bool,
        include_event_url: bool,
    ) -> Result<(), CalendarError> {
        self.calendar.validate()?;
        self.validate_identity_and_dates(timezone)?;
        self.validate_lengths()?;
        // Notes that did not travel say so with their numbers, and
        // never alongside notes that did.
        if let Some(left_out) = self.notes_left_out
            && (self.notes.is_some()
                || left_out.limit != MAX_NOTES_BYTES as u64
                || left_out.bytes <= left_out.limit)
        {
            return Err(CalendarError::InvalidValue);
        }
        // The reminders that travel are the ones a write may set,
        // under the same limits; the others are a count, never zero.
        if self.alarms_minutes.as_ref().is_some_and(|minutes| {
            minutes.len() > MAX_EVENT_ALARMS
                || minutes.iter().any(|value| *value > MAX_ALARM_MINUTES)
        }) || self.alarms_not_editable == Some(0)
        {
            return Err(CalendarError::InvalidValue);
        }
        if let Some(organizer) = &self.organizer {
            organizer.validate()?;
        }
        if let Some(recurrence) = &self.recurrence {
            recurrence.validate()?;
        }
        optional(
            include_invitees,
            self.invitees_status,
            self.invitees.is_some(),
        )?;
        optional(
            include_event_url,
            self.event_url_status,
            self.event_url.is_some(),
        )?;
        // Every invitee EventKit returned travels. A meeting with a great many
        // guests is still one event; its size is the page's business.
        for invitee in self.invitees.iter().flatten() {
            invitee.validate()?;
        }
        Ok(())
    }

    /// Its identifiers, dates, time zones and status are well formed.
    fn validate_identity_and_dates(&self, timezone: &str) -> Result<(), CalendarError> {
        if !crate::wire::canonical_uuid(&self.observation_id)
            || !identifier(&self.calendar_item_id)
            || self.event_id.as_deref().is_some_and(|id| !identifier(id))
            || !crate::wire::unix_ms(self.start_unix_ms)
            || !crate::wire::unix_ms(self.end_unix_ms)
            || self.end_unix_ms < self.start_unix_ms
            || self
                .original_occurrence_unix_ms
                .is_some_and(|value| !crate::wire::unix_ms(value))
            || self
                .last_modified_unix_ms
                .is_some_and(|value| !crate::wire::unix_ms(value))
            || self.is_floating != self.event_timezone.is_none()
            || self.interpreted_timezone.is_empty()
            || self.interpreted_timezone != self.event_timezone.as_deref().unwrap_or(timezone)
            || !["none", "confirmed", "tentative", "canceled", "unknown"]
                .contains(&self.status.as_str())
        {
            return Err(CalendarError::InvalidValue);
        }
        Ok(())
    }

    /// Every text it carries is within its limit.
    fn validate_lengths(&self) -> Result<(), CalendarError> {
        if self.title.len() > MAX_TITLE_BYTES
            || self.interpreted_timezone.len() > MAX_TIMEZONE_BYTES
            || self
                .event_timezone
                .as_ref()
                .is_some_and(|zone| zone.len() > MAX_TIMEZONE_BYTES)
            || self
                .event_url
                .as_ref()
                .is_some_and(|url| url.len() > MAX_URL_BYTES)
            || self
                .location
                .as_ref()
                .is_some_and(|location| location.len() > MAX_TITLE_BYTES)
            || self
                .notes
                .as_ref()
                .is_some_and(|notes| notes.len() > MAX_NOTES_BYTES)
        {
            return Err(CalendarError::LimitExceeded);
        }
        Ok(())
    }
}

fn optional(
    authorized: bool,
    status: CalendarFieldStatus,
    present: bool,
) -> Result<(), CalendarError> {
    if !authorized && (status != CalendarFieldStatus::NotRequested || present) {
        return Err(CalendarError::OptionalDataNotAuthorized);
    }
    if (status == CalendarFieldStatus::Provided) != present
        || authorized && status == CalendarFieldStatus::NotRequested
    {
        return Err(CalendarError::InvalidValue);
    }
    Ok(())
}

impl CalendarInvitee {
    fn validate(&self) -> Result<(), CalendarError> {
        if self
            .name
            .as_ref()
            .is_some_and(|name| name.len() > MAX_TITLE_BYTES)
            || self.participant_url.len() > MAX_URL_BYTES
        {
            return Err(CalendarError::LimitExceeded);
        }
        if self.participant_url.is_empty()
            || ascii_control(&self.participant_url)
            || ![
                "unknown",
                "required",
                "optional",
                "chair",
                "non_participant",
            ]
            .contains(&self.role.as_str())
            || ![
                "unknown",
                "pending",
                "accepted",
                "declined",
                "tentative",
                "delegated",
                "completed",
                "in_process",
            ]
            .contains(&self.response_status.as_str())
            || !["unknown", "person", "room", "resource", "group"]
                .contains(&self.participant_type.as_str())
        {
            return Err(CalendarError::InvalidValue);
        }
        // Missing email stays missing. A supplied candidate must be exactly the
        // single decoded mailto address, not a guessed identity from another URI.
        if let Some(email) = &self.email
            && super::mailbox::candidate(&self.participant_url).as_deref() != Some(email)
        {
            return Err(CalendarError::InvalidValue);
        }
        Ok(())
    }
}
