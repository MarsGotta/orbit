use serde_json::{Value, json};

use super::*;

type TestResult = Result<(), Box<dyn std::error::Error>>;
const HOST: &str = "22222222-2222-4222-8222-222222222222";
const INVENTORY: &str = "11111111-1111-4111-8111-111111111111";
const READ: &str = "44444444-4444-4444-8444-444444444444";
const MACH_START: u64 = 9_000_000_000_000_000_000;
const DAY_MS: i64 = 86_400_000;

fn facts() -> CalendarReadFacts<'static> {
    CalendarReadFacts {
        native_instance_id: HOST,
        privacy_generation: 2,
        authorization: CalendarAuthorization::FullAccess,
        privacy_stopped: false,
        elapsed_since_request_ns: Some(100_000),
    }
}

fn reference(index: usize) -> CalendarReference {
    CalendarReference {
        calendar_id: format!("calendar-{index}"),
        source_id: "source-local".into(),
    }
}

fn entry(index: usize) -> CalendarEntry {
    CalendarEntry {
        reference: reference(index),
        title: format!("Agenda sintética {index}"),
        source_title: "Cuenta de prueba".into(),
        calendar_type: "caldav".into(),
        color: None,
        can_edit: None,
        is_default: None,
    }
}

fn inventory(calendars: Vec<CalendarEntry>) -> CalendarInventoryPage {
    let total = u32::try_from(calendars.len()).unwrap_or(u32::MAX);
    CalendarInventoryPage {
        inventory_id: INVENTORY.into(),
        native_instance_id: HOST.into(),
        privacy_generation: 2,
        clock_domain: CLOCK_DOMAIN.into(),
        observed_monotonic_ns: MACH_START,
        completed_monotonic_ns: MACH_START + 100,
        calendars,
        after_index: 0,
        total,
        next_after_index: None,
    }
}

