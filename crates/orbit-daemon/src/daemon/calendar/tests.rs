//! Actor fixtures with a synthetic native NDJSON peer. No EventKit/TCC/audio SDK.
//!
//! The agenda on screen reads every calendar in pages and windows,
//! with nothing selected; the helper's pages are joined only when they come
//! from one enumeration; a withdrawn permission cuts the agenda off.
use std::time::Duration;

use anyhow::{Context, Result};
use orbit_core::{calendar::*, calendar_control::CalendarCommand as Request, protocol::Command};
use tokio::sync::mpsc;

use super::*;

// The projection measurement uses the real queue bound, without a second value.
pub(in crate::daemon) const WRITE_RECORDS: usize = super::writes::WRITE_RECORDS;
use crate::daemon::audio::tests::fixture;

#[tokio::test]
async fn stop_and_revocation_clear_the_active_calendar_view() -> Result<()> {
    for revoked in [false, true] {
        let directory = tempfile::tempdir()?;
        let (mut runtime, _native) = fixture(directory.path())?;
        runtime.features.calendar.screen = Some(ScreenRead::new(CalendarWindow {
            start_unix_ms: OCTOBER_MS,
            end_unix_ms: OCTOBER_MS + DAY_MS,
        }));
        if revoked {
            runtime
                .apply(orbit_core::Event::Connect(orbit_core::Connection::Revoked))
                .await?;
        } else {
            runtime
                .privacy_stop(orbit_core::privacy_stop::UserStop::Order)
                .await;
        }
        assert!(
            runtime.features.calendar.screen.is_none(),
            "revoked={revoked}"
        );
        let detail = read_failure(&mut runtime, agenda_request(week()))?;
        assert!(
            serde_json::from_str::<Value>(&detail)?["code"] == "privacy_stopped",
            "revoked={revoked}: {detail}"
        );
    }
    Ok(())
}

#[test]
fn local_calendar_permission_command_failure_does_not_claim_macos_was_asked() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _native) = fixture(directory.path())?;
    runtime.calendar_job_failed(Job::Permission, "calendar_command_invalid");
    let detail = read_failure(&mut runtime, agenda_request(week()))?;
    assert!(
        serde_json::from_str::<Value>(&detail)?["cause"]["kind"] == "native_command",
        "{detail}"
    );
    assert!(!detail.contains("macOS no contestó"), "{detail}");
    Ok(())
}

#[tokio::test]
async fn full_helper_channel_records_calendar_disconnect_once() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut native) = fixture(directory.path())?;
    seed(&mut runtime, &mut native).await?;
    runtime.diagnostics = Some(crate::diagnostics::DiagnosticLog::in_root(directory.path()));
    for _ in 0..64 {
        runtime.native.try_send(json!({"fixture":"occupied"}))?;
    }
    runtime.retire_native_calendar();
    let log = std::fs::read_to_string(directory.path().join("logs/daemon.log")).unwrap_or_default();
    assert_eq!(
        log.matches("calendar_disconnect_undelivered").count(),
        1,
        "{log}"
    );
    assert!(log.contains("64"), "{log}");
    assert_eq!(native.try_recv()?["fixture"], "occupied");
    Ok(())
}

pub(in crate::daemon) const HOST: &str = "22222222-2222-4222-8222-222222222222";
pub(in crate::daemon) const MACH: u64 = 9_000_000_000_000_000_000;
pub(in crate::daemon) const DAY_MS: i64 = 86_400_000;
/// 1 October 2026, 00:00 UTC.
pub(in crate::daemon) const OCTOBER_MS: i64 = 1_790_812_800_000;

pub(in crate::daemon) fn status(authorization: &str) -> Value {
    json!({"authorization": authorization, "physical_read_pending": false})
}

pub(in crate::daemon) fn host_state(generation: u64, stopped: bool, calendar: Value) -> Value {
    json!({"event":"state","native_instance_id":HOST,"privacy_generation":generation,
        "privacy_stopped":stopped,"calendar":calendar})
}

/// A Mac whose helper announced the calendar backend and full access. With
/// the permission known, the daemon lists every calendar by itself; the
/// synthetic helper answers that listing with three calendars.
pub(in crate::daemon) async fn seed(
    runtime: &mut Runtime,
    receiver: &mut mpsc::Receiver<Value>,
) -> Result<()> {
    runtime
        .native_event(json!({"event":"ready","capabilities":["calendar"]}))
        .await?;
    runtime
        .native_event(host_state(2, false, status("full_access")))
        .await?;
    while let Ok(command) = receiver.try_recv() {
        if command["command"] == "calendar_sources" {
            ack(runtime, &command, calendars_page(3)).await?;
        }
    }
    assert_eq!(runtime.state.capabilities().calendars, Some(3));
    Ok(())
}

#[tokio::test]
async fn changed_generation_calendar_reply_is_not_reported_as_eventkit_timeout() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut native) = fixture(directory.path())?;
    seed(&mut runtime, &mut native).await?;
    runtime.features.calendar.wanted_inventory = true;
    runtime.pump_calendar();
    let command = native.try_recv().context("inventory request")?;
    let started = runtime.state.generation();
    {
        let fixture_value = serde_json::json!(runtime.state.generation() + (1));
        crate::daemon::authority_fixture::set(&mut runtime.state, "generation", fixture_value);
    }
    assert!(
        runtime.calendar_response(&json!({"event":"response","id":command["id"],
        "ok":false,"error":{"code":"fixture","message":"fixture"}}))
    );
    assert_eq!(
        runtime.features.calendar.inventory_failure,
        Some(orbit_core::refusal::Refusal::ReadInterrupted {
            cause: orbit_core::refusal::ReadInterruption::GenerationChanged {
                started_generation: started,
                current_generation: runtime.state.generation()
            }
        })
    );
    let detail = read_failure(
        &mut runtime,
        orbit_core::reads::ReadRequest::AgendaCalendars { cursor: None },
    )?;
    assert!(detail.contains(&started.to_string()), "{detail}");
    assert!(
        detail.contains(&runtime.state.generation().to_string()),
        "{detail}"
    );
    Ok(())
}

