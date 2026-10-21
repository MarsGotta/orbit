use std::collections::HashSet;

use super::*;

pub(super) fn identifier(value: &str) -> bool {
    !value.is_empty() && value.len() <= MAX_LABEL_BYTES && !ascii_control(value)
}

pub(super) fn ascii_control(value: &str) -> bool {
    value.bytes().any(|byte| byte < 32 || byte == 127)
}

/// One native call's interval: its own clock domain, and shorter than a read may take.
pub(super) fn interval(domain: &str, started: u64, completed: u64) -> Result<(), CalendarError> {
    if domain != CLOCK_DOMAIN
        || completed
            .checked_sub(started)
            .is_none_or(|duration| duration >= READ_LIFETIME_NS)
    {
        return Err(CalendarError::InvalidClock);
    }
    Ok(())
}

/// A read joined from several native calls: the same clock domain and time
/// that does not run backwards. Each page was already held to `interval`; the
/// whole read may take longer than one of them.
pub(super) fn joined_interval(
    domain: &str,
    started: u64,
    completed: u64,
) -> Result<(), CalendarError> {
    if domain != CLOCK_DOMAIN || completed < started {
        return Err(CalendarError::InvalidClock);
    }
    Ok(())
}

impl CalendarStatus {
    pub fn validate(&self) -> Result<(), CalendarError> {
        // A physical SDK read can remain pending after delivery was revoked.
        // It is not proof that any data was delivered. P9-21: nothing says
        // «read only» any more; the helper writes too.
        wire::size(self)
    }
}

/// `#RRGGBB`, the only color form the helper sends.
fn hex_color(value: &str) -> bool {
    value.len() == 7
        && value.starts_with('#')
        && value.bytes().skip(1).all(|byte| byte.is_ascii_hexdigit())
}

impl CalendarReference {
    /// The same identifier shape applies to a saved page cursor after its row disappears.
    pub fn valid_id(value: &str) -> bool {
        identifier(value)
    }

    pub fn validate(&self) -> Result<(), CalendarError> {
        if !identifier(&self.calendar_id) || !identifier(&self.source_id) {
            return Err(CalendarError::InvalidValue);
        }
        Ok(())
    }
}

impl CalendarEntry {
    pub fn validate(&self) -> Result<(), CalendarError> {
        self.reference.validate()?;
        if self.title.len() > MAX_TITLE_BYTES || self.source_title.len() > MAX_TITLE_BYTES {
            return Err(CalendarError::LimitExceeded);
        }
        if ![
            "local",
            "caldav",
            "exchange",
            "subscription",
            "birthday",
            "unknown",
        ]
        .contains(&self.calendar_type.as_str())
            || self.color.as_deref().is_some_and(|color| !hex_color(color))
        {
            return Err(CalendarError::InvalidValue);
        }
        Ok(())
    }
}

impl CalendarScope {
    /// Structure only. An empty list is a read whose window held no events: it
    /// covered every calendar and found nothing, which is an answer.
    pub fn validate(&self) -> Result<(), CalendarError> {
        if !crate::wire::canonical_uuid(&self.scope_id)
            || !crate::wire::canonical_uuid(&self.native_instance_id)
        {
            return Err(CalendarError::InvalidValue);
        }
        let mut ids = HashSet::new();
        for reference in &self.calendars {
            reference.validate()?;
            if !ids.insert(&reference.calendar_id) {
                return Err(CalendarError::InvalidValue);
            }
        }
        Ok(())
    }
}

impl CalendarQuery {
    /// Any positive window of representable dates. The width is not bounded
    /// here: a read is chunked into windows the helper accepts before it is
    /// asked for, so a wide window is never a reason to refuse or trim.
    pub fn validate(&self) -> Result<(), CalendarError> {
        if !crate::wire::canonical_uuid(&self.scope_id)
            || !crate::wire::unix_ms(self.start_unix_ms)
            || !crate::wire::unix_ms(self.end_unix_ms)
            || self.end_unix_ms <= self.start_unix_ms
        {
            return Err(CalendarError::InvalidValue);
        }
        Ok(())
    }
}

impl CalendarWindow {
    pub fn validate(&self) -> Result<(), CalendarError> {
        if !crate::wire::unix_ms(self.start_unix_ms)
            || !crate::wire::unix_ms(self.end_unix_ms)
            || self.end_unix_ms <= self.start_unix_ms
        {
            return Err(CalendarError::InvalidValue);
        }
        Ok(())
    }
}

impl CalendarReadQuery {
    /// One window the helper can read whole: never wider than EventKit reads
    /// without shortening it silently (`EVENTKIT_MAX_QUERY_MS`).
    pub fn validate(&self) -> Result<(), CalendarError> {
        CalendarWindow {
            start_unix_ms: self.start_unix_ms,
            end_unix_ms: self.end_unix_ms,
        }
        .validate()?;
        if self.end_unix_ms - self.start_unix_ms > EVENTKIT_MAX_QUERY_MS {
            return Err(CalendarError::InvalidValue);
        }
        Ok(())
    }
}