fn event(index: usize) -> CalendarOccurrence {
    CalendarOccurrence {
        observation_id: format!("55555555-5555-4555-8555-{index:012x}"),
        calendar: reference(1),
        calendar_item_id: "local-series-id".into(),
        event_id: None,
        original_occurrence_unix_ms: None,
        start_unix_ms: 1000,
        end_unix_ms: 2000,
        event_timezone: None,
        interpreted_timezone: "Europe/Madrid".into(),
        is_floating: true,
        is_all_day: false,
        is_detached: false,
        title: "Evento sintético".into(),
        status: "confirmed".into(),
        last_modified_unix_ms: None,
        invitees: None,
        invitees_status: CalendarFieldStatus::NotProvided,
        event_url: None,
        event_url_status: CalendarFieldStatus::NotProvided,
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

fn page(events: Vec<CalendarOccurrence>) -> CalendarReadPage {
    let total = u32::try_from(events.len()).unwrap_or(u32::MAX);
    CalendarReadPage {
        read_id: READ.into(),
        native_instance_id: HOST.into(),
        privacy_generation: 2,
        window: CalendarWindow {
            start_unix_ms: 0,
            end_unix_ms: DAY_MS,
        },
        clock_domain: CLOCK_DOMAIN.into(),
        capture_started_monotonic_ns: MACH_START,
        capture_completed_monotonic_ns: MACH_START + 100,
        observed_at_unix_ms: 1_800_000_000_000,
        system_timezone: "Europe/Madrid".into(),
        provider_sync_status: "unknown".into(),
        enumeration: "bounded_local_read".into(),
        events,
        omitted: vec![],
        oversized: vec![],
        after_index: 0,
        total,
        next_after_index: None,
    }
}

fn query() -> CalendarReadQuery {
    CalendarReadQuery {
        start_unix_ms: 0,
        end_unix_ms: DAY_MS,
        after_index: 0,
    }
}

fn invitee(url: &str, email: Option<&str>) -> CalendarInvitee {
    CalendarInvitee {
        name: None,
        participant_url: url.into(),
        email: email.map(String::from),
        role: "required".into(),
        response_status: "accepted".into(),
        participant_type: "person".into(),
        is_current_user: false,
    }
}

// ------------------------------------------------- paged reads of every calendar

/// No constant limits how many calendars are listed. A thousand is
/// a page like any other, and several pages join into one list.
#[test]
fn any_number_of_calendars_is_listed_and_joined() -> TestResult {
    let all: Vec<_> = (0..1_000).map(entry).collect();
    let whole = inventory(all.clone());
    whole.validate_response(0, &facts())?;

    let mut first = inventory(all[..600].to_vec());
    first.total = 1_000;
    first.next_after_index = Some(600);
    let mut second = inventory(all[600..].to_vec());
    second.after_index = 600;
    second.total = 1_000;
    let mut joined = InventoryRead::start(first)?;
    assert_eq!(joined.next_after_index(), Some(600));
    joined.push(second)?;
    assert_eq!(joined.next_after_index(), None);
    assert_eq!(joined.calendars, all);
    Ok(())
}

#[test]
fn a_listing_that_changed_between_pages_is_listed_again() -> TestResult {
    let all: Vec<_> = (0..4).map(entry).collect();
    let mut first = inventory(all[..2].to_vec());
    first.total = 4;
    first.next_after_index = Some(2);
    let mut second = inventory(all[2..].to_vec());
    second.after_index = 2;
    second.total = 5; // a calendar was added between the two pages
    second.next_after_index = Some(4);
    let mut joined = InventoryRead::start(first)?;
    assert_eq!(joined.push(second), Err(CalendarError::ScopeChanged));
    Ok(())
}

/// A window with more events than a page is read in pages, joined
/// in order, and nothing is counted as left out.
#[test]
fn a_full_day_arrives_whole_in_pages() -> TestResult {
    let events: Vec<_> = (0..500).map(event).collect();
    let mut first = page(events[..200].to_vec());
    first.total = 500;
    first.next_after_index = Some(200);
    let mut second = page(events[200..450].to_vec());
    second.after_index = 200;
    second.total = 500;
    second.next_after_index = Some(450);
    second.capture_started_monotonic_ns = MACH_START + 200;
    second.capture_completed_monotonic_ns = MACH_START + 300;
    let mut third = page(events[450..].to_vec());
    third.after_index = 450;
    third.total = 500;
    third.capture_started_monotonic_ns = MACH_START + 400;
    third.capture_completed_monotonic_ns = MACH_START + 500;
    for (item, after) in [(&first, 0), (&second, 200), (&third, 450)] {
        item.validate_response(
            &CalendarReadQuery {
                after_index: after,
                ..query()
            },
            &facts(),
        )?;
    }
    let mut window = WindowRead::start(first)?;
    window.push(second)?;
    assert_eq!(window.next_after_index(), Some(450));
    window.push(third)?;
    assert_eq!(window.next_after_index(), None);
    assert_eq!(window.events, events);
    assert!(window.omitted.is_empty());
    Ok(())
}

/// A page from another enumeration of the same window — EventKit changed in
/// between — never joins the first: the window is read again from its start.
#[test]
fn pages_of_two_different_enumerations_are_never_joined() -> TestResult {
    let mut first = page(vec![event(1), event(2)]);
    first.total = 3;
    first.next_after_index = Some(2);
    let mut changed = page(vec![event(3), event(4)]);
    changed.after_index = 2;
    changed.total = 4;
    let mut window = WindowRead::start(first.clone())?;
    assert_eq!(window.push(changed), Err(CalendarError::ScopeChanged));

    let mut gap = page(vec![event(3)]);
    gap.after_index = 1;
    gap.total = 3;
    let mut window = WindowRead::start(first.clone())?;
    assert_eq!(window.push(gap), Err(CalendarError::ScopeChanged));

    let mut zone = page(vec![event(3)]);
    zone.after_index = 2;
    zone.total = 3;
    zone.system_timezone = "UTC".into();
    zone.events[0].interpreted_timezone = "UTC".into();
    let mut window = WindowRead::start(first)?;
    assert_eq!(window.push(zone), Err(CalendarError::ScopeChanged));
    Ok(())
}

#[test]
fn page_indexes_must_add_up_and_a_continuing_page_carries_something() {
    let mut value = page(vec![event(1)]);
    value.total = 2;
    assert_eq!(value.validate(), Err(CalendarError::InvalidValue));
    value.next_after_index = Some(1);
    assert_eq!(value.validate(), Ok(()));
    value.next_after_index = Some(2);
    assert_eq!(value.validate(), Err(CalendarError::InvalidValue));
    let mut empty = page(vec![]);
    empty.total = 1;
    empty.next_after_index = Some(0);
    assert_eq!(empty.validate(), Err(CalendarError::InvalidValue));
    let empty_window = page(vec![]);
    assert_eq!(
        empty_window.validate(),
        Ok(()),
        "an empty window is an answer"
    );
}

/// Any range becomes windows the helper reads whole, none wider
/// than `READ_WINDOW_MS`, together covering it exactly.
#[test]
fn any_range_is_cut_into_windows_that_cover_it_exactly() {
    let start = 1_767_225_600_000; // 2026-01-01
    let end = start + 365 * DAY_MS;
    let mut windows = Vec::new();
    let mut from = start;
    while let Some(window) = next_window(from, end) {
        assert!(window.end_unix_ms - window.start_unix_ms <= READ_WINDOW_MS);
        assert!(window.end_unix_ms - window.start_unix_ms <= EVENTKIT_MAX_QUERY_MS);
        from = window.end_unix_ms;
        windows.push(window);
    }
    assert_eq!(windows.len(), 12);
    assert_eq!(windows[0].start_unix_ms, start);
    assert_eq!(windows[11].end_unix_ms, end);
    assert!(
        windows
            .windows(2)
            .all(|pair| pair[0].end_unix_ms == pair[1].start_unix_ms)
    );
    assert_eq!(next_window(end, end), None);
}

#[test]
fn a_helper_query_never_asks_eventkit_for_more_than_it_reads_whole() {
    let mut value = query();
    value.end_unix_ms = EVENTKIT_MAX_QUERY_MS;
    assert_eq!(value.validate(), Ok(()));
    value.end_unix_ms = EVENTKIT_MAX_QUERY_MS + 1;
    assert_eq!(value.validate(), Err(CalendarError::InvalidValue));
    value.end_unix_ms = 0;
    assert_eq!(value.validate(), Err(CalendarError::InvalidValue));
}

/// An event that overlaps two windows is returned by both and kept once: in
/// the window where it starts.
#[test]
fn an_event_across_two_windows_is_kept_once() -> TestResult {
    let mut first = page(vec![event(1)]);
    first.events[0].start_unix_ms = DAY_MS - 1_000;
    first.events[0].end_unix_ms = DAY_MS + 1_000;
    let mut second = page(vec![event(1), event(2)]);
    second.window = CalendarWindow {
        start_unix_ms: DAY_MS,
        end_unix_ms: 2 * DAY_MS,
    };
    second.read_id = "44444444-4444-4444-8444-000000000002".into();
    second.events[0].start_unix_ms = DAY_MS - 1_000;
    second.events[0].end_unix_ms = DAY_MS + 1_000;
    second.events[1].start_unix_ms = DAY_MS;
    second.events[1].end_unix_ms = DAY_MS;
    let view = assemble_view(vec![WindowRead::start(first)?, WindowRead::start(second)?])?;
    assert_eq!(view.events.len(), 2);
    assert_eq!(view.window.start_unix_ms, 0);
    assert_eq!(view.window.end_unix_ms, 2 * DAY_MS);
    Ok(())
}

/// The agenda view reads every calendar and says which calendars its
/// events came from; invitees and URL are always read.
#[test]
fn the_assembled_view_covers_every_calendar_it_found() -> TestResult {
    let mut events: Vec<_> = (0..6).map(event).collect();
    for (index, item) in events.iter_mut().enumerate() {
        item.calendar = reference(index % 3);
    }
    let view = assemble_view(vec![WindowRead::start(page(events))?])?;
    assert_eq!(
        view.scope.calendars,
        vec![reference(0), reference(1), reference(2)]
    );
    assert!(view.scope.include_invitees && view.scope.include_event_url);
    view.validate()?;
    // An empty window is still a read, with no calendars to name.
    let empty = assemble_view(vec![WindowRead::start(page(vec![]))?])?;
    assert!(empty.scope.calendars.is_empty());
    empty.validate()?;
    Ok(())
}

/// A meeting with more guests than the old ceiling of 64 is one valid event.
#[test]
fn a_meeting_with_hundreds_of_invitees_is_one_valid_event() -> TestResult {
    let mut value = page(vec![event(1)]);
    value.events[0].invitees_status = CalendarFieldStatus::Provided;
    value.events[0].invitees = Some(vec![invitee("urn:test", None); 300]);
    value.validate()?;
    Ok(())
}

// ------------------------------------------------ events, as before the paging

#[test]
fn clocks_have_unrelated_origins_and_wall_time_does_not_refresh_a_response() -> TestResult {
    let value = page(vec![event(1)]);
    value.validate_response(&query(), &facts())?;
    let mut late = facts();
    late.elapsed_since_request_ns = Some(READ_LIFETIME_NS);
    assert_eq!(
        value.validate_response(&query(), &late),
        Err(CalendarError::Expired)
    );
    late.elapsed_since_request_ns = None;
    assert_eq!(
        value.validate_response(&query(), &late),
        Err(CalendarError::ClockUnavailable)
    );
    let mut slow = value.clone();
    slow.capture_completed_monotonic_ns = MACH_START + READ_LIFETIME_NS;
    assert_eq!(slow.validate(), Err(CalendarError::InvalidClock));
    slow.capture_completed_monotonic_ns = MACH_START - 1;
    assert_eq!(slow.validate(), Err(CalendarError::InvalidClock));
    Ok(())
}

#[test]
fn another_host_generation_or_query_is_never_adopted() {
    let value = page(vec![event(1)]);
    let mut other = facts();
    other.privacy_generation = 3;
    assert_eq!(
        value.validate_response(&query(), &other),
        Err(CalendarError::ScopeChanged)
    );
    let mut asked = query();
    asked.after_index = 1;
    assert_eq!(
        value.validate_response(&asked, &facts()),
        Err(CalendarError::ScopeChanged)
    );
    asked = query();
    asked.end_unix_ms += 1;
    assert_eq!(
        value.validate_response(&asked, &facts()),
        Err(CalendarError::ScopeChanged)
    );
}

#[test]
fn all_non_read_permissions_and_privacy_stop_deny_delivery() {
    let value = page(vec![event(1)]);
    for authorization in [
        CalendarAuthorization::NotDetermined,
        CalendarAuthorization::Restricted,
        CalendarAuthorization::Denied,
        CalendarAuthorization::WriteOnly,
        CalendarAuthorization::Unavailable,
        CalendarAuthorization::Unknown,
    ] {
        let mut denied = facts();
        denied.authorization = authorization;
        assert_eq!(
            value.validate_response(&query(), &denied),
            Err(CalendarError::PermissionRequired),
            "{authorization:?}"
        );
    }
    let mut stopped = facts();
    stopped.privacy_stopped = true;
    assert_eq!(
        value.validate_response(&query(), &stopped),
        Err(CalendarError::PrivacyStopped)
    );
}

#[test]
fn mailto_candidates_preserve_case_unicode_and_do_not_become_people() -> TestResult {
    let mut value = page(vec![event(1)]);
    value.events[0].invitees_status = CalendarFieldStatus::Provided;
    value.events[0].invitees = Some(vec![
        invitee(
            "MAILTO:Prueba%2B%C3%B1@example.test",
            Some("Prueba+ñ@example.test"),
        ),
        invitee("urn:uuid:66666666-6666-4666-8666-666666666666", None),
        invitee("https://example.test/participant", None),
    ]);
    value.validate()?;
    assert_eq!(decode_read_page(&serde_json::to_vec(&value)?)?, value);
    for uri in [
        "mailto:a%ZZ@example.test",
        "mailto:a@example.test?subject=x",
        "mailto:a@example.test#x",
        "mailto:a@example.test,b@example.test",
        "mailto:a%0A@example.test",
        "mailto:a%FF@example.test",
        "mailto://a@example.test",
        "https://example.test/a@example.test",
    ] {
        value.events[0].invitees = Some(vec![invitee(uri, Some("a@example.test"))]);
        assert_eq!(value.validate(), Err(CalendarError::InvalidValue), "{uri}");
    }
    Ok(())
}

#[test]
fn a_page_over_the_native_limit_is_refused_before_it_is_parsed() {
    assert_eq!(
        decode_read_page(&vec![b' '; MAX_NATIVE_PAGE_BYTES + 1]),
        Err(CalendarError::LimitExceeded)
    );
    let mut value = page(vec![event(1)]);
    value.omitted = vec![CalendarOmission {
        reason: "missing_title".into(),
        count: 0,
    }];
    assert_eq!(value.validate(), Err(CalendarError::InvalidValue));
    value.omitted[0].count = 1;
    value.omitted[0].reason = "x".repeat(65);
    assert_eq!(value.validate(), Err(CalendarError::InvalidValue));
    value.omitted[0].reason = "missing\ntitle".into();
    assert_eq!(value.validate(), Err(CalendarError::InvalidValue));
}

#[test]
fn no_controls_in_identifiers() {
    for bad in ["", "calendar\n", "calendar\u{0}", "calendar\u{7f}"] {
        let mut reference = reference(1);
        reference.calendar_id = bad.into();
        assert_eq!(reference.validate(), Err(CalendarError::InvalidValue));
    }
}

#[test]
fn data_unicode_and_control_characters_are_not_normalized_or_truncated() -> TestResult {
    let mut value = page(vec![event(1)]);
    value.events[0].title = "ñ e\u{301} \u{1b}[31m\u{202e}\n\u{0}".into();
    let original = value.clone();
    value.validate()?;
    assert_eq!(decode_read_page(&serde_json::to_vec(&value)?)?, original);
    // A long title is data, not a reason to refuse the page —
    // 5 KB passes; only one past a whole bridge page is a limit.
    value.events[0].title = "ñ".repeat(2_600);
    value.validate()?;
    value.events[0].title = "x".repeat(MAX_TITLE_BYTES + 1);
    assert_eq!(value.validate(), Err(CalendarError::LimitExceeded));
    Ok(())
}

#[test]
fn recurring_detached_all_day_events_preserve_original_dates_and_dst_length() -> TestResult {
    let mut value = page(vec![event(1)]);
    value.window.start_unix_ms = -DAY_MS;
    value.window.end_unix_ms = 2 * DAY_MS;
    let row = &mut value.events[0];
    row.start_unix_ms = 0;
    row.end_unix_ms = 25 * 3_600_000;
    row.original_occurrence_unix_ms = Some(-DAY_MS);
    row.is_all_day = true;
    row.is_detached = true;
    row.event_timezone = Some("Europe/Madrid".into());
    row.is_floating = false;
    let mut recurrence = row.clone();
    recurrence.observation_id = event(2).observation_id;
    recurrence.start_unix_ms = DAY_MS;
    recurrence.end_unix_ms = 90_000_000;
    value.events.push(recurrence);
    value.total = 2;
    let original = value.clone();
    value.validate()?;
    assert_eq!(decode_read_page(&serde_json::to_vec(&value)?)?, original);
    // A synthetic 25-hour occurrence checks preservation, not real EventKit DST.
    Ok(())
}

#[test]
fn event_observation_and_timezone_must_be_consistent() {
    let mut value = page(vec![event(1), event(1)]);
    assert_eq!(value.validate(), Err(CalendarError::InvalidValue));
    value = page(vec![event(1)]);
    value.events[0].is_floating = false;
    assert_eq!(value.validate(), Err(CalendarError::InvalidValue));
    value = page(vec![event(1)]);
    value.events[0].interpreted_timezone = "UTC".into();
    assert_eq!(value.validate(), Err(CalendarError::InvalidValue));
}

#[test]
fn status_denial_physical_pending_and_empty_local_read_are_not_conflated() -> TestResult {
    let bytes = br#"{"authorization":"denied",
        "physical_read_pending":true,"failure":"calendar_permission_required","read_only":true}"#;
    let status = decode_status(bytes)?;
    assert!(status.physical_read_pending);
    let empty = page(vec![]);
    empty.validate_response(&query(), &facts())?;
    let mut value = page(vec![event(1)]);
    value.provider_sync_status = "complete".into();
    assert_eq!(value.validate(), Err(CalendarError::InvalidValue));
    // A status never carries a selection any more.
    assert_eq!(
        decode_status(
            br#"{"authorization":"full_access","connected":true,
        "physical_read_pending":false,"read_only":true}"#
        ),
        Err(CalendarError::InvalidWire)
    );
    Ok(())
}

