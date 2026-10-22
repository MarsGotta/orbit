//! The agenda in the app, with the macOS permission and nothing else.
//!
//! Granting Calendarios in macOS is the whole decision. So
//! the app has nothing to choose, confirm or share: its agenda shows every
//! calendar on the Mac, Ajustes shows the permission and how many calendars
//! there are, and when Deimos cannot read the agenda the app names which of
//! the real conditions is missing — the permission, the pairing, a reachable
//! Mac — the same ones the bridge enforces.
//!
//! These cases keep what an earlier suite protected about the
//! live read, without its fourth condition («calendarios elegidos»), which no
//! longer exists, and with the permission kept in three values: an unknown
//! permission is never shown as granted and never as denied.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use orbit_app_core::{
    AppSnapshot, CalendarReadingBlocker, CalendarReadingState, DaemonConnection, LocalZone,
    PermissionState,
    features::{FeatureInput, agenda, permissions, status},
    topics::TopicCache,
};
use orbit_core::{
    AccountSummary, Capabilities, Connection,
    calendar::{
        CLOCK_DOMAIN, CalendarAuthorization, CalendarEntry, CalendarOccurrence, CalendarQuery,
        CalendarReference, CalendarScope, CalendarSnapshot,
    },
    reads::ReadRequest,
    refusal::{Permission, Refusal},
    topics::{Knowledge, Pending, TopicData, TopicFrame, TopicId},
};
use serde_json::json;

const READ_ID: &str = "88888888-8888-4888-8888-888888888888";
const INSTANCE_ID: &str = "99999999-9999-4999-8999-999999999999";
/// Wednesday 14 October 2026, 10:00 UTC.
const NOW_MS: i64 = 1_792_058_400_000;
const DAY_MS: i64 = 86_400_000;

fn entry(index: usize) -> CalendarEntry {
    CalendarEntry {
        reference: CalendarReference {
            calendar_id: format!("calendar-{index:03}"),
            source_id: "icloud".into(),
        },
        title: format!("Agenda {index}"),
        source_title: "iCloud".into(),
        calendar_type: "caldav".into(),
        color: None,
        can_edit: None,
        is_default: None,
    }
}

fn event(calendar: &CalendarEntry) -> CalendarOccurrence {
    serde_json::from_value(json!({
        "observation_id": "55555555-5555-4555-8555-000000000001",
        "calendar": calendar.reference,
        "calendar_item_id": "item-1",
        "start_unix_ms": NOW_MS + 3_600_000,
        "end_unix_ms": NOW_MS + 7_200_000,
        "interpreted_timezone": "UTC",
        "is_floating": true,
        "is_all_day": false,
        "is_detached": false,
        "title": "Revisión de Orbit",
        "status": "confirmed",
        "invitees_status": "not_provided",
        "event_url_status": "not_provided",
    }))
    .expect("synthetic event")
}

struct CalendarRead {
    authorization: CalendarAuthorization,
    calendars: Option<Vec<CalendarEntry>>,
    snapshot: Option<Box<CalendarSnapshot>>,
    hidden_events: u32,
}

/// The week as the daemon assembles it: every calendar, nothing chosen.
fn read_view(calendars: usize, hidden_events: u32) -> CalendarRead {
    let list: Vec<CalendarEntry> = (0..calendars).map(entry).collect();
    let events: Vec<CalendarOccurrence> = list.first().map(event).into_iter().collect();
    CalendarRead {
        authorization: CalendarAuthorization::FullAccess,
        calendars: Some(list),
        snapshot: Some(Box::new(CalendarSnapshot {
            snapshot_id: READ_ID.into(),
            scope: CalendarScope {
                scope_id: READ_ID.into(),
                native_instance_id: INSTANCE_ID.into(),
                privacy_generation: 1,
                calendars: events.iter().map(|event| event.calendar.clone()).collect(),
                include_invitees: true,
                include_event_url: true,
            },
            window: CalendarQuery {
                scope_id: READ_ID.into(),
                start_unix_ms: NOW_MS - DAY_MS,
                end_unix_ms: NOW_MS + 6 * DAY_MS,
            },
            clock_domain: CLOCK_DOMAIN.into(),
            capture_started_monotonic_ns: 1,
            capture_completed_monotonic_ns: 2,
            observed_at_unix_ms: NOW_MS,
            system_timezone: "UTC".into(),
            provider_sync_status: "unknown".into(),
            enumeration: "bounded_local_read".into(),
            events,
            omitted: vec![],
            unreadable_calendars: vec![],
        })),
        hidden_events,
    }
}