#[tokio::test]
async fn restarted_helper_names_the_calendar_instance_and_generation_it_replaced() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut native) = fixture(directory.path())?;
    seed(&mut runtime, &mut native).await?;
    runtime.features.calendar.wanted_inventory = true;
    runtime.pump_calendar();
    let _command = native.try_recv().context("inventory request")?;

    runtime
        .native_event(json!({"event":"ready","capabilities":["calendar"]}))
        .await?;

    let failure = serde_json::to_value(
        runtime
            .features
            .calendar
            .inventory_failure
            .as_ref()
            .context("inventory failure")?,
    )?;
    assert_eq!(failure["code"], "read_interrupted");
    assert_eq!(failure["cause"]["kind"], "helper_restarted");
    let detail = read_failure(
        &mut runtime,
        orbit_core::reads::ReadRequest::AgendaCalendars { cursor: None },
    )?;
    assert!(detail.contains(HOST), "{detail}");
    assert!(
        serde_json::from_str::<Value>(&detail)?["cause"]["previous_generation"] == 2,
        "{detail}"
    );
    assert!(
        serde_json::from_str::<Value>(&detail)?["cause"]["current_generation"].is_null(),
        "{detail}"
    );
    Ok(())
}

#[tokio::test]
async fn calendar_read_pages_the_known_listing_by_id() -> Result<()> {
    use orbit_core::{
        reads::ReadRequest,
        topics::{Cursor, Knowledge},
    };
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut native) = fixture(directory.path())?;
    seed(&mut runtime, &mut native).await?;
    runtime.features.calendar.view.calendars = None;
    assert!(matches!(
        super::super::reads::dispatch(
            &mut runtime,
            ReadRequest::AgendaCalendars { cursor: None },
            7
        ),
        Knowledge::Unknown { .. }
    ));
    let listing = calendars_page(150);
    let mut calendars: Vec<orbit_core::calendar::CalendarEntry> =
        serde_json::from_value(listing["calendars"].clone())?;
    for calendar in &mut calendars {
        calendar.title.push_str(&"x".repeat(56));
    }
    runtime.features.calendar.view.calendars = Some(calendars);
    for invalid_id in [
        String::new(),
        "\n".into(),
        "x".repeat(orbit_core::calendar::MAX_LABEL_BYTES + 1),
    ] {
        let invalid = super::super::reads::dispatch(
            &mut runtime,
            ReadRequest::AgendaCalendars {
                cursor: Some(Cursor::Id { id: invalid_id }),
            },
            7,
        );
        assert!(matches!(
            invalid,
            Knowledge::Absent {
                refusal: orbit_core::refusal::Refusal::InvalidParameter { .. }
            }
        ));
    }
    let answer = super::super::reads::dispatch(
        &mut runtime,
        ReadRequest::AgendaCalendars { cursor: None },
        7,
    );
    let bytes = serde_json::to_vec(&answer)?.len();
    let Knowledge::Present { value, .. } = answer else {
        anyhow::bail!("known calendar listing was not returned: {answer:?}")
    };
    assert_eq!(value.items.len(), 150);
    assert!(value.next.is_none());
    assert!(
        bytes
            <= ReadRequest::AgendaCalendars { cursor: None }
                .id()
                .max_bytes()
    );
    assert!(
        (28 * 1024..30 * 1024).contains(&bytes),
        "fixture is not the ADR-sized listing: {bytes}"
    );
    assert_eq!(value.items[0]["reference"]["calendar_id"], "calendar-0000");
    assert_eq!(
        value.items[149]["reference"]["calendar_id"],
        "calendar-0149"
    );
    let bad = super::super::reads::dispatch(
        &mut runtime,
        ReadRequest::AgendaCalendars {
            cursor: Some(Cursor::Index { index: 1 }),
        },
        7,
    );
    assert!(matches!(
        bad,
        Knowledge::Absent {
            refusal: orbit_core::refusal::Refusal::InvalidParameter { .. }
        }
    ));
    runtime
        .features
        .calendar
        .view
        .calendars
        .as_mut()
        .context("listing missing")?
        .retain(|entry| entry.reference.calendar_id != "calendar-0073");
    let missing = super::super::reads::dispatch(
        &mut runtime,
        ReadRequest::AgendaCalendars {
            cursor: Some(Cursor::Id {
                id: "calendar-0073".into(),
            }),
        },
        7,
    );
    let Knowledge::Present { value, .. } = missing else {
        anyhow::bail!("a removed calendar cursor did not continue: {missing:?}")
    };
    assert_eq!(value.items[0]["reference"]["calendar_id"], "calendar-0074");
    Ok(())
}

#[test]
fn unread_listing_and_failed_helper_are_distinct() -> Result<()> {
    use orbit_core::{reads::ReadRequest, topics::Knowledge};
    let directory = tempfile::tempdir()?;
    let (mut runtime, _native) = fixture(directory.path())?;
    let read = || ReadRequest::AgendaCalendars { cursor: None };
    assert!(matches!(
        super::super::reads::dispatch(&mut runtime, read(), 1),
        Knowledge::Unknown { .. }
    ));
    runtime.features.calendar.view.authorization = CalendarAuthorization::Unavailable;
    assert!(matches!(
        super::super::reads::dispatch(&mut runtime, read(), 1),
        Knowledge::Absent {
            refusal: orbit_core::refusal::Refusal::CalendarBackendUnavailable
        }
    ));
    {
        let fixture_value = serde_json::json!(true);
        crate::daemon::authority_fixture::set(&mut runtime.state, "privacy_stopped", fixture_value);
    }
    assert!(matches!(
        super::super::reads::dispatch(&mut runtime, read(), 1),
        Knowledge::Absent {
            refusal: orbit_core::refusal::Refusal::PrivacyStopped { .. }
        }
    ));
    Ok(())
}

#[tokio::test]
async fn cursor_walks_large_listing_without_repeating_or_cutting_rows() -> Result<()> {
    use orbit_core::{
        reads::ReadRequest,
        topics::{Cursor, Knowledge},
    };
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut native) = fixture(directory.path())?;
    seed(&mut runtime, &mut native).await?;
    let listing = calendars_page(500);
    let mut calendars: Vec<orbit_core::calendar::CalendarEntry> =
        serde_json::from_value(listing["calendars"].clone())?;
    for calendar in &mut calendars {
        calendar.title.push_str(&"x".repeat(56));
    }
    runtime.features.calendar.view.calendars = Some(calendars);
    let full_page = super::super::reads::dispatch(
        &mut runtime,
        ReadRequest::AgendaCalendars { cursor: None },
        7,
    );
    let full_page_bytes = serde_json::to_vec(&full_page)?.len();
    assert!(full_page_bytes > 60 * 1024 && full_page_bytes <= 64 * 1024);
    let mut cursor = None;
    let mut ids = Vec::new();
    let mut pages = 0;
    loop {
        let answer =
            super::super::reads::dispatch(&mut runtime, ReadRequest::AgendaCalendars { cursor }, 7);
        assert!(
            serde_json::to_vec(&answer)?.len()
                <= ReadRequest::AgendaCalendars { cursor: None }
                    .id()
                    .max_bytes()
        );
        let Knowledge::Present { value, .. } = answer else {
            anyhow::bail!("known listing became unavailable")
        };
        ids.extend(value.items.iter().map(|item| {
            item["reference"]["calendar_id"]
                .as_str()
                .unwrap_or("")
                .to_owned()
        }));
        pages += 1;
        cursor = value.next;
        if let Some(next) = &cursor {
            assert!(matches!(next, Cursor::Id { .. }));
        }
        if cursor.is_none() {
            break;
        }
    }
    assert!(pages >= 2);
    assert_eq!(ids.len(), 500);
    assert_eq!(
        ids.iter().collect::<std::collections::BTreeSet<_>>().len(),
        500
    );
    assert_eq!(ids.first().map(String::as_str), Some("calendar-0000"));
    assert_eq!(ids.last().map(String::as_str), Some("calendar-0499"));
    Ok(())
}

