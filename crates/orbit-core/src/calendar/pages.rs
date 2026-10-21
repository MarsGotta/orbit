//! The helper's pages, and the reads the daemon joins them into.
//!
//! REF-10. A read of any range is a sequence of windows (`READ_WINDOW_MS`) and
//! each window a sequence of pages (`NATIVE_PAGE_EVENT_BYTES`). Nothing here
//! decides how many windows or pages a read may take: that is the only way a
//! year, or a day with more events than fit one message, arrives whole.
//!
//! Two rules make the joined read the same read the person would see in
//! Calendar. A window is kept only when every one of its pages came from the
//! same enumeration (same total, same zone, contiguous indexes); if EventKit
//! changed underneath, the pages disagree and the window is read again. And an
//! event that overlaps two windows is returned by both, so it belongs to the
//! window where it starts: every window after the first drops the events that
//! began before it.

use std::collections::HashSet;

use super::{validation::interval, *};

fn index_math(
    after_index: u32,
    items: usize,
    total: u32,
    next: Option<u32>,
) -> Result<(), CalendarError> {
    let end = u64::from(after_index)
        .checked_add(u64::try_from(items).map_err(|_| CalendarError::InvalidValue)?)
        .ok_or(CalendarError::InvalidValue)?;
    let valid = match next {
        // Progress is part of the contract: a page that promises more carries
        // something, so a walk over pages always ends.
        Some(next) => items > 0 && u64::from(next) == end && end < u64::from(total),
        None => end == u64::from(total),
    };
    if !valid {
        return Err(CalendarError::InvalidValue);
    }
    Ok(())
}

impl CalendarReadPage {
    pub fn validate(&self) -> Result<(), CalendarError> {
        self.window.validate()?;
        interval(
            &self.clock_domain,
            self.capture_started_monotonic_ns,
            self.capture_completed_monotonic_ns,
        )?;
        if !crate::wire::canonical_uuid(&self.read_id)
            || !crate::wire::canonical_uuid(&self.native_instance_id)
            || !crate::wire::unix_ms(self.observed_at_unix_ms)
            || self.system_timezone.is_empty()
            || self.system_timezone.len() > 128
            || self.provider_sync_status != "unknown"
            || self.enumeration != "bounded_local_read"
        {
            return Err(CalendarError::InvalidValue);
        }
        index_math(
            self.after_index,
            self.events.len(),
            self.total,
            self.next_after_index,
        )?;
        for omission in &self.omitted {
            omission.validate()?;
        }
        for left_out in &self.oversized {
            left_out.validate()?;
        }
        let mut observations = HashSet::new();
        for event in &self.events {
            // Invitees and the event URL are always asked for: the macOS
            // permission is the only consent, so neither may come back as
            // «not requested».
            event.validate(&self.window, &self.system_timezone, true, true)?;
            if !observations.insert(&event.observation_id) {
                return Err(CalendarError::InvalidValue);
            }
        }
        Ok(())
    }

    /// Admit a page for the exact query the actor sent, with its current facts.
    pub fn validate_response(
        &self,
        query: &CalendarReadQuery,
        facts: &CalendarReadFacts<'_>,
    ) -> Result<(), CalendarError> {
        self.validate()?;
        facts.authorize(&self.native_instance_id, self.privacy_generation)?;
        if self.window.start_unix_ms != query.start_unix_ms
            || self.window.end_unix_ms != query.end_unix_ms
            || self.after_index != query.after_index
        {
            return Err(CalendarError::ScopeChanged);
        }
        Ok(())
    }
}

impl CalendarInventoryPage {
    pub fn validate(&self) -> Result<(), CalendarError> {
        if !crate::wire::canonical_uuid(&self.inventory_id)
            || !crate::wire::canonical_uuid(&self.native_instance_id)
        {
            return Err(CalendarError::InvalidValue);
        }
        interval(
            &self.clock_domain,
            self.observed_monotonic_ns,
            self.completed_monotonic_ns,
        )?;
        index_math(
            self.after_index,
            self.calendars.len(),
            self.total,
            self.next_after_index,
        )?;
        let mut ids = HashSet::new();
        for calendar in &self.calendars {
            calendar.validate()?;
            if !ids.insert(&calendar.reference.calendar_id) {
                return Err(CalendarError::InvalidValue);
            }
        }
        Ok(())
    }

