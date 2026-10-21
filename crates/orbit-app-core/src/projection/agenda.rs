use super::*;

pub(crate) fn project_calendar_occurrence(event: &CalendarOccurrence) -> CalendarEventSnapshot {
    let invitees = event.invitees.as_deref().unwrap_or_default();
    let attendees = invitees
        .iter()
        .map(|invitee| CalendarAttendeeSnapshot {
            name: invitee
                .name
                .clone()
                .or_else(|| invitee.email.clone())
                .unwrap_or_default(),
            email: invitee.email.clone(),
            response: Some(calendar_response(&invitee.response_status)),
            is_self: invitee.is_current_user,
        })
        .collect();
    // The user's answer is the one on the guest EventKit marks as the user.
    // No such guest means the event is not an invitation to them.
    let self_response = invitees
        .iter()
        .find(|invitee| invitee.is_current_user)
        .map(|invitee| calendar_response(&invitee.response_status));
    CalendarEventSnapshot {
        recording: None,
        id: event.observation_id.clone(),
        calendar_id: Some(event.calendar.calendar_id.clone()),
        title: event.title.clone(),
        start_ms: event.start_unix_ms,
        end_ms: event.end_unix_ms,
        all_day: Some(event.is_all_day),
        attendees: Some(attendees),
        url: event.event_url.clone(),
        location: event.location.clone(),
        status: Some(project_event_status(&event.status)),
        self_response,
        conference_url: event.event_url.as_deref().and_then(conference_url),
        // What a change or a delete names, and what the edit shows.
        calendar_item_id: Some(event.calendar_item_id.clone()),
        occurrence_start_ms: Some(
            event
                .original_occurrence_unix_ms
                .unwrap_or(event.start_unix_ms),
        ),
        notes: event.notes.clone(),
        notes_left_out_bytes: event.notes_left_out.map(|left_out| left_out.bytes),
        organizer: event
            .organizer
            .as_ref()
            .map(|organizer| CalendarAttendeeSnapshot {
                name: organizer
                    .name
                    .clone()
                    .or_else(|| organizer.email.clone())
                    .unwrap_or_default(),
                email: organizer.email.clone(),
                response: Some(calendar_response(&organizer.response_status)),
                is_self: organizer.is_current_user,
            }),
        availability: event.availability,
        recurrence: event.recurrence.clone(),
        can_edit: event.can_edit,
        alarms_minutes: event.alarms_minutes.clone(),
        alarms_not_editable: event.alarms_not_editable,
    }
}

/// EventKit's participant status, in the words of the calendar card.
/// The helper's vocabulary is closed (`orbit_core::calendar` validates it);
/// anything that is not an answer is `None`, never a refusal.
pub(crate) fn calendar_response(status: &str) -> CalendarResponse {
    match status {
        "accepted" => CalendarResponse::Accepted,
        "tentative" => CalendarResponse::Tentative,
        "declined" => CalendarResponse::Declined,
        "pending" => CalendarResponse::NeedsAction,
        _ => CalendarResponse::None,
    }
}

/// `url` when it is an `https` link to a known video call, else nothing.
/// Which links count lives in `orbit_core::calendar`, shared with
/// the choice of a call's event.
pub(crate) fn conference_url(url: &str) -> Option<String> {
    orbit_core::calendar::conference_platform(url).map(|_| url.trim().to_owned())
}

pub(super) fn project_event_status(status: &str) -> CalendarEventStatus {
    match status {
        "tentative" => CalendarEventStatus::Tentative,
        "cancelled" | "canceled" => CalendarEventStatus::Cancelled,
        _ => CalendarEventStatus::Confirmed,
    }
}

/// Can Deimos read the user's agenda right now, and if not, which condition is
/// missing.
///
/// The live read crosses the device bridge, which admits `calendar_read` on the
/// macOS permission alone, and needs the pairing and a reachable Mac to get
/// there. Those three are what this answers, in the order the user can act on
/// them, nearest first. There is no fourth, «calendarios elegidos»: nothing is
/// chosen, and the stored copy this was once confused with no longer exists.
///
/// The permission is three-valued, read from the calendar view when
/// it is on screen and otherwise from the helper's own status, which no other
/// screen evicts. Unknown — the helper has not said yet — is `Unknown`: never
/// `Available`, and never the refusal `CalendarPermission` either.
pub(crate) fn stable_calendar_color(id: &str) -> String {
    const COLORS: [&str; 4] = ["accent", "blue", "orange", "green"];
    let hash = id.bytes().fold(2_166_136_261_u32, |value, byte| {
        value.wrapping_mul(16_777_619) ^ u32::from(byte)
    });
    COLORS[(hash as usize) % COLORS.len()].into()
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests;
