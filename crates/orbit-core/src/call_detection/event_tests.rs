//! P9-11 criterion 2: which event a call belongs to.
use super::*;
use crate::calendar::{
    CalendarFieldStatus, CalendarInvitee, CalendarOccurrence, CalendarReference,
};

const NOW: i64 = 1_800_000_000_000;
const MINUTE: i64 = 60_000;
const ZOOM_APP: &str = "us.zoom.xos";
const CHROME: &str = "com.google.Chrome";

fn guest(name: &str, me: bool, response: &str) -> CalendarInvitee {
    CalendarInvitee {
        name: Some(name.into()),
        participant_url: format!("mailto:{name}@example.com"),
        email: Some(format!("{name}@example.com")),
        role: "required".into(),
        response_status: response.into(),
        participant_type: "person".into(),
        is_current_user: me,
    }
}

fn event(item: &str, start: i64, end: i64) -> CalendarOccurrence {
    CalendarOccurrence {
        observation_id: format!("observation-{item}"),
        calendar: CalendarReference {
            calendar_id: "calendar".into(),
            source_id: "source".into(),
        },
        calendar_item_id: item.into(),
        event_id: None,
        original_occurrence_unix_ms: None,
        start_unix_ms: start,
        end_unix_ms: end,
        event_timezone: None,
        interpreted_timezone: "Europe/Madrid".into(),
        is_floating: false,
        is_all_day: false,
        is_detached: false,
        title: format!("Evento {item}"),
        status: "confirmed".into(),
        last_modified_unix_ms: None,
        invitees: Some(vec![
            guest("mars", true, "accepted"),
            guest("lucia", false, "accepted"),
        ]),
        invitees_status: CalendarFieldStatus::Provided,
        event_url: None,
        event_url_status: CalendarFieldStatus::Provided,
        location: None,
        notes: None,
        notes_left_out: None,
        organizer: None,
        availability: None,
        recurrence: None,
        can_edit: None,
        alarms_minutes: None,
        alarms_not_editable: None,
    }
}

fn with_link(mut occurrence: CalendarOccurrence, url: &str) -> CalendarOccurrence {
    occurrence.event_url = Some(url.into());
    occurrence
}

fn alone(mut occurrence: CalendarOccurrence) -> CalendarOccurrence {
    occurrence.invitees = Some(vec![guest("mars", true, "accepted")]);
    occurrence
}

fn chosen<'a>(events: &'a [CalendarOccurrence], app: &str) -> Option<&'a str> {
    choose_event(events, app, NOW).map(|event| event.calendar_item_id.as_str())
}

#[test]
fn overlapping_events_go_to_the_one_on_the_same_platform() {
    let events = [
        with_link(
            event("meet", NOW - 10 * MINUTE, NOW + 20 * MINUTE),
            "https://meet.google.com/abc-defg-hij",
        ),
        with_link(
            event("zoom", NOW - 5 * MINUTE, NOW + 25 * MINUTE),
            "https://acme.zoom.us/j/42",
        ),
    ];
    assert_eq!(chosen(&events, ZOOM_APP), Some("zoom"));
    // Meet lives in a browser.
    assert_eq!(chosen(&events, CHROME), Some("meet"));
}

#[test]
fn without_a_platform_match_the_nearest_start_wins() {
    let events = [
        event("long", NOW - 50 * MINUTE, NOW + 10 * MINUTE),
        event("next", NOW + 2 * MINUTE, NOW + 32 * MINUTE),
    ];
    assert_eq!(chosen(&events, ZOOM_APP), Some("next"));
}

#[test]
fn an_event_without_guests_or_a_link_is_not_a_call() {
    let events = [alone(event("focus", NOW - 5 * MINUTE, NOW + 55 * MINUTE))];
    assert_eq!(chosen(&events, ZOOM_APP), None);
    // The same event with a video link is one.
    let events = [with_link(
        alone(event("solo", NOW - 5 * MINUTE, NOW + 55 * MINUTE)),
        "https://acme.zoom.us/j/7",
    )];
    assert_eq!(chosen(&events, ZOOM_APP), Some("solo"));
}

#[test]
fn a_link_of_another_platform_still_counts_when_nothing_matches_better() {
    let events = [with_link(
        event("teams", NOW - 5 * MINUTE, NOW + 25 * MINUTE),
        "https://teams.microsoft.com/l/meetup-join/1",
    )];
    assert_eq!(chosen(&events, ZOOM_APP), Some("teams"));
    let events = [
        with_link(
            event("teams", NOW - 5 * MINUTE, NOW + 25 * MINUTE),
            "https://teams.microsoft.com/l/meetup-join/1",
        ),
        with_link(
            event("zoom", NOW + 4 * MINUTE, NOW + 34 * MINUTE),
            "https://zoom.us/j/1",
        ),
    ];
    assert_eq!(chosen(&events, ZOOM_APP), Some("zoom"));
}