#[test]
fn strict_wire_rejects_floats_negative_u64_invalid_utf8_and_null_status() -> TestResult {
    let mut value = serde_json::to_value(inventory(vec![entry(1)]))?;
    for bad in [json!(-1), json!(2.0), json!("2"), Value::Null] {
        value["privacy_generation"] = bad;
        assert_eq!(
            decode_inventory_page(&serde_json::to_vec(&value)?),
            Err(CalendarError::InvalidWire)
        );
    }
    assert_eq!(
        decode_inventory_page(&[b'{', 255, b'}']),
        Err(CalendarError::InvalidWire)
    );
    assert_eq!(
        decode_status(br#"{"authorization":"unknown","physical_read_pending":false,"read_only":true,"failure":null}"#),
        Err(CalendarError::InvalidWire)
    );
    Ok(())
}

/// An event's reminders travel in the read under the limits a write
/// has, and the ones Orbit does not edit as a count that is never zero.
#[test]
fn reminders_travel_under_the_write_limits() {
    let with = |minutes: Option<Vec<u64>>, other: Option<u32>| CalendarOccurrence {
        alarms_minutes: minutes,
        alarms_not_editable: other,
        ..event(1)
    };
    assert_eq!(
        with(Some(vec![0, 10, 1440]), Some(1)).validate_alone("Europe/Madrid"),
        Ok(())
    );
    assert_eq!(with(None, None).validate_alone("Europe/Madrid"), Ok(()));
    for invalid in [
        with(Some(vec![5; MAX_EVENT_ALARMS + 1]), None),
        with(Some(vec![MAX_ALARM_MINUTES + 1]), None),
        with(Some(vec![10]), Some(0)),
    ] {
        assert_eq!(
            invalid.validate_alone("Europe/Madrid"),
            Err(CalendarError::InvalidValue)
        );
    }
}