#[tokio::test]
async fn failed_refresh_does_not_serve_an_old_listing() -> Result<()> {
    use orbit_core::{reads::ReadRequest, topics::Knowledge};
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut native) = fixture(directory.path())?;
    seed(&mut runtime, &mut native).await?;
    assert!(matches!(
        super::super::reads::dispatch(
            &mut runtime,
            ReadRequest::AgendaCalendars { cursor: None },
            1
        ),
        Knowledge::Present { .. }
    ));
    runtime.calendar_job_failed(
        Job::Inventory {
            after: 0,
            read: None,
            restarts: 0,
        },
        "calendar_read_expired",
    );
    let answer = super::super::reads::dispatch(
        &mut runtime,
        ReadRequest::AgendaCalendars { cursor: None },
        1,
    );
    let Knowledge::Absent { refusal } = answer else {
        anyhow::bail!("failed listing was served as a fact: {answer:?}")
    };
    assert!(serde_json::to_value(&refusal)?["cause"]["kind"] == "calendar_expired");
    assert!(refusal.to_string().contains("5000"));
    let mut board = super::super::topics::TopicBoard::new("failed-inventory".into());
    let receiver = board.channels()[&orbit_core::topics::TopicId::Agenda].subscribe();
    board.publish(&super::super::topics::TopicSource {
        state: &runtime.state,
        runtime: Some(&runtime),
        now_ms: 0,
    });
    let topic = receiver.borrow().data.clone();
    let orbit_core::topics::TopicData::Agenda(Knowledge::Present { value, .. }) = topic else {
        anyhow::bail!("a failed inventory blanked the agenda topic: {topic:?}")
    };
    assert_eq!(value.calendars, None);
    runtime.revoke_calendar(
        "calendar_read_restarted",
        Runtime::bridge_refusal_code("calendar_read_restarted"),
    );
    assert!(matches!(
        super::super::reads::dispatch(
            &mut runtime,
            ReadRequest::AgendaCalendars { cursor: None },
            1
        ),
        Knowledge::Unknown { .. }
    ));
    Ok(())
}

/// One synthetic event starting `minutes` after `start`.
pub(in crate::daemon) fn event(index: usize, start: i64, calendar: &str) -> Value {
    json!({
        "observation_id": format!("55555555-5555-4555-8555-{index:012}"),
        "calendar": {"calendar_id": calendar, "source_id": "account"},
        "calendar_item_id": format!("item-{index}"),
        "start_unix_ms": start,
        "end_unix_ms": start + 1_800_000,
        "interpreted_timezone": "UTC",
        "is_floating": true,
        "is_all_day": false,
        "is_detached": false,
        "title": format!("PRIVATE_SYNTHETIC_EVENT_{index}"),
        "status": "confirmed",
        "invitees_status": "not_provided",
        "event_url_status": "not_provided",
    })
}

/// The page the helper would send for `query`, holding `events` of `total`.
pub(in crate::daemon) fn page(
    query: &Value,
    events: Vec<Value>,
    total: usize,
    read: &str,
) -> Value {
    let after = query["after_index"].as_u64().unwrap_or(0) as usize;
    let mut value = json!({
        "read_id": read,
        "native_instance_id": HOST,
        "privacy_generation": 2,
        "window": {"start_unix_ms": query["start_unix_ms"], "end_unix_ms": query["end_unix_ms"]},
        "clock_domain": CLOCK_DOMAIN,
        "capture_started_monotonic_ns": MACH,
        "capture_completed_monotonic_ns": MACH + 100,
        "observed_at_unix_ms": 1_800_000_000_000_i64,
        "system_timezone": "UTC",
        "provider_sync_status": "unknown",
        "enumeration": "bounded_local_read",
        "after_index": after,
        "total": total,
        "events": events,
    });
    let end = after + value["events"].as_array().map_or(0, Vec::len);
    if end < total {
        value["next_after_index"] = json!(end);
    }
    value
}

pub(in crate::daemon) fn calendars_page(count: usize) -> Value {
    let calendars: Vec<Value> = (0..count)
        .map(|index| {
            json!({"reference":{"calendar_id":format!("calendar-{index:04}"),"source_id":"account"},
                "title":format!("Agenda {index}"),"source_title":"Fixture","calendar_type":"caldav"})
        })
        .collect();
    json!({"inventory_id":"11111111-1111-4111-8111-111111111111","native_instance_id":HOST,
        "privacy_generation":2,"clock_domain":CLOCK_DOMAIN,"observed_monotonic_ns":MACH,
        "completed_monotonic_ns":MACH+100,"calendars":calendars,"after_index":0,"total":count})
}

pub(in crate::daemon) async fn ack(
    runtime: &mut Runtime,
    command: &Value,
    result: Value,
) -> Result<()> {
    runtime
        .native_event(json!({"event":"response","id":command["id"],"ok":true,"result":result}))
        .await
}

pub(in crate::daemon) async fn reject(
    runtime: &mut Runtime,
    command: &Value,
    code: &str,
) -> Result<()> {
    runtime
        .native_event(json!({"event":"response","id":command["id"],"ok":false,
            "error":{"code":code,"message":"synthetic helper refusal"}}))
        .await
}

/// The next calendar command the helper receives. The daemon's own general
/// `status` (sent on every `ready`) is not the calendar's and is skipped.
pub(in crate::daemon) async fn next_command(receiver: &mut mpsc::Receiver<Value>) -> Result<Value> {
    loop {
        let command = tokio::time::timeout(Duration::from_secs(2), receiver.recv())
            .await
            .map_err(|_| anyhow::anyhow!("the helper never received the order"))?
            .context("the fake helper disconnected")?;
        if command["command"] != "status" {
            return Ok(command);
        }
    }
}

fn week() -> CalendarWindow {
    CalendarWindow {
        start_unix_ms: OCTOBER_MS,
        end_unix_ms: OCTOBER_MS + 7 * DAY_MS,
    }
}