#[test]
fn an_event_starting_in_eight_minutes_is_the_call_and_in_eleven_is_not() {
    let events = [event("soon", NOW + 8 * MINUTE, NOW + 38 * MINUTE)];
    assert_eq!(chosen(&events, ZOOM_APP), Some("soon"));
    let events = [event("later", NOW + 11 * MINUTE, NOW + 41 * MINUTE)];
    assert_eq!(chosen(&events, ZOOM_APP), None);
    let events = [event("edge", NOW + 10 * MINUTE, NOW + 40 * MINUTE)];
    assert_eq!(chosen(&events, ZOOM_APP), Some("edge"));
    let events = [event("over", NOW - 60 * MINUTE, NOW)];
    assert_eq!(
        chosen(&events, ZOOM_APP),
        None,
        "an event that ended is not on"
    );
}

#[test]
fn a_link_in_the_location_or_the_notes_names_the_platform() {
    let mut zoom = event("zoom", NOW - 5 * MINUTE, NOW + 25 * MINUTE);
    zoom.location = Some("https://acme.zoom.us/j/42".into());
    let mut meet = event("meet", NOW - 5 * MINUTE, NOW + 25 * MINUTE);
    meet.notes = Some("Unirse: https://meet.google.com/abc-defg-hij".into());
    let events = [meet, zoom];
    assert_eq!(chosen(&events, ZOOM_APP), Some("zoom"));
    assert_eq!(chosen(&events, CHROME), Some("meet"));
}

#[test]
fn all_day_cancelled_and_declined_events_are_never_the_call() {
    let mut all_day = event("all-day", NOW - 60 * MINUTE, NOW + 600 * MINUTE);
    all_day.is_all_day = true;
    let mut cancelled = event("cancelled", NOW - 5 * MINUTE, NOW + 25 * MINUTE);
    cancelled.status = "cancelled".into();
    let mut declined = event("declined", NOW - 5 * MINUTE, NOW + 25 * MINUTE);
    declined.invitees = Some(vec![
        guest("mars", true, "declined"),
        guest("lucia", false, "accepted"),
    ]);
    assert_eq!(chosen(&[all_day, cancelled, declined], ZOOM_APP), None);
}

#[test]
fn the_link_is_the_stable_reference_never_the_observation() {
    let mut moved = event("series", NOW + MINUTE, NOW + 31 * MINUTE);
    moved.original_occurrence_unix_ms = Some(NOW - 24 * 60 * MINUTE);
    let linked = LinkedEvent::from_occurrence(&moved);
    assert_eq!(linked.calendar_item_id, "series");
    assert_eq!(linked.occurrence_start_ms, NOW - 24 * 60 * MINUTE);
    assert_eq!(linked.starts_at_ms, NOW + MINUTE);
    assert_eq!(linked.title, "Evento series");
    assert_eq!(linked.attendees.len(), 2);
    assert!(linked.attendees.iter().any(|guest| guest.is_self));
    assert_eq!(linked.check(), Ok(()));
}

/// P9-06: whether the event repeats is kept with the link, as the calendar
/// read it when the recording started, so the note never has to find the
/// event again in whatever range the calendar shows later. A link stored
/// before the field existed reads as unknown, never as «single».
#[test]
fn p9_06_the_link_keeps_whether_the_event_repeats() -> Result<(), serde_json::Error> {
    let single = LinkedEvent::from_occurrence(&event("single", NOW, NOW + MINUTE));
    assert_eq!(single.recurring, Some(false));
    let mut weekly = event("weekly", NOW, NOW + MINUTE);
    weekly.recurrence = Some(serde_json::from_value(
        serde_json::json!({"frequency": "weekly", "interval": 1}),
    )?);
    assert_eq!(LinkedEvent::from_occurrence(&weekly).recurring, Some(true));
    let stored_before: LinkedEvent = serde_json::from_value(serde_json::json!({
        "calendar_item_id": "series", "occurrence_start_ms": NOW, "title": "Comité",
        "starts_at_ms": NOW, "ends_at_ms": NOW + MINUTE, "attendees": []
    }))?;
    assert_eq!(stored_before.recurring, None);
    Ok(())
}

#[test]
fn a_linked_event_out_of_shape_is_refused_by_field() {
    let mut linked = LinkedEvent::from_occurrence(&event("x", NOW, NOW + MINUTE));
    linked.calendar_item_id = String::new();
    assert_eq!(
        linked.check(),
        Err(LinkedEventRefusal::Identifier {
            bytes: 0,
            max_bytes: crate::calendar::MAX_LABEL_BYTES
        })
    );
    let mut linked = LinkedEvent::from_occurrence(&event("x", NOW, NOW + MINUTE));
    linked.ends_at_ms = NOW - 1;
    assert_eq!(
        linked.check(),
        Err(LinkedEventRefusal::EndsBeforeStart {
            starts_at_ms: NOW,
            ends_at_ms: NOW - 1
        })
    );
    let mut linked = LinkedEvent::from_occurrence(&event("x", NOW, NOW + MINUTE));
    linked.calendar_item_id = "id\u{7}".into();
    assert!(matches!(
        linked.check(),
        Err(LinkedEventRefusal::Identifier { .. })
    ));
}
