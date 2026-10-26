use crate::{
    features::FeatureInput,
    projection::test_support::{calendar_occurrence, calendar_reference},
    topics::TopicCache,
};
use orbit_core::{
    calendar::{CalendarEntry, CalendarWindow},
    reads::ReadRequest,
    topics::{Knowledge, Page, TopicData, TopicFrame, TopicId, agenda::Value},
};

const START: i64 = 1_789_257_600_000;
const END: i64 = 1_789_862_400_000;

fn fixture(reverse: bool, with_pages: bool) -> TopicCache {
    let mut topics = TopicCache::default();
    topics.observe_at(
        TopicFrame {
            epoch: "agenda-fixture".into(),
            revision: 1,
            data: TopicData::Agenda(Knowledge::Present {
                value: Value {
                    invalidation: 1,
                    calendars: Some(2),
                    read_window: Some(CalendarWindow {
                        start_unix_ms: START,
                        end_unix_ms: END,
                    }),
                    read_id: Some("read-1".into()),
                    timezone: Some("Europe/Madrid".into()),
                    writes: Vec::new(),
                },
                as_of_ms: None,
            }),
        },
        Some(1),
    );
    if !with_pages {
        return topics;
    }
    let mut calendars = vec![
        CalendarEntry {
            reference: calendar_reference("work"),
            title: "Trabajo".into(),
            source_title: "Fixture".into(),
            calendar_type: "caldav".into(),
            color: None,
            can_edit: None,
            is_default: None,
        },
        CalendarEntry {
            reference: calendar_reference("team"),
            title: "Equipo".into(),
            source_title: "Fixture".into(),
            calendar_type: "caldav".into(),
            color: None,
            can_edit: None,
            is_default: None,
        },
    ];
    if reverse {
        calendars.reverse();
    }
    assert!(
        topics.remember_read(
            &ReadRequest::AgendaCalendars { cursor: None },
            TopicId::Agenda,
            Knowledge::Present {
                value: Page {
                    items: calendars
                        .into_iter()
                        .map(|item| serde_json::to_value(item).expect("calendar row"))
                        .collect(),
                    next: None,
                    total: 2,
                    revision: 1,
                    store_version: None,
                    handed_over_from: None,
                },
                as_of_ms: None
            },
        )
    );
    let mut events = vec![
        calendar_occurrence(
            "11111111-1111-4111-8111-111111111111",
            "work",
            "Evento enlazado",
            1_789_469_100_000,
            1_789_470_000_000,
        ),
        calendar_occurrence(
            "22222222-2222-4222-8222-222222222222",
            "team",
            "Weekly Sync",
            1_789_469_400_000,
            1_789_471_200_000,
        ),
    ];
    if reverse {
        events.reverse();
    }
    let rows = events
        .into_iter()
        .map(|event| serde_json::to_value(event).expect("event row"))
        .collect();
    assert!(topics.remember_read(
        &ReadRequest::AgendaRange {
            start_ms: START,
            end_ms: END,
            cursor: None
        },
        TopicId::Agenda,
        Knowledge::Present {
            value: Page {
                items: rows,
                next: None,
                total: 2,
                revision: 1,
                store_version: None,
                handed_over_from: None,
            },
            as_of_ms: None
        },
    ));
    topics
}

#[test]
fn assigns_calendar_colors_stably_from_the_identifier() {
    let snapshot = crate::AppSnapshot::default();
    let first_topics = fixture(false, true);
    let second_topics = fixture(true, true);
    let first = crate::features::agenda::project(&FeatureInput::new(&snapshot, &first_topics));
    let second = crate::features::agenda::project(&FeatureInput::new(&snapshot, &second_topics));
    for calendar in &first.calendars {
        let same = second
            .calendars
            .iter()
            .find(|candidate| candidate.id == calendar.id)
            .expect("same calendar");
        assert_eq!(calendar.color, same.color);
        assert!(["accent", "blue", "orange", "green"].contains(&calendar.color.as_str()));
    }
}

#[test]
fn projects_daemon_timezone_and_leaves_webview_fallback_explicit() {
    let snapshot = crate::AppSnapshot::default();
    let topics = fixture(false, false);
    let with_head = crate::features::agenda::project(&FeatureInput::new(&snapshot, &topics));
    let without_head =
        crate::features::agenda::project(&FeatureInput::new(&snapshot, &TopicCache::default()));
    assert_eq!(with_head.window.timezone, "Europe/Madrid");
    assert!(without_head.window.timezone.is_empty());
}