#[test]
fn unobserved_helper_is_unknown_not_a_refusal() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (runtime, _helper) = fixture(directory.path())?;
    assert!(matches!(
        runtime.calendar_access(),
        orbit_core::topics::Knowledge::Unknown {
            why: orbit_core::topics::Pending::NotObservedYet
        }
    ));
    Ok(())
}

async fn read_week(runtime: &mut Runtime) -> Result<()> {
    runtime
        .command(Command::Calendar {
            request: Request::Read { window: week() },
        })
        .await
}

#[tokio::test]
async fn nothing_is_read_before_the_helper_announces_its_calendar_backend() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    runtime
        .native_event(host_state(2, false, status("full_access")))
        .await?;
    let detail = read_failure(&mut runtime, agenda_request(week()))?;
    assert!(detail.contains("backend"), "{detail}");
    assert!(read_week(&mut runtime).await.is_err());
    assert!(receiver.try_recv().is_err());
    Ok(())
}

/// With the permission, the agenda on screen is read — every
/// calendar, nothing listed or chosen first — and kept in RAM only.
#[tokio::test]
async fn the_week_on_screen_is_read_with_nothing_chosen() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    seed(&mut runtime, &mut receiver).await?;
    read_week(&mut runtime).await?;
    let read = next_command(&mut receiver).await?;
    assert_eq!(read["command"], "calendar_snapshot");
    assert!(read["calendar_selection"].is_null());
    assert_eq!(read["calendar_query"]["start_unix_ms"], OCTOBER_MS);
    assert_eq!(
        read["calendar_query"]["end_unix_ms"],
        OCTOBER_MS + 7 * DAY_MS
    );
    assert_eq!(read["calendar_query"]["after_index"], 0);
    let events = vec![
        event(1, OCTOBER_MS + 3_600_000, "work"),
        event(2, OCTOBER_MS + DAY_MS, "personal"),
        event(3, OCTOBER_MS + 2 * DAY_MS, "holidays"),
    ];
    ack(
        &mut runtime,
        &read,
        page(
            &read["calendar_query"],
            events,
            3,
            "44444444-4444-4444-8444-444444444444",
        ),
    )
    .await?;

    assert!(matches!(
        read_agenda(&mut runtime, week()),
        orbit_core::topics::Knowledge::Present { .. }
    ));
    let snapshot = runtime
        .features
        .calendar
        .complete
        .as_ref()
        .context("the agenda did not arrive")?;
    assert_eq!(snapshot.events.len(), 3);
    assert_eq!(
        snapshot.scope.calendars.len(),
        3,
        "every calendar, none chosen"
    );
    assert!(snapshot.scope.include_invitees && snapshot.scope.include_event_url);
    // The list of calendars came first, by itself, when the permission was
    // known: the count and the names are there without anyone asking.
    assert_eq!(runtime.state.capabilities().calendars, Some(3));
    assert_eq!(
        runtime
            .features
            .calendar
            .view
            .calendars
            .as_ref()
            .map(Vec::len),
        Some(3)
    );
    // RAM only: nothing of the agenda is saved or sent to the hub.
    assert!(runtime.hub.is_none());
    runtime.fixture_store.save_snapshot(&runtime.state)?;
    let saved = runtime
        .fixture_store
        .load_snapshot::<serde_json::Value>()?
        .context("state missing")?;
    assert!(saved.get("calendar").is_none());
    assert!(!serde_json::to_string(&saved)?.contains("PRIVATE_SYNTHETIC_EVENT"));
    Ok(())
}

#[tokio::test]
async fn a_month_of_435_events_arrives_in_bounded_keyset_pages() -> Result<()> {
    use orbit_core::{
        reads::ReadRequest,
        topics::{Cursor, Knowledge},
    };

    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    seed(&mut runtime, &mut receiver).await?;
    let end_ms = OCTOBER_MS + 31 * DAY_MS;
    let events: Vec<Value> = (0..435)
        .map(|index| {
            let mut value = event(index, OCTOBER_MS + 3_600_000, "calendar-0000");
            value["title"] = json!(format!("Evento {index} {}", "x".repeat(500)));
            value
        })
        .collect();
    runtime
        .command(Command::Calendar {
            request: Request::Read {
                window: CalendarWindow {
                    start_unix_ms: OCTOBER_MS,
                    end_unix_ms: end_ms,
                },
            },
        })
        .await?;
    while !matches!(
        read_agenda(
            &mut runtime,
            CalendarWindow {
                start_unix_ms: OCTOBER_MS,
                end_unix_ms: end_ms
            }
        ),
        orbit_core::topics::Knowledge::Present { .. }
    ) {
        assert!(!matches!(
            read_agenda(
                &mut runtime,
                CalendarWindow {
                    start_unix_ms: OCTOBER_MS,
                    end_unix_ms: end_ms
                }
            ),
            orbit_core::topics::Knowledge::Absent { .. }
        ));
        let command = next_command(&mut receiver).await?;
        let query = &command["calendar_query"];
        let window_start = query["start_unix_ms"].as_i64().context("window start")?;
        let window_end = query["end_unix_ms"].as_i64().context("window end")?;
        let inside: Vec<_> = events
            .iter()
            .filter(|event| {
                event["start_unix_ms"]
                    .as_i64()
                    .is_some_and(|start| start >= window_start && start < window_end)
            })
            .cloned()
            .collect();
        let after = query["after_index"].as_u64().context("after_index")? as usize;
        let slice = inside.iter().skip(after).take(100).cloned().collect();
        let mut response = page(
            query,
            slice,
            inside.len(),
            "44444444-4444-4444-8444-444444444444",
        );
        let started = MACH + (after as u64 + 1) * 1_000;
        response["capture_started_monotonic_ns"] = json!(started);
        response["capture_completed_monotonic_ns"] = json!(started + 100);
        ack(&mut runtime, &command, response).await?;
    }
    assert_eq!(
        runtime
            .features
            .calendar
            .complete
            .as_ref()
            .context("complete read absent")?
            .events
            .len(),
        events.len()
    );
    let mut cursor = None;
    let mut seen = Vec::new();
    let mut pages = 0;
    let mut largest_page = 0;
    loop {
        let answer = crate::daemon::reads::dispatch(
            &mut runtime,
            ReadRequest::AgendaRange {
                start_ms: OCTOBER_MS,
                end_ms,
                cursor: cursor.clone(),
            },
            7,
        );
        let bytes = serde_json::to_vec(&answer)?.len();
        largest_page = largest_page.max(bytes);
        assert!(bytes <= 128 * 1024, "page {pages} is {bytes} bytes");
        let Knowledge::Present { value, .. } = answer else {
            anyhow::bail!("agenda page was not present");
        };
        assert_eq!(value.revision, 7);
        assert_eq!(value.total, 435);
        seen.extend(
            value
                .items
                .iter()
                .map(|event| event["observation_id"].clone()),
        );
        pages += 1;
        cursor = value.next;
        if cursor.is_none() {
            break;
        }
        assert!(matches!(cursor, Some(Cursor::StartAndId { .. })));
    }
    assert_eq!(seen.len(), 435);
    seen.dedup();
    assert_eq!(seen.len(), 435);
    assert!(pages > 1);
    let first = crate::daemon::reads::dispatch(
        &mut runtime,
        ReadRequest::AgendaRange {
            start_ms: OCTOBER_MS,
            end_ms,
            cursor: None,
        },
        7,
    );
    let first_bytes = serde_json::to_vec(&first)?.len();
    let Knowledge::Present {
        value: first_page, ..
    } = first
    else {
        anyhow::bail!("first agenda page was not present");
    };
    let last_id = first_page
        .items
        .last()
        .and_then(|item| item["observation_id"].as_str())
        .context("last event of the full page")?
        .to_owned();
    let ceiling = ReadRequest::AgendaRange {
        start_ms: OCTOBER_MS,
        end_ms,
        cursor: None,
    }
    .id()
    .max_bytes();
    let slack = ceiling - first_bytes;
    assert!(first_page.items.len() > 2 && slack < 500);
    let event = runtime
        .features
        .calendar
        .complete
        .as_deref_mut()
        .context("complete agenda")?
        .events
        .iter_mut()
        .find(|event| event.observation_id == last_id)
        .context("last event in complete agenda")?;
    event.title.push_str(&"q".repeat(slack + 1));
    let boundary = crate::daemon::reads::dispatch(
        &mut runtime,
        ReadRequest::AgendaRange {
            start_ms: OCTOBER_MS,
            end_ms,
            cursor: None,
        },
        7,
    );
    let boundary_bytes = serde_json::to_vec(&boundary)?.len();
    assert!(boundary_bytes <= ceiling, "page is {boundary_bytes} bytes");
    let Knowledge::Present {
        value: boundary_page,
        ..
    } = boundary
    else {
        anyhow::bail!("boundary agenda page was not present");
    };
    assert_eq!(boundary_page.items.len() + 1, first_page.items.len());
    eprintln!("agenda_month_pages events=435 pages={pages} largest_page_bytes={largest_page}");
    Ok(())
}