    pub fn validate_response(
        &self,
        after_index: u32,
        facts: &CalendarReadFacts<'_>,
    ) -> Result<(), CalendarError> {
        self.validate()?;
        facts.authorize(&self.native_instance_id, self.privacy_generation)?;
        if self.after_index != after_index {
            return Err(CalendarError::ScopeChanged);
        }
        Ok(())
    }
}

/// Every calendar on the Mac, joined from the inventory's pages.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InventoryRead {
    pub calendars: Vec<CalendarEntry>,
    total: u32,
    native_instance_id: String,
    privacy_generation: u64,
}

impl InventoryRead {
    pub fn start(page: CalendarInventoryPage) -> Result<Self, CalendarError> {
        if page.after_index != 0 {
            return Err(CalendarError::InvalidValue);
        }
        Ok(Self {
            calendars: page.calendars,
            total: page.total,
            native_instance_id: page.native_instance_id,
            privacy_generation: page.privacy_generation,
        })
    }

    /// The next page of the same listing. A page from another listing — the
    /// total moved because a calendar was added or removed between pages —
    /// is refused, and the caller lists again from the start.
    pub fn push(&mut self, page: CalendarInventoryPage) -> Result<(), CalendarError> {
        if page.total != self.total
            || page.native_instance_id != self.native_instance_id
            || page.privacy_generation != self.privacy_generation
            || usize::try_from(page.after_index).ok() != Some(self.calendars.len())
        {
            return Err(CalendarError::ScopeChanged);
        }
        for calendar in page.calendars {
            if self
                .calendars
                .iter()
                .any(|known| known.reference.calendar_id == calendar.reference.calendar_id)
            {
                return Err(CalendarError::ScopeChanged);
            }
            self.calendars.push(calendar);
        }
        Ok(())
    }

    pub fn next_after_index(&self) -> Option<u32> {
        let read = u32::try_from(self.calendars.len()).ok()?;
        (read < self.total).then_some(read)
    }
}

/// One window of every calendar, joined from its pages in order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WindowRead {
    pub read_id: String,
    pub native_instance_id: String,
    pub privacy_generation: u64,
    pub window: CalendarWindow,
    pub capture_started_monotonic_ns: u64,
    pub capture_completed_monotonic_ns: u64,
    pub observed_at_unix_ms: i64,
    pub system_timezone: String,
    pub events: Vec<CalendarOccurrence>,
    pub omitted: Vec<CalendarOmission>,
    /// The window's events that could not travel, named (decision 4). Like
    /// `omitted`, the helper repeats them on every page of the window; the
    /// first page's list is the window's.
    pub oversized: Vec<CalendarOversizedEvent>,
    total: u32,
}

impl WindowRead {
    pub fn start(page: CalendarReadPage) -> Result<Self, CalendarError> {
        if page.after_index != 0 {
            return Err(CalendarError::InvalidValue);
        }
        Ok(Self {
            read_id: page.read_id,
            native_instance_id: page.native_instance_id,
            privacy_generation: page.privacy_generation,
            window: page.window,
            capture_started_monotonic_ns: page.capture_started_monotonic_ns,
            capture_completed_monotonic_ns: page.capture_completed_monotonic_ns,
            observed_at_unix_ms: page.observed_at_unix_ms,
            system_timezone: page.system_timezone,
            events: page.events,
            omitted: page.omitted,
            oversized: page.oversized,
            total: page.total,
        })
    }

    /// The next page of the same enumeration. Anything that shows the store
    /// changed between pages — another total, another zone, a gap — refuses
    /// the page, and the caller reads the window again from its first page.
    pub fn push(&mut self, page: CalendarReadPage) -> Result<(), CalendarError> {
        if page.window != self.window
            || page.total != self.total
            || page.system_timezone != self.system_timezone
            || page.native_instance_id != self.native_instance_id
            || page.privacy_generation != self.privacy_generation
            || page.capture_started_monotonic_ns < self.capture_completed_monotonic_ns
            || usize::try_from(page.after_index).ok() != Some(self.events.len())
        {
            return Err(CalendarError::ScopeChanged);
        }
        for event in page.events {
            if self
                .events
                .iter()
                .any(|known| known.observation_id == event.observation_id)
            {
                return Err(CalendarError::InvalidValue);
            }
            self.events.push(event);
        }
        self.capture_completed_monotonic_ns = page.capture_completed_monotonic_ns;
        Ok(())
    }