fn account() -> AccountSummary {
    AccountSummary {
        hub_url: Some("https://hub.orbit.test".into()),
        device_name: Some("Mac de Alina".into()),
        device_id_short: Some("7f31c1a2".into()),
    }
}

/// The user's Mac: the permission granted, paired and online, and 140
/// calendars — more than the 128 Orbit used to allow — with nothing chosen.
#[derive(Default)]
struct CalendarFixture {
    account: Option<AccountSummary>,
    connection: Connection,
    session_id: Option<String>,
    calendar: Option<Box<CalendarRead>>,
    capabilities: Capabilities,
}

fn set_capability(capabilities: &mut Capabilities, field: &str, value: serde_json::Value) {
    let mut wire = serde_json::to_value(&*capabilities).expect("fixture serializes");
    wire[field] = value;
    *capabilities = serde_json::from_value(wire).expect("fixture deserializes");
}

fn user_state() -> CalendarFixture {
    let mut fixture = CalendarFixture {
        connection: Connection::Online,
        account: Some(account()),
        calendar: Some(Box::new(read_view(140, 0))),
        ..CalendarFixture::default()
    };
    set_capability(&mut fixture.capabilities, "native_helper", json!(true));
    set_capability(
        &mut fixture.capabilities,
        "calendar",
        json!(CalendarAuthorization::FullAccess),
    );
    fixture.capabilities.calendars = Some(140);
    fixture
}

fn project(fixture: &CalendarFixture) -> AppSnapshot {
    let mut topics = TopicCache::default();
    let calendar_permission = match fixture.capabilities.calendar() {
        Some(CalendarAuthorization::FullAccess) => orbit_core::PermissionState::Granted,
        Some(CalendarAuthorization::Denied | CalendarAuthorization::Restricted) => {
            orbit_core::PermissionState::Denied
        }
        Some(CalendarAuthorization::NotDetermined) => orbit_core::PermissionState::NotDetermined,
        Some(CalendarAuthorization::Unknown | CalendarAuthorization::Unavailable) | None => {
            orbit_core::PermissionState::Unknown
        }
        Some(CalendarAuthorization::WriteOnly) => orbit_core::PermissionState::NotGranted,
    };
    topics.observe_at(
        TopicFrame {
            epoch: "calendar".into(),
            revision: 1,
            data: TopicData::Permissions(Knowledge::Present {
                value: orbit_core::topics::permissions::Value {
                    microphone: orbit_core::PermissionState::Unknown,
                    screen_audio: orbit_core::PermissionState::Unknown,
                    screen_recording: orbit_core::PermissionState::Unknown,
                    accessibility: orbit_core::PermissionState::Unknown,
                    calendar: calendar_permission,
                },
                as_of_ms: None,
            }),
        },
        Some(NOW_MS),
    );
    topics.observe_at(
        TopicFrame {
            epoch: "calendar".into(),
            revision: 2,
            data: TopicData::Status(Knowledge::Present {
                value: orbit_core::topics::status::Value {
                    attachments: None,
                    assistant_name: None,
                    agents: Vec::new(),
                    default_agent_id: None,
                    hub: fixture.connection.clone(),
                    account: fixture.account.clone(),
                    privacy_stopped: false,
                    helper_connected: true,
                    error_reports: None,
                },
                as_of_ms: None,
            }),
        },
        Some(NOW_MS),
    );
    let topic = if calendar_permission == orbit_core::PermissionState::Denied {
        TopicData::Agenda(Knowledge::Absent {
            refusal: Refusal::PermissionDenied {
                permission: Permission::Calendar,
            },
        })
    } else if calendar_permission == orbit_core::PermissionState::Unknown {
        TopicData::Agenda(Knowledge::Unknown {
            why: Pending::NotObservedYet,
        })
    } else {
        let native = fixture
            .calendar
            .as_deref()
            .and_then(|view| view.snapshot.as_deref());
        TopicData::Agenda(Knowledge::Present {
            value: orbit_core::topics::agenda::Value {
                invalidation: 0,
                calendars: fixture.capabilities.calendars,
                read_window: native.map(|snapshot| orbit_core::calendar::CalendarWindow {
                    start_unix_ms: snapshot.window.start_unix_ms,
                    end_unix_ms: snapshot.window.end_unix_ms,
                }),
                read_id: native.map(|snapshot| snapshot.snapshot_id.clone()),
                timezone: Some("UTC".into()),
                writes: Vec::new(),
            },
            as_of_ms: None,
        })
    };
    topics.observe_at(
        TopicFrame {
            epoch: "calendar".into(),
            revision: 3,
            data: topic,
        },
        Some(NOW_MS),
    );
    if let Some(view) = fixture.calendar.as_deref() {
        if let Some(calendars) = &view.calendars {
            assert!(
                topics.remember_read(
                    &ReadRequest::AgendaCalendars { cursor: None },
                    TopicId::Agenda,
                    Knowledge::Present {
                        value: orbit_core::topics::Page {
                            items: calendars
                                .iter()
                                .map(|row| serde_json::to_value(row).expect("calendar fixture"))
                                .collect(),
                            next: None,
                            total: calendars.len() as u64,
                            revision: 3,
                            store_version: None,
                        },
                        as_of_ms: None
                    }
                )
            );
        }
        if let Some(native) = view.snapshot.as_deref() {
            assert!(
                topics.remember_read(
                    &ReadRequest::AgendaRange {
                        start_ms: native.window.start_unix_ms,
                        end_ms: native.window.end_unix_ms,
                        cursor: None,
                    },
                    TopicId::Agenda,
                    Knowledge::Present {
                        value: orbit_core::topics::Page {
                            items: native
                                .events
                                .iter()
                                .map(|row| serde_json::to_value(row).expect("event fixture"))
                                .collect(),
                            next: None,
                            total: native.events.len() as u64 + u64::from(view.hidden_events),
                            revision: 3,
                            store_version: None,
                        },
                        as_of_ms: None
                    }
                )
            );
        }
    }
    let mut snapshot = AppSnapshot::empty(7, DaemonConnection::Ready);
    snapshot.system_timezone = "UTC".into();
    let input = FeatureInput::with_clock(
        &snapshot,
        &topics,
        orbit_app_core::calendar::LocalClock {
            now_ms: NOW_MS,
            zone: LocalZone::utc(),
            timezone_name: Ok("UTC".into()),
        },
    );
    let status_view = status::project(&input);
    let permission_view = permissions::project(&input);
    let agenda_view = agenda::project(&input);
    status::apply(&status_view, &mut snapshot);
    permissions::apply(&permission_view, &mut snapshot);
    agenda::apply(&agenda_view, &mut snapshot);
    snapshot
}