/// The agenda view asks for 63 days and the month view for up to 42,
/// wider than one helper window (`READ_WINDOW_MS`, 31 days). The daemon reads
/// the range in consecutive windows and serves it as one: the topic names the
/// whole range and `AgendaRange` for exactly that range returns the events of
/// every window. Nothing was missing for this; the test keeps it so.
#[tokio::test]
async fn a_range_wider_than_one_window_is_read_in_windows_and_served_whole() -> Result<()> {
    use orbit_core::{calendar::READ_WINDOW_MS, reads::ReadRequest, topics::Knowledge};

    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    seed(&mut runtime, &mut receiver).await?;
    let range = CalendarWindow {
        start_unix_ms: OCTOBER_MS,
        end_unix_ms: OCTOBER_MS + 63 * DAY_MS,
    };
    runtime
        .command(Command::Calendar {
            request: Request::Read { window: range },
        })
        .await?;
    let mut windows = Vec::new();
    while !matches!(
        read_agenda(&mut runtime, range),
        orbit_core::topics::Knowledge::Present { .. }
    ) {
        assert!(!matches!(
            read_agenda(&mut runtime, range),
            orbit_core::topics::Knowledge::Absent { .. }
        ));
        let command = next_command(&mut receiver).await?;
        let query = &command["calendar_query"];
        let start = query["start_unix_ms"].as_i64().context("window start")?;
        let end = query["end_unix_ms"].as_i64().context("window end")?;
        windows.push((start, end));
        // One event on the first day of each window.
        let index = windows.len();
        let mut response = page(
            query,
            vec![event(index, start + 3_600_000, "work")],
            1,
            "44444444-4444-4444-8444-444444444444",
        );
        let started = MACH + index as u64 * 1_000;
        response["capture_started_monotonic_ns"] = json!(started);
        response["capture_completed_monotonic_ns"] = json!(started + 100);
        ack(&mut runtime, &command, response).await?;
    }
    assert_eq!(
        windows,
        vec![
            (OCTOBER_MS, OCTOBER_MS + READ_WINDOW_MS),
            (OCTOBER_MS + READ_WINDOW_MS, OCTOBER_MS + 2 * READ_WINDOW_MS),
            (OCTOBER_MS + 2 * READ_WINDOW_MS, OCTOBER_MS + 63 * DAY_MS),
        ],
        "consecutive windows of at most 31 days, covering the range exactly"
    );
    assert_eq!(runtime.features.calendar.topic_window(), Some(range));
    let answer = crate::daemon::reads::dispatch(
        &mut runtime,
        ReadRequest::AgendaRange {
            start_ms: range.start_unix_ms,
            end_ms: range.end_unix_ms,
            cursor: None,
        },
        7,
    );
    let Knowledge::Present { value, .. } = answer else {
        anyhow::bail!("the 63-day range was not served: {answer:?}");
    };
    assert_eq!(value.total, 3);
    assert_eq!(value.items.len(), 3);
    assert_eq!(value.next, None);
    Ok(())
}

/// The store changed between two pages of one window: the pages are never
/// joined; the window is read again from its first page.
#[tokio::test]
async fn a_store_change_between_pages_reads_the_window_again() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    seed(&mut runtime, &mut receiver).await?;
    read_week(&mut runtime).await?;
    let all: Vec<Value> = (0..4)
        .map(|index| event(index, OCTOBER_MS + index as i64 * 60_000, "work"))
        .collect();
    let first = next_command(&mut receiver).await?;
    ack(
        &mut runtime,
        &first,
        page(
            &first["calendar_query"],
            all[..2].to_vec(),
            4,
            "44444444-4444-4444-8444-444444444444",
        ),
    )
    .await?;
    let second = next_command(&mut receiver).await?;
    runtime
        .native_event(json!({"event":"calendar_invalidated","code":"calendar_invalidated"}))
        .await?;
    ack(
        &mut runtime,
        &second,
        page(
            &second["calendar_query"],
            all[2..].to_vec(),
            4,
            "44444444-4444-4444-8444-000000000002",
        ),
    )
    .await?;
    let again = next_command(&mut receiver).await?;
    assert_eq!(again["command"], "calendar_snapshot");
    assert_eq!(
        again["calendar_query"]["after_index"], 0,
        "from its first page"
    );
    assert!(runtime.features.calendar.complete.is_none());
    Ok(())
}