    pub fn next_after_index(&self) -> Option<u32> {
        let read = u32::try_from(self.events.len()).ok()?;
        (read < self.total).then_some(read)
    }

    /// Drops the events that began before `from_ms`: they overlap an earlier
    /// window of the same read and were already returned there.
    pub fn keep_from(&mut self, from_ms: i64) {
        self.events.retain(|event| event.start_unix_ms >= from_ms);
        self.oversized
            .retain(|event| event.start_unix_ms >= from_ms);
    }
}

/// Where one range is cut into the windows the helper is asked for.
///
/// Pure: the next window after `from_ms`, never wider than `READ_WINDOW_MS`
/// and never past `end_ms`. `None` once the range is covered.
pub fn next_window(from_ms: i64, end_ms: i64) -> Option<CalendarWindow> {
    (from_ms < end_ms).then(|| CalendarWindow {
        start_unix_ms: from_ms,
        end_unix_ms: from_ms.saturating_add(READ_WINDOW_MS).min(end_ms),
    })
}

/// Joins the windows of one read, in order, into the snapshot the app shows.
///
/// `windows` must cover `[start_ms, end_ms)` contiguously. The scope lists the
/// calendars the kept events came from, invitees and URL always read. Events
/// are kept in start order; the keyed read bounds each serialized page.
pub fn assemble_view(windows: Vec<WindowRead>) -> Result<CalendarSnapshot, CalendarError> {
    let mut windows = windows.into_iter();
    let first = windows.next().ok_or(CalendarError::InvalidValue)?;
    let start = first.window.start_unix_ms;
    let mut end = first.window.end_unix_ms;
    let mut completed = first.capture_completed_monotonic_ns;
    let mut events = first.events;
    let mut omitted = first.omitted;
    let mut left_out = u32::try_from(first.oversized.len()).unwrap_or(u32::MAX);
    for window in windows {
        if window.window.start_unix_ms != end
            || window.native_instance_id != first.native_instance_id
            || window.privacy_generation != first.privacy_generation
            || window.system_timezone != first.system_timezone
        {
            return Err(CalendarError::ScopeChanged);
        }
        end = window.window.end_unix_ms;
        completed = completed.max(window.capture_completed_monotonic_ns);
        let from = window.window.start_unix_ms;
        events.extend(
            window
                .events
                .into_iter()
                .filter(|event| event.start_unix_ms >= from),
        );
        for omission in window.omitted {
            match omitted
                .iter_mut()
                .find(|known| known.reason == omission.reason)
            {
                Some(known) => known.count = known.count.saturating_add(omission.count),
                None => omitted.push(omission),
            }
        }
        let later = window
            .oversized
            .iter()
            .filter(|event| event.start_unix_ms >= from)
            .count();
        left_out = left_out.saturating_add(u32::try_from(later).unwrap_or(u32::MAX));
    }
    // Decision 4: what could not travel is counted where the view keeps its
    // omissions; the reads Deimos makes name each one.
    if left_out > 0 {
        omitted.push(CalendarOmission {
            reason: "event_too_large".into(),
            count: left_out,
        });
    }
    events.sort_by(|left, right| {
        (left.start_unix_ms, left.end_unix_ms).cmp(&(right.start_unix_ms, right.end_unix_ms))
    });
    let mut calendars: Vec<CalendarReference> = Vec::new();
    for event in &events {
        if !calendars.contains(&event.calendar) {
            calendars.push(event.calendar.clone());
        }
    }
    let scope = CalendarScope {
        scope_id: first.read_id.clone(),
        native_instance_id: first.native_instance_id,
        privacy_generation: first.privacy_generation,
        calendars,
        include_invitees: true,
        include_event_url: true,
    };
    let snapshot = CalendarSnapshot {
        snapshot_id: first.read_id.clone(),
        window: CalendarQuery {
            scope_id: first.read_id,
            start_unix_ms: start,
            end_unix_ms: end,
        },
        scope,
        clock_domain: CLOCK_DOMAIN.into(),
        capture_started_monotonic_ns: first.capture_started_monotonic_ns,
        capture_completed_monotonic_ns: completed,
        observed_at_unix_ms: first.observed_at_unix_ms,
        system_timezone: first.system_timezone,
        provider_sync_status: "unknown".into(),
        enumeration: "bounded_local_read".into(),
        events,
        omitted,
        unreadable_calendars: Vec::new(),
    };
    snapshot.validate()?;
    Ok(snapshot)
}