fn reading(state: &CalendarFixture) -> (CalendarReadingState, Option<CalendarReadingBlocker>) {
    let snapshot = project(state);
    (
        snapshot.calendar.reading.state,
        snapshot.calendar.reading.blocker,
    )
}

// ------------------------------------------------- with the permission

/// With the permission, Deimos can read the user's agenda; nothing was chosen,
/// confirmed or switched on, and no conversation is open.
#[test]
fn with_the_permission_deimos_can_read_with_nothing_chosen() {
    assert_eq!(
        reading(&user_state()),
        (CalendarReadingState::Available, None)
    );
    let mut open = user_state();
    open.session_id = Some("33333333-3333-4333-8333-333333333333".into());
    assert_eq!(reading(&open), reading(&user_state()));
}

/// The agenda shows every calendar on the Mac, however many.
#[test]
fn the_agenda_shows_every_calendar_and_offers_nothing_to_choose() {
    let snapshot = project(&user_state());
    assert_eq!(snapshot.calendar.calendars.len(), 140);
    assert_eq!(snapshot.calendar.events.len(), 1);
}

/// Ajustes shows the permission and how many calendars there are, from the
/// data — and an unknown count is unknown, never «0».
#[test]
fn settings_show_the_permission_and_the_count_from_the_data() {
    let snapshot = project(&user_state());
    assert_eq!(
        snapshot.settings.permissions.calendars,
        PermissionState::Granted
    );
    assert_eq!(snapshot.settings.calendars.total, Some(140));

    let mut unknown = user_state();
    unknown.calendar = None;
    unknown.capabilities.calendars = None;
    let settings = serde_json::to_value(project(&unknown).settings).expect("settings");
    assert_eq!(settings["calendars"], json!({"total": null}));

    // Opening another screen evicts the calendar view; the count stays.
    let mut evicted = user_state();
    evicted.calendar = None;
    assert_eq!(project(&evicted).settings.calendars.total, Some(140));
}