/// EventKit ran out of time on a window: it is read in halves, not refused.
#[tokio::test]
async fn a_window_eventkit_could_not_finish_in_time_is_read_in_halves() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    seed(&mut runtime, &mut receiver).await?;
    read_week(&mut runtime).await?;
    let first = next_command(&mut receiver).await?;
    let span = |command: &Value| {
        command["calendar_query"]["end_unix_ms"]
            .as_i64()
            .unwrap_or(0)
            - command["calendar_query"]["start_unix_ms"]
                .as_i64()
                .unwrap_or(0)
    };
    assert_eq!(span(&first), 7 * DAY_MS);
    reject(&mut runtime, &first, "calendar_read_expired").await?;
    let halved = next_command(&mut receiver).await?;
    assert_eq!(halved["calendar_query"]["start_unix_ms"], OCTOBER_MS);
    assert!(span(&halved) < 7 * DAY_MS, "{halved}");
    assert!(matches!(
        read_agenda(&mut runtime, week()),
        orbit_core::topics::Knowledge::Unknown { .. }
    ));
    Ok(())
}

#[tokio::test]
async fn privacy_stop_does_not_retire_permission_requests() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    seed(&mut runtime, &mut receiver).await?;
    {
        let fixture_value = serde_json::json!(true);
        crate::daemon::authority_fixture::set(&mut runtime.state, "privacy_stopped", fixture_value);
    }
    runtime.features.calendar.wanted_permission = true;
    runtime.pump_calendar();
    let command = receiver.try_recv().context("calendar request")?;
    assert_eq!(command["command"], "permissions");

    runtime.maintain_calendar();

    assert!(
        runtime.features.calendar.pending.is_some(),
        "{} was retired by privacy stop",
        command["command"]
    );
    assert!(
        receiver.try_recv().is_err(),
        "{} was disconnected by privacy stop",
        command["command"]
    );
    Ok(())
}

#[tokio::test]
async fn a_lost_window_reply_is_read_again_in_halves() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    seed(&mut runtime, &mut receiver).await?;
    read_week(&mut runtime).await?;
    let first = next_command(&mut receiver).await?;
    let span = |command: &Value| {
        command["calendar_query"]["end_unix_ms"]
            .as_i64()
            .unwrap_or(0)
            - command["calendar_query"]["start_unix_ms"]
                .as_i64()
                .unwrap_or(0)
    };
    assert_eq!(span(&first), 7 * DAY_MS);
    runtime
        .features
        .calendar
        .pending
        .as_mut()
        .context("pending window")?
        .started = Instant::now() - Duration::from_secs(8);

    runtime.maintain_calendar();

    assert_eq!(
        next_command(&mut receiver).await?["command"],
        "calendar_disconnect"
    );
    let halved = next_command(&mut receiver).await?;
    assert_eq!(halved["command"], "calendar_snapshot");
    assert_eq!(halved["calendar_query"]["start_unix_ms"], OCTOBER_MS);
    assert!(span(&halved) < 7 * DAY_MS, "{halved}");
    assert!(matches!(
        read_agenda(&mut runtime, week()),
        orbit_core::topics::Knowledge::Unknown { .. }
    ));
    Ok(())
}

/// Withdrawing the permission cuts off what was on screen.
#[tokio::test]
async fn withdrawing_the_permission_removes_the_agenda_on_screen() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    seed(&mut runtime, &mut receiver).await?;
    read_week(&mut runtime).await?;
    let read = next_command(&mut receiver).await?;
    ack(
        &mut runtime,
        &read,
        page(
            &read["calendar_query"],
            vec![event(1, OCTOBER_MS, "work")],
            1,
            "44444444-4444-4444-8444-444444444444",
        ),
    )
    .await?;
    assert!(runtime.features.calendar.complete.is_some());
    runtime
        .native_event(host_state(2, false, status("denied")))
        .await?;
    let view = runtime.features.calendar.view.clone();
    assert!(runtime.features.calendar.complete.is_none());
    assert_eq!(view.authorization, CalendarAuthorization::Denied);
    // And a new read is not even sent: the permission is known to be missing.
    while receiver.try_recv().is_ok() {}
    read_week(&mut runtime).await?;
    assert!(receiver.try_recv().is_err());
    let detail = read_failure(&mut runtime, agenda_request(week()))?;
    assert!(detail.contains("permission_denied"), "{detail}");
    assert!(detail.contains("calendar"), "{detail}");
    Ok(())
}

/// The agenda arrives by itself the moment the permission does.
#[tokio::test]
async fn granting_the_permission_reads_the_agenda_without_being_asked() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    runtime
        .native_event(json!({"event":"ready","capabilities":["calendar"]}))
        .await?;
    runtime
        .native_event(host_state(2, false, status("not_determined")))
        .await?;
    while receiver.try_recv().is_ok() {}
    read_week(&mut runtime).await?;
    assert!(
        receiver.try_recv().is_err(),
        "without the permission nothing is read"
    );
    runtime
        .native_event(host_state(2, false, status("full_access")))
        .await?;
    let read = next_command(&mut receiver).await?;
    assert_eq!(read["command"], "calendar_snapshot");
    Ok(())
}

/// An unknown permission is neither: the read is asked of the helper, which
/// knows, instead of being refused or shown as denied.
#[tokio::test]
async fn an_unknown_permission_is_not_treated_as_denied() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    runtime
        .native_event(json!({"event":"ready","capabilities":["calendar"]}))
        .await?;
    runtime
        .native_event(json!({"event":"state","native_instance_id":HOST,"privacy_generation":2,"privacy_stopped":false}))
        .await?;
    // With the permission unknown the daemon still lists: the helper knows.
    let listing = next_command(&mut receiver).await?;
    assert_eq!(listing["command"], "calendar_sources");
    ack(&mut runtime, &listing, calendars_page(2)).await?;
    assert_eq!(
        runtime.features.calendar.view.authorization,
        CalendarAuthorization::Unknown
    );
    read_week(&mut runtime).await?;
    assert_eq!(
        next_command(&mut receiver).await?["command"],
        "calendar_snapshot"
    );
    assert!(matches!(
        read_agenda(&mut runtime, week()),
        orbit_core::topics::Knowledge::Unknown { .. }
    ));
    Ok(())
}