// ------------------------------------------------ answers and video calls

#[test]
fn known_video_calls_are_offered_to_join_and_nothing_else_is() {
    use crate::projection::conference_url;
    for joinable in [
        "https://meet.google.com/abc-defg-hij",
        "https://zoom.us/j/123456789?pwd=x",
        "https://us02web.zoom.us/j/123456789",
        "https://acme.zoom.com/j/1",
        "https://teams.microsoft.com/l/meetup-join/19%3ameeting",
        "https://teams.live.com/meet/9876",
        "https://acme.webex.com/meet/team-room",
        "https://whereby.com/orbit",
        "https://facetime.apple.com/join#v=1&p=abc",
        "HTTPS://Meet.Google.com/abc-defg-hij",
        "  https://meet.google.com/abc-defg-hij  ",
        "https://meet.google.com:443/abc",
    ] {
        assert_eq!(
            conference_url(joinable).as_deref(),
            Some(joinable.trim()),
            "{joinable}"
        );
    }
    for other in [
        "http://meet.google.com/abc-defg-hij",
        "https://meet.google.com.example.com/abc",
        "https://meet.google.com@example.com/abc",
        "https://notzoom.us/j/1",
        "https://example.com/?u=https://zoom.us/j/1",
        "https://docs.google.com/document/d/1",
        "zoommtg://zoom.us/join?confno=1",
        "meet.google.com/abc-defg-hij",
        "https://",
        "",
    ] {
        assert_eq!(conference_url(other), None, "{other}");
    }
}

#[test]
fn the_users_answer_is_the_one_on_the_guest_eventkit_marks_as_the_user() {
    use crate::{CalendarResponse, projection::test_support::briefing_event};
    let mut event = briefing_event(
        "daily",
        "Daily",
        START,
        START + 900_000,
        &["bruno.soler", "alina.perez"],
        Some("https://meet.google.com/abc-defg-hij"),
    );
    for (written, expected) in [
        ("accepted", CalendarResponse::Accepted),
        ("tentative", CalendarResponse::Tentative),
        ("declined", CalendarResponse::Declined),
        ("pending", CalendarResponse::NeedsAction),
        ("unknown", CalendarResponse::None),
        ("delegated", CalendarResponse::None),
    ] {
        if let Some(guests) = event.invitees.as_mut() {
            guests[1].response_status = written.into();
        }
        let projected = crate::projection::project_calendar_occurrence(&event);
        assert_eq!(projected.self_response, Some(expected), "{written}");
        let guests = projected.attendees.unwrap_or_default();
        assert_eq!(guests[0].response, Some(CalendarResponse::Accepted));
        assert_eq!(guests[1].response, Some(expected));
        // The guest the calendar marks as the user travels as the user.
        assert!(!guests[0].is_self);
        assert!(guests[1].is_self);
        assert_eq!(
            projected.conference_url.as_deref(),
            Some("https://meet.google.com/abc-defg-hij")
        );
    }
    // Not on the list, or no list at all: not an invitation to the user.
    let not_theirs = briefing_event("other", "Otra", START, START + 1, &["bruno.soler"], None);
    let projected = crate::projection::project_calendar_occurrence(&not_theirs);
    assert_eq!(projected.self_response, None);
    assert_eq!(projected.conference_url, None);
    let alone = calendar_occurrence("alone", "work", "Foco", START, START + 1);
    assert_eq!(
        crate::projection::project_calendar_occurrence(&alone).self_response,
        None
    );
}

/// The reminders the Mac read reach the event panel as they were
/// read; the ones Orbit does not edit, by number; and absent stays absent.
#[test]
fn reminders_reach_the_event_as_read() {
    let mut event = calendar_occurrence("alarms", "work", "Revisión", 0, 3_600_000);
    let unread = crate::projection::project_calendar_occurrence(&event);
    assert_eq!(
        (unread.alarms_minutes, unread.alarms_not_editable),
        (None, None)
    );
    event.alarms_minutes = Some(vec![0, 15]);
    event.alarms_not_editable = Some(2);
    let read = crate::projection::project_calendar_occurrence(&event);
    assert_eq!(read.alarms_minutes, Some(vec![0, 15]));
    assert_eq!(read.alarms_not_editable, Some(2));
}
