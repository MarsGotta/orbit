//! The agenda read from the calendar's last complete snapshot: its calendars
//! and one window's events, page by page.
use super::*;

pub(super) fn agenda_calendars(
    runtime: &Runtime,
    cursor: Option<Cursor>,
    revision: u64,
) -> Knowledge<Page<serde_json::Value>> {
    if let Some(refusal) = runtime.calendar.inventory_refusal() {
        return Knowledge::Absent { refusal };
    }
    match runtime.calendar_access() {
        Knowledge::Unknown { why } => return Knowledge::Unknown { why },
        Knowledge::Absent { refusal } => return Knowledge::Absent { refusal },
        Knowledge::Present { .. } => {}
    }
    if !runtime.calendar.inventory_valid {
        return Knowledge::Unknown {
            why: Pending::NotObservedYet,
        };
    }
    let Some(calendars) = runtime.calendar.view.calendars.as_ref() else {
        return Knowledge::Unknown {
            why: Pending::NotObservedYet,
        };
    };
    let mut sorted: Vec<_> = calendars.iter().collect();
    sorted.sort_by(|left, right| left.reference.calendar_id.cmp(&right.reference.calendar_id));
    let start = match cursor_value(cursor.as_ref(), |cursor| {
        if let Cursor::Id { id } = cursor {
            orbit_core::calendar::CalendarReference::valid_id(id)
                .then(|| sorted.partition_point(|entry| entry.reference.calendar_id <= *id))
        } else {
            None
        }
    }) {
        Ok(start) => start.unwrap_or(0),
        Err(refusal) => return Knowledge::Absent { refusal },
    };
    let total = sorted.len();
    paginate(
        sorted.iter().enumerate().skip(start).map(|(index, entry)| {
            let next = (index + 1 < total).then(|| Cursor::Id {
                id: entry.reference.calendar_id.clone(),
            });
            (entry, next)
        }),
        total as u64,
        revision,
        None,
        ReadId::AgendaCalendars,
    )
}

pub(super) fn agenda_range(
    runtime: &Runtime,
    start_ms: i64,
    end_ms: i64,
    cursor: Option<Cursor>,
    revision: u64,
) -> Knowledge<Page<serde_json::Value>> {
    let snapshot = match agenda_snapshot(runtime, start_ms, end_ms) {
        Ok(snapshot) => snapshot,
        Err(answer) => return answer,
    };
    let after = match cursor_value(cursor.as_ref(), |cursor| {
        if let Cursor::StartAndId { start_ms, id } = cursor {
            Some((*start_ms, id.clone()))
        } else {
            None
        }
    }) {
        Ok(after) => after,
        Err(refusal) => return Knowledge::Absent { refusal },
    };
    let mut events: Vec<_> = snapshot.events.iter().collect();
    events.sort_by(|left, right| {
        (left.start_unix_ms, left.observation_id.as_str())
            .cmp(&(right.start_unix_ms, right.observation_id.as_str()))
    });
    if let Some((start, id)) = &after
        && !events
            .iter()
            .any(|event| event.start_unix_ms == *start && event.observation_id == *id)
    {
        return Knowledge::Absent {
            refusal: Refusal::InvalidParameter {
                parameter: Parameter::Identifier,
                expected: Expected::ValidIdentifier,
            },
        };
    }
    let total = u64::try_from(events.len()).unwrap_or(u64::MAX);
    let pending: Vec<_> = events
        .into_iter()
        .filter(|event| {
            after.as_ref().is_none_or(|(start, id)| {
                (event.start_unix_ms, event.observation_id.as_str()) > (*start, id.as_str())
            })
        })
        .collect();
    paginate(
        pending.iter().enumerate().map(|(index, event)| {
            let next = (index + 1 < pending.len()).then(|| Cursor::StartAndId {
                start_ms: event.start_unix_ms,
                id: event.observation_id.clone(),
            });
            (event, next)
        }),
        total,
        revision,
        Some(snapshot.observed_at_unix_ms),
        ReadId::AgendaRange,
    )
}

/// The complete snapshot of exactly this window, or what the read
/// answers instead: an invalid range, the window's read failure, no access
/// to the calendar, or a window not read yet.
pub(super) fn agenda_snapshot(
    runtime: &Runtime,
    start_ms: i64,
    end_ms: i64,
) -> Result<&orbit_core::calendar::CalendarSnapshot, Knowledge<Page<serde_json::Value>>> {
    if (CalendarWindow {
        start_unix_ms: start_ms,
        end_unix_ms: end_ms,
    })
    .validate()
    .is_err()
    {
        return Err(Knowledge::Absent {
            refusal: Refusal::InvalidParameter {
                parameter: Parameter::Request,
                expected: Expected::CalendarRange {
                    start_ms,
                    end_ms,
                    min_ms: MIN_UNIX_MS,
                    max_ms: MAX_UNIX_MS,
                },
            },
        });
    }
    if runtime
        .calendar
        .topic_window()
        .as_ref()
        .is_none_or(|range| range.start_unix_ms == start_ms && range.end_unix_ms == end_ms)
        && let Some(refusal) = &runtime.calendar.read_failure
    {
        return Err(Knowledge::Absent {
            refusal: refusal.clone(),
        });
    }
    match runtime.calendar_access() {
        Knowledge::Unknown { why } => return Err(Knowledge::Unknown { why }),
        Knowledge::Absent { refusal } => return Err(Knowledge::Absent { refusal }),
        Knowledge::Present { .. } => {}
    }
    let Some(snapshot) = runtime.calendar.complete.as_deref() else {
        return Err(Knowledge::Unknown {
            why: Pending::NotObservedYet,
        });
    };
    if snapshot.window.start_unix_ms != start_ms || snapshot.window.end_unix_ms != end_ms {
        return Err(Knowledge::Unknown {
            why: Pending::NotObservedYet,
        });
    }
    Ok(snapshot)
}