/// No count of calendars is refused; the list is joined from pages.
#[tokio::test]
async fn the_calendar_list_is_joined_whatever_its_length() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    seed(&mut runtime, &mut receiver).await?;
    // The store changed: the list is read again, by itself.
    runtime
        .native_event(json!({"event":"calendar_invalidated","code":"calendar_invalidated"}))
        .await?;
    let first = next_command(&mut receiver).await?;
    assert_eq!(first["command"], "calendar_sources");
    let mut page_one = calendars_page(300);
    let all = page_one["calendars"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    page_one["calendars"] = json!(all[..200]);
    page_one["next_after_index"] = json!(200);
    ack(&mut runtime, &first, page_one).await?;
    let second = next_command(&mut receiver).await?;
    assert_eq!(second["calendar_page"]["after_index"], 200);
    let mut page_two = calendars_page(300);
    page_two["calendars"] = json!(all[200..]);
    page_two["after_index"] = json!(200);
    ack(&mut runtime, &second, page_two).await?;
    assert_eq!(runtime.state.capabilities().calendars, Some(300));
    Ok(())
}

/// Obsolete helper answers do not consume the read in flight, and a calendar
/// answer never stops an unrelated recording.
#[tokio::test]
async fn obsolete_answers_do_not_consume_a_new_read_or_stop_audio() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    seed(&mut runtime, &mut receiver).await?;
    runtime
        .command(Command::MeetingStart {
            title: "Synthetic manual recording".into(),
            audio: orbit_core::protocol::MeetingAudio::MicrophoneOnly,
            duration_minutes: 15,
            event: None,
        })
        .await?;
    let audio = receiver.try_recv()?;
    assert_eq!(audio["command"], "audio_start");
    ack(
        &mut runtime,
        &audio,
        json!({"stream_id":audio["stream_id"]}),
    )
    .await?;
    let wake = receiver.try_recv()?;
    assert_eq!(wake["command"], "audio_wake_phrase");
    assert_eq!(wake["stream_id"], audio["stream_id"]);
    assert_eq!(wake["enabled"], true);
    ack(
        &mut runtime,
        &wake,
        json!({"stream_id":audio["stream_id"],"enabled":true}),
    )
    .await?;
    read_week(&mut runtime).await?;
    let current = next_command(&mut receiver).await?;
    for ok in [true, false] {
        runtime
            .native_event(
                json!({"event":"response","id":"calendar-local-obsolete","ok":ok,
                "result":{},"error":{"code":"calendar_read_failed","message":"untrusted"}}),
            )
            .await?;
    }
    assert_eq!(
        runtime
            .features
            .calendar
            .pending
            .as_ref()
            .map(|p| p.id.as_str()),
        current["id"].as_str()
    );
    assert!(!runtime.state.privacy_stopped());
    assert_eq!(
        runtime
            .state
            .durable
            .recording
            .as_ref()
            .map(|recording| &recording.phase),
        Some(&orbit_core::CapturePhase::Recording)
    );
    Ok(())
}

/// A lost answer ends by name on the daemon's own tick, and the helper is told
/// to retire it; the late answer changes nothing.
#[tokio::test]
async fn a_lost_answer_ends_on_the_tick_and_its_late_arrival_is_ignored() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    seed(&mut runtime, &mut receiver).await?;
    runtime.features.calendar.screen_range = Some(week());
    read_week(&mut runtime).await?;
    let read = next_command(&mut receiver).await?;
    runtime
        .features
        .calendar
        .pending
        .as_mut()
        .context("pending")?
        .started = Instant::now() - Duration::from_secs(8);
    runtime.maintain_calendar();
    assert_eq!(receiver.try_recv()?["command"], "calendar_disconnect");
    ack(
        &mut runtime,
        &read,
        page(
            &read["calendar_query"],
            vec![event(1, OCTOBER_MS, "work")],
            1,
            "44444444-4444-4444-8444-444444444444",
        ),
    )
    .await?;
    assert!(runtime.features.calendar.complete.is_none());
    Ok(())
}

/// The Mac cutting its capture retires the agenda like
/// any change of epoch, and says what happened: it is not a privacy stop.
#[tokio::test]
async fn a_capture_cut_retires_the_agenda_without_calling_it_a_privacy_stop() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    seed(&mut runtime, &mut receiver).await?;
    read_week(&mut runtime).await?;
    let read = next_command(&mut receiver).await?;
    ack(
        &mut runtime,
        &read,
        page(
            &read["calendar_query"],
            vec![event(1, OCTOBER_MS, "work")],
            1,
            "44444444-4444-4444-8444-444444444444",
        ),
    )
    .await?;
    assert!(runtime.features.calendar.complete.is_some());
    runtime
        .native_event(
            json!({"event":"capture_stopped","reason":"session_inactive",
            "privacy_stopped":false,"drain_complete":false}),
        )
        .await?;
    assert!(runtime.features.calendar.complete.is_none());
    let detail = read_failure(&mut runtime, agenda_request(week()))?;
    assert!(
        serde_json::from_str::<Value>(&detail)?["cause"]["kind"] == "capture_stopped",
        "{detail}"
    );
    assert!(
        serde_json::from_str::<Value>(&detail)?["code"] == "read_interrupted",
        "{detail}"
    );
    assert!(!runtime.state.privacy_stopped());
    Ok(())
}

/// Privacy stop retires the read in flight and what is on screen.
#[tokio::test]
async fn privacy_stop_retires_the_read_and_the_agenda_on_screen() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    seed(&mut runtime, &mut receiver).await?;
    read_week(&mut runtime).await?;
    let read = next_command(&mut receiver).await?;
    ack(
        &mut runtime,
        &read,
        page(
            &read["calendar_query"],
            vec![event(1, OCTOBER_MS, "work")],
            1,
            "44444444-4444-4444-8444-444444444444",
        ),
    )
    .await?;
    assert!(runtime.features.calendar.complete.is_some());
    runtime
        .native_event(host_state(2, true, status("full_access")))
        .await?;
    assert!(runtime.features.calendar.complete.is_none());
    Ok(())
}