/// A keyed page that announces more events than it returns keeps the
/// missing count in the refusal instead of presenting a truncated week.
#[test]
fn a_short_page_names_how_many_events_are_missing() {
    let mut state = user_state();
    state.calendar = Some(Box::new(read_view(3, 12)));
    let view = project(&state).calendar;
    assert_eq!(view.reading.state, CalendarReadingState::Unavailable);
    let refusal: serde_json::Value =
        serde_json::from_str(view.reading.detail.as_deref().unwrap()).unwrap();
    assert_eq!(
        refusal,
        serde_json::json!({"code":"daemon_reply_invalid", "field":"calendar.events", "issue":{"kind":"count_mismatch", "delivered":1, "total":13}, "retry":{"kind":"check_outcome"}})
    );
    assert!(view.events.is_empty());
    assert_eq!(project(&user_state()).calendar.events.len(), 1);
}

// ------------------------------------------- each missing condition, named

/// Withdrawing Calendarios in macOS is named, not folded into anything.
#[test]
fn withdrawing_the_permission_is_named() {
    let mut state = user_state();
    let mut view = read_view(3, 0);
    view.authorization = CalendarAuthorization::Denied;
    view.snapshot = None;
    state.calendar = Some(Box::new(view));
    set_capability(
        &mut state.capabilities,
        "calendar",
        json!(CalendarAuthorization::Denied),
    );
    assert_eq!(
        reading(&state),
        (
            CalendarReadingState::Unavailable,
            Some(CalendarReadingBlocker::CalendarPermission)
        )
    );
}

/// Unpairing is its own cause, and a revoked device is unpaired.
#[test]
fn unpairing_is_named_and_is_not_a_disconnection() {
    let mut state = user_state();
    state.account = None;
    assert_eq!(
        reading(&state),
        (
            CalendarReadingState::Unavailable,
            Some(CalendarReadingBlocker::NotPaired)
        )
    );
    let mut revoked = user_state();
    revoked.connection = Connection::Revoked;
    assert_eq!(reading(&revoked).1, Some(CalendarReadingBlocker::NotPaired));
}

/// A paired Mac Deimos cannot reach is neither readable nor unpaired.
#[test]
fn a_disconnected_mac_is_neither_readable_nor_unpaired() {
    for connection in [Connection::Offline, Connection::Connecting] {
        let mut state = user_state();
        state.connection = connection.clone();
        assert_eq!(
            reading(&state),
            (
                CalendarReadingState::Unavailable,
                Some(CalendarReadingBlocker::MacDisconnected)
            ),
            "{connection:?}"
        );
    }
}

/// An unknown permission — the helper has not said yet — is shown as
/// unknown: never as granted, never as denied, and names no cause.
#[test]
fn an_unknown_permission_is_neither_granted_nor_denied() {
    for authorization in [None, Some(CalendarAuthorization::Unknown)] {
        let mut state = user_state();
        state.calendar = None;
        set_capability(&mut state.capabilities, "calendar", json!(authorization));
        let snapshot = project(&state);
        assert_eq!(
            (
                snapshot.calendar.reading.state,
                snapshot.calendar.reading.blocker
            ),
            (CalendarReadingState::Unknown, None),
            "{authorization:?}"
        );
        assert_eq!(
            snapshot.settings.permissions.calendars,
            PermissionState::Unknown
        );
    }
}

/// §3.5 of the 2026-09-24 handoff: the daemon evicts the calendar view when
/// another screen takes its place. With the permission known from the
/// helper's status, the eviction changes nothing: absence is not a fact.
#[test]
fn an_evicted_calendar_view_does_not_change_the_answer() {
    let mut state = user_state();
    state.calendar = None;
    assert_eq!(reading(&state), (CalendarReadingState::Available, None));
}

/// The nearest missing condition wins: without the permission, the
/// pairing and the connection are not the cause the user is told.
#[test]
fn the_nearest_missing_condition_is_the_one_named() {
    let mut state = user_state();
    state.connection = Connection::Offline;
    state.account = None;
    set_capability(
        &mut state.capabilities,
        "calendar",
        json!(CalendarAuthorization::Denied),
    );
    state.calendar = None;
    assert_eq!(
        reading(&state).1,
        Some(CalendarReadingBlocker::CalendarPermission)
    );
}