#[tokio::test]
async fn agenda_read_preserves_the_native_contract_failure() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    seed(&mut runtime, &mut receiver).await?;
    read_week(&mut runtime).await?;
    let read = next_command(&mut receiver).await?;
    let mut invalid = page(
        &read["calendar_query"],
        vec![],
        0,
        "44444444-4444-4444-8444-444444444444",
    );
    invalid["total"] = json!(-1);
    ack(&mut runtime, &read, invalid).await?;
    let answer = super::super::reads::dispatch(
        &mut runtime,
        orbit_core::reads::ReadRequest::AgendaRange {
            start_ms: OCTOBER_MS,
            end_ms: OCTOBER_MS + 7 * DAY_MS,
            cursor: None,
        },
        1,
    );
    let text = serde_json::to_string(&answer)?;
    assert!(
        matches!(answer, orbit_core::topics::Knowledge::Absent { .. }),
        "{text}"
    );
    let fields = serde_json::to_value(&answer)?;
    assert_eq!(fields["refusal"]["code"], "helper_reply_invalid");
    assert_eq!(fields["refusal"]["field"], "total");
    assert_eq!(fields["refusal"]["received_type"], "number");
    let orbit_core::topics::Knowledge::Absent { refusal } = &answer else {
        anyhow::bail!("expected native failure");
    };
    assert_eq!(
        read_failure(&mut runtime, agenda_request(week()))?,
        serde_json::to_string(refusal)?
    );
    let other = CalendarWindow {
        start_unix_ms: OCTOBER_MS + DAY_MS,
        end_unix_ms: OCTOBER_MS + 2 * DAY_MS,
    };
    assert!(matches!(
        read_agenda(&mut runtime, other),
        orbit_core::topics::Knowledge::Unknown { .. }
    ));
    read_week(&mut runtime).await?;
    assert!(matches!(
        read_agenda(&mut runtime, week()),
        orbit_core::topics::Knowledge::Unknown { .. }
    ));
    let retry = next_command(&mut receiver).await?;
    ack(
        &mut runtime,
        &retry,
        page(
            &retry["calendar_query"],
            vec![],
            0,
            "44444444-4444-4444-8444-444444444444",
        ),
    )
    .await?;
    assert!(matches!(
        read_agenda(&mut runtime, week()),
        orbit_core::topics::Knowledge::Present { .. }
    ));
    Ok(())
}

#[tokio::test]
async fn agenda_read_preserves_a_lost_reply_deadline_and_elapsed_time() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    seed(&mut runtime, &mut receiver).await?;
    let range = CalendarWindow {
        start_unix_ms: OCTOBER_MS,
        end_unix_ms: OCTOBER_MS + MIN_WINDOW_MS,
    };
    runtime
        .command(Command::Calendar {
            request: Request::Read { window: range },
        })
        .await?;
    let _read = next_command(&mut receiver).await?;
    runtime
        .features
        .calendar
        .pending
        .as_mut()
        .context("pending window")?
        .started = Instant::now() - Duration::from_secs(8);
    runtime.maintain_calendar();
    let answer = super::super::reads::dispatch(
        &mut runtime,
        orbit_core::reads::ReadRequest::AgendaRange {
            start_ms: range.start_unix_ms,
            end_ms: range.end_unix_ms,
            cursor: None,
        },
        1,
    );
    let text = serde_json::to_string(&answer)?;
    assert!(
        matches!(answer, orbit_core::topics::Knowledge::Absent { .. }),
        "{text}"
    );
    let fields = serde_json::to_value(&answer)?;
    assert_eq!(fields["refusal"]["code"], "timeout");
    assert_eq!(fields["refusal"]["deadline_ms"], 7000);
    let waited = fields["refusal"]["waited_ms"]
        .as_u64()
        .context("elapsed milliseconds")?;
    assert!(waited >= 8_000, "{text}");
    Ok(())
}

#[tokio::test]
async fn a_rearm_does_not_keep_the_stop_as_the_agenda_answer() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    seed(&mut runtime, &mut receiver).await?;
    read_week(&mut runtime).await?;
    let read = next_command(&mut receiver).await?;
    ack(
        &mut runtime,
        &read,
        page(
            &read["calendar_query"],
            vec![],
            0,
            "44444444-4444-4444-8444-444444444444",
        ),
    )
    .await?;
    assert!(matches!(
        read_agenda(&mut runtime, week()),
        orbit_core::topics::Knowledge::Present { .. }
    ));
    runtime
        .privacy_stop(orbit_core::privacy_stop::UserStop::Order)
        .await;
    assert!(runtime.state.privacy_stopped());
    assert!(matches!(
        read_agenda(&mut runtime, week()),
        orbit_core::topics::Knowledge::Absent { .. }
    ));
    let mut stop = None;
    while let Ok(command) = receiver.try_recv() {
        if command["command"] == "privacy_stop" {
            stop = Some(command);
        }
    }
    let stop = stop.context("the helper was not told to stop")?;
    runtime
        .native_event(json!({"event":"response", "id":stop["id"], "ok":true,
        "result":{"privacy_stopped":true,"drain_complete":true}}))
        .await?;
    runtime
        .native_event(json!({"event":"local_rearm","privacy_stopped":false}))
        .await?;
    runtime
        .native_event(host_state(2, false, status("full_access")))
        .await?;
    assert!(!runtime.state.privacy_stopped());
    let after = read_agenda(&mut runtime, week());
    assert!(
        matches!(
            after,
            orbit_core::topics::Knowledge::Unknown {
                why: orbit_core::topics::Pending::NotObservedYet,
            }
        ),
        "{after:?}"
    );
    Ok(())
}

#[tokio::test]
async fn a_permission_grant_does_not_keep_the_revocation_as_the_agenda_answer() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    runtime
        .native_event(json!({"event":"ready","capabilities":["calendar"]}))
        .await?;
    runtime
        .native_event(host_state(2, false, status("full_access")))
        .await?;
    assert_eq!(
        next_command(&mut receiver).await?["command"],
        "calendar_sources"
    );
    // Permission changes while the initial inventory is still with EventKit,
    // before any screen range has been requested.
    runtime
        .native_event(host_state(2, false, status("denied")))
        .await?;
    let detail = read_failure(&mut runtime, agenda_request(week()))?;
    assert!(
        serde_json::from_str::<Value>(&detail)?["code"] == "permission_denied"
            && serde_json::from_str::<Value>(&detail)?["permission"] == "calendar",
        "{detail}"
    );
    runtime
        .native_event(host_state(2, false, status("full_access")))
        .await?;
    assert_eq!(
        runtime.features.calendar.view.authorization,
        CalendarAuthorization::FullAccess
    );
    let after = read_agenda(&mut runtime, week());
    assert!(
        matches!(
            after,
            orbit_core::topics::Knowledge::Unknown {
                why: orbit_core::topics::Pending::NotObservedYet,
            }
        ),
        "{after:?}"
    );
    Ok(())
}

fn agenda_request(window: CalendarWindow) -> orbit_core::reads::ReadRequest {
    orbit_core::reads::ReadRequest::AgendaRange {
        start_ms: window.start_unix_ms,
        end_ms: window.end_unix_ms,
        cursor: None,
    }
}

fn read_agenda(
    runtime: &mut Runtime,
    window: CalendarWindow,
) -> orbit_core::topics::Knowledge<orbit_core::topics::Page<Value>> {
    super::super::reads::dispatch(runtime, agenda_request(window), 1)
}

fn read_failure(runtime: &mut Runtime, request: orbit_core::reads::ReadRequest) -> Result<String> {
    let answer = super::super::reads::dispatch(runtime, request, 1);
    let orbit_core::topics::Knowledge::Absent { refusal } = answer else {
        anyhow::bail!("expected read refusal: {answer:?}");
    };
    Ok(serde_json::to_string(&refusal)?)
}
