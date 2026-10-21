//! P9-21 criterion 4, through the daemon's real path to a synthetic helper:
//! a write reaches the helper with its shape, its answer comes back typed, a
//! refusal keeps its cause and numbers, and a lost answer is
//! `calendar_write_outcome_unknown` — the agenda is read again and the write
//! is never sent a second time. No EventKit, TCC or account.
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use orbit_core::{
    calendar::*,
    calendar_control::{
        CalendarCommand as Request, CalendarWriteOperation, CalendarWriteStage, CalendarWriteView,
    },
    protocol::Command,
};
use serde_json::{Value, json};
use tokio::sync::mpsc;

use super::tests::{
    DAY_MS, HOST, OCTOBER_MS, ack, event, host_state, next_command, page, seed, status,
};
use super::{writes::WRITE_RECORDS, *};
use crate::daemon::audio::tests::fixture;

const HOUR: i64 = 3_600_000;
/// 1 October 2026, 10:00 UTC.
const START: i64 = OCTOBER_MS + 10 * HOUR;
const WRITE_ID: &str = "77777777-7777-4777-8777-777777777777";

fn move_request(request_id: &str, span: CalendarSpan) -> Request {
    Request::Save {
        request_id: request_id.into(),
        save: CalendarSave::Change(Box::new(CalendarEventChange {
            calendar_item_id: "item-1".into(),
            occurrence_start_ms: START,
            span,
            changes: CalendarEventChanges {
                start_ms: Some(START + HOUR),
                end_ms: Some(START + 2 * HOUR),
                ..CalendarEventChanges::default()
            },
        })),
    }
}

fn create_request(request_id: &str) -> Request {
    Request::Save {
        request_id: request_id.into(),
        save: CalendarSave::Create(Box::new(CalendarEventDraft {
            calendar_id: None,
            title: "Café con Ada".into(),
            start_ms: START,
            end_ms: START + HOUR,
            all_day: false,
            time_zone: None,
            location: Some("Sala 3".into()),
            notes: None,
            url: None,
            availability: Some(CalendarAvailability::Busy),
            recurrence: None,
            alarms_minutes: Some(vec![10]),
        })),
    }
}

fn delete_request(request_id: &str) -> Request {
    Request::Delete {
        request_id: request_id.into(),
        delete: CalendarDelete {
            calendar_item_id: "item-1".into(),
            occurrence_start_ms: START,
            span: CalendarSpan::This,
        },
    }
}

/// The helper's answer to a save: the occurrence as it is now.
fn saved(start: i64) -> Value {
    let mut occurrence = event(1, start, "calendar-0000");
    occurrence["original_occurrence_unix_ms"] = json!(START);
    occurrence["availability"] = json!("busy");
    occurrence["can_edit"] = json!(true);
    json!({"write_id": WRITE_ID, "native_instance_id": HOST, "privacy_generation": 2,
        "system_timezone": "UTC", "occurrence": occurrence})
}

fn write(runtime: &Runtime, request_id: &str) -> Result<CalendarWriteView> {
    runtime
        .features
        .calendar
        .view
        .clone()
        .writes
        .iter()
        .find(|write| write.request_id == request_id)
        .cloned()
        .context("la escritura no está en la vista")
}

async fn ask(runtime: &mut Runtime, request: Request) -> Result<()> {
    runtime.command(Command::Calendar { request }).await
}

/// Every command the helper received since the last look, by name.
fn drained(receiver: &mut mpsc::Receiver<Value>) -> Vec<String> {
    let mut names = Vec::new();
    while let Ok(command) = receiver.try_recv() {
        names.push(command["command"].as_str().unwrap_or_default().to_owned());
    }
    names
}

/// A queue that is full refuses the next write by that cause, with the
/// count and the limit, and sends nothing.
#[tokio::test]
async fn p9_21_a_full_queue_refuses_by_name_with_its_numbers() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    seed(&mut runtime, &mut receiver).await?;
    for index in 0..WRITE_RECORDS {
        ask(&mut runtime, delete_request(&format!("queued-{index}"))).await?;
    }
    let error = ask(&mut runtime, delete_request("one-too-many"))
        .await
        .err()
        .context("una escritura de más se aceptó")?;
    assert_eq!(
        error.downcast_ref::<CalendarWriteRefusal>(),
        Some(&CalendarWriteRefusal::QueueFull {
            unfinished: WRITE_RECORDS as u64,
            limit: WRITE_RECORDS as u64,
        })
    );
    assert!(error.to_string().contains("16"), "{error}");
    let sent = drained(&mut receiver)
        .iter()
        .filter(|name| *name == "calendar_delete")
        .count();
    assert_eq!(sent, 1, "only the first write holds the slot");
    Ok(())
}

/// Seventeen saves of events with long titles: only the newest record keeps
/// its occurrence; an occurrence past `MAX_WRITE_OCCURRENCE_BYTES` stays out of
/// the record, which says its size.
#[tokio::test]
async fn p9_21_only_the_latest_saved_write_retains_a_bounded_occurrence() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    seed(&mut runtime, &mut receiver).await?;
    let long_title = "x".repeat(90_000);
    for index in 0..=WRITE_RECORDS {
        let request_id = format!("move-{index}");
        ask(&mut runtime, move_request(&request_id, CalendarSpan::This)).await?;
        let command = next_command(&mut receiver).await?;
        let mut answer = saved(START + HOUR);
        answer["occurrence"]["title"] = json!(long_title);
        ack(&mut runtime, &command, answer).await?;
        assert_eq!(
            write(&runtime, &request_id)?.stage,
            CalendarWriteStage::Saved
        );
    }
    let writes = runtime.features.calendar.view.clone().writes;
    assert_eq!(writes.len(), WRITE_RECORDS);
    let carrying: Vec<&str> = writes
        .iter()
        .filter(|write| write.occurrence.is_some())
        .map(|write| write.request_id.as_str())
        .collect();
    assert_eq!(carrying, vec![format!("move-{WRITE_RECORDS}").as_str()]);
    ask(&mut runtime, move_request("huge", CalendarSpan::This)).await?;
    let command = next_command(&mut receiver).await?;
    let mut answer = saved(START + HOUR);
    answer["occurrence"]["title"] = json!("t".repeat(MAX_TITLE_BYTES));
    answer["occurrence"]["event_url"] =
        json!(format!("https://example.test/{}", "u".repeat(60_000)));
    answer["occurrence"]["event_url_status"] = json!("provided");
    ack(&mut runtime, &command, answer).await?;
    let huge = write(&runtime, "huge")?;
    assert_eq!(huge.stage, CalendarWriteStage::Saved);
    assert!(huge.occurrence.is_none());
    assert!(
        huge.detail
            .contains(&super::writes::MAX_WRITE_OCCURRENCE_BYTES.to_string()),
        "{}",
        huge.detail
    );
    Ok(())
}

/// The next agenda command the helper receives, answering the calendar
/// listings the daemon asks for on the way and skipping its retirements.
async fn next_window(runtime: &mut Runtime, receiver: &mut mpsc::Receiver<Value>) -> Result<Value> {
    loop {
        let command = next_command(receiver).await?;
        if command["command"] == "calendar_sources" {
            ack(runtime, &command, super::tests::calendars_page(3)).await?;
            continue;
        }
        // Best effort, answered by nobody the daemon waits on.
        if command["command"] == "calendar_disconnect" {
            continue;
        }
        return Ok(command);
    }
}

/// A store that keeps changing under a long read ends it by name: restarts
/// count for the whole read, not per window, so it cannot start over for ever.
#[tokio::test]
async fn p9_21_a_read_that_keeps_restarting_ends_by_name() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    seed(&mut runtime, &mut receiver).await?;
    let range = CalendarWindow {
        start_unix_ms: OCTOBER_MS,
        end_unix_ms: OCTOBER_MS + 62 * DAY_MS,
    };
    runtime
        .command(Command::Calendar {
            request: Request::Read { window: range },
        })
        .await?;
    for cycle in 0..8 {
        let first = next_window(&mut runtime, &mut receiver).await?;
        assert_eq!(
            first["calendar_query"]["start_unix_ms"], OCTOBER_MS,
            "cycle {cycle}"
        );
        ack(
            &mut runtime,
            &first,
            page(
                &first["calendar_query"],
                vec![],
                0,
                "44444444-4444-4444-8444-444444444441",
            ),
        )
        .await?;
        let second = next_window(&mut runtime, &mut receiver).await?;
        runtime
            .native_event(json!({"event":"calendar_invalidated","code":"calendar_invalidated"}))
            .await?;
        ack(
            &mut runtime,
            &second,
            page(
                &second["calendar_query"],
                vec![],
                0,
                "44444444-4444-4444-8444-444444444442",
            ),
        )
        .await?;
        let answer = super::super::reads::dispatch(
            &mut runtime,
            orbit_core::reads::ReadRequest::AgendaRange {
                start_ms: range.start_unix_ms,
                end_ms: range.end_unix_ms,
                cursor: None,
            },
            1,
        );
        if let orbit_core::topics::Knowledge::Absent { refusal } = answer {
            assert_eq!(cycle, usize::from(WINDOW_RESTARTS), "ended after the bound");
            let detail = serde_json::to_string(&refusal)?;
            let fields = serde_json::from_str::<Value>(&detail)?;
            assert_eq!(fields["code"], "read_interrupted");
            assert_eq!(fields["cause"]["kind"], "calendar_changed");
            assert_eq!(fields["cause"]["changes"], 4);
            assert_eq!(fields["cause"]["max_restarts"], 3);
            assert!(runtime.features.calendar.screen.is_none());
            return Ok(());
        }
    }
    anyhow::bail!("la lectura se reinició ocho veces sin terminar")
}

#[tokio::test]
async fn p9_21_a_move_reaches_the_helper_and_its_answer_comes_back_typed() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    seed(&mut runtime, &mut receiver).await?;
    for (request_id, span) in [("move-this", "this"), ("move-future", "future")] {
        let typed = if span == "this" {
            CalendarSpan::This
        } else {
            CalendarSpan::Future
        };
        ask(&mut runtime, move_request(request_id, typed)).await?;
        let command = next_command(&mut receiver).await?;
        assert_eq!(command["command"], "calendar_save");
        let change = &command["calendar_save"]["change"];
        assert_eq!(change["calendar_item_id"], "item-1");
        assert_eq!(change["occurrence_start_ms"], START);
        assert_eq!(change["span"], span);
        assert_eq!(
            change["changes"],
            json!({"start_ms": START + HOUR, "end_ms": START + 2 * HOUR})
        );
        assert_eq!(
            write(&runtime, request_id)?.stage,
            CalendarWriteStage::Writing
        );
        ack(&mut runtime, &command, saved(START + HOUR)).await?;
        let outcome = write(&runtime, request_id)?;
        assert_eq!(outcome.operation, CalendarWriteOperation::Update);
        assert_eq!(outcome.stage, CalendarWriteStage::Saved);
        let occurrence = outcome.occurrence.context("sin la ocurrencia resultante")?;
        assert_eq!(occurrence.start_unix_ms, START + HOUR);
        assert_eq!(occurrence.calendar_item_id, "item-1");
        assert!(outcome.refusal.is_none());
    }
    Ok(())
}

#[tokio::test]
async fn p9_21_a_create_and_a_delete_reach_the_helper_with_their_shapes() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    seed(&mut runtime, &mut receiver).await?;
    ask(&mut runtime, create_request("create-1")).await?;
    let command = next_command(&mut receiver).await?;
    assert_eq!(command["command"], "calendar_save");
    let create = &command["calendar_save"]["create"];
    assert!(
        create.get("calendar_id").is_none(),
        "sin calendario: el que macOS usa por defecto"
    );
    assert_eq!(create["title"], "Café con Ada");
    assert_eq!(create["alarms_minutes"], json!([10]));
    ack(&mut runtime, &command, saved(START)).await?;
    let created = write(&runtime, "create-1")?;
    assert_eq!(
        (created.operation, created.stage),
        (CalendarWriteOperation::Create, CalendarWriteStage::Saved)
    );

    ask(&mut runtime, delete_request("delete-1")).await?;
    let command = next_command(&mut receiver).await?;
    assert_eq!(command["command"], "calendar_delete");
    assert_eq!(
        command["calendar_delete"],
        json!({"calendar_item_id": "item-1", "occurrence_start_ms": START, "span": "this"})
    );
    ack(
        &mut runtime,
        &command,
        json!({"write_id": WRITE_ID, "native_instance_id": HOST, "privacy_generation": 2,
            "calendar_item_id": "item-1", "occurrence_start_ms": START, "span": "this"}),
    )
    .await?;
    let deleted = write(&runtime, "delete-1")?;
    assert_eq!(
        (deleted.operation, deleted.stage),
        (CalendarWriteOperation::Delete, CalendarWriteStage::Deleted)
    );
    Ok(())
}

/// A refusal keeps its code and numbers to whoever asked, and is not retried.
#[tokio::test]
async fn p9_21_a_refusal_comes_back_by_name_with_its_numbers() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    seed(&mut runtime, &mut receiver).await?;
    ask(&mut runtime, move_request("move-1", CalendarSpan::This)).await?;
    let command = next_command(&mut receiver).await?;
    runtime
        .native_event(json!({"event":"response","id":command["id"],"ok":false,
            "error":{"code":"calendar_not_editable","message":"organizado por otra persona",
                "detail":{"reason":"organized_by_someone_else","calendar_id":"calendar-0000"}}}))
        .await?;
    let outcome = write(&runtime, "move-1")?;
    assert_eq!(outcome.stage, CalendarWriteStage::Refused);
    assert_eq!(
        outcome.refusal,
        Some(CalendarWriteRefusal::NotEditable {
            reason: CalendarNotEditable::OrganizedBySomeoneElse,
            calendar_id: "calendar-0000".into(),
        })
    );
    assert!(
        outcome.detail.contains("otra persona"),
        "{}",
        outcome.detail
    );
    runtime.maintain_calendar();
    assert!(!drained(&mut receiver).contains(&"calendar_save".to_owned()));
    Ok(())
}

/// A write Orbit can already tell is wrong never reaches the helper, and the
/// refusal names the field and its numbers.
#[tokio::test]
async fn p9_21_an_invalid_write_is_refused_before_it_is_sent() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    seed(&mut runtime, &mut receiver).await?;
    let backwards = Request::Save {
        request_id: "backwards".into(),
        save: CalendarSave::Change(Box::new(CalendarEventChange {
            calendar_item_id: "item-1".into(),
            occurrence_start_ms: START,
            span: CalendarSpan::This,
            changes: CalendarEventChanges {
                start_ms: Some(START + HOUR),
                end_ms: Some(START),
                ..CalendarEventChanges::default()
            },
        })),
    };
    let error = ask(&mut runtime, backwards)
        .await
        .err()
        .context("una escritura al revés se aceptó")?;
    assert!(error.to_string().contains(&START.to_string()), "{error}");
    let outcome = write(&runtime, "backwards")?;
    assert_eq!(outcome.stage, CalendarWriteStage::Refused);
    let Some(CalendarWriteRefusal::InvalidChange(change)) = outcome.refusal else {
        anyhow::bail!("sin causa: {:?}", outcome.refusal);
    };
    assert_eq!(change.problem, CalendarChangeProblem::EndBeforeStart);
    assert_eq!(
        (change.start_ms, change.end_ms),
        (Some(START + HOUR), Some(START))
    );
    assert!(!drained(&mut receiver).contains(&"calendar_save".to_owned()));
    Ok(())
}

/// The answer never came: the outcome is unknown, by name; the agenda on
/// screen is read again; the write is not sent a second time, and its late
/// answer does not turn the unknown into a fact.
#[tokio::test]
async fn p9_21_a_lost_answer_is_outcome_unknown_and_never_sent_twice() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    seed(&mut runtime, &mut receiver).await?;
    let week = CalendarWindow {
        start_unix_ms: OCTOBER_MS,
        end_unix_ms: OCTOBER_MS + 7 * DAY_MS,
    };
    runtime
        .command(Command::Calendar {
            request: Request::Read { window: week },
        })
        .await?;
    let read = next_command(&mut receiver).await?;
    ack(
        &mut runtime,
        &read,
        page(
            &read["calendar_query"],
            vec![event(1, START, "calendar-0000")],
            1,
            "44444444-4444-4444-8444-444444444444",
        ),
    )
    .await?;
    drained(&mut receiver);

    ask(&mut runtime, move_request("lost", CalendarSpan::This)).await?;
    let command = next_command(&mut receiver).await?;
    assert_eq!(command["command"], "calendar_save");
    runtime
        .features
        .calendar
        .pending
        .as_mut()
        .context("pending")?
        .started = Instant::now() - Duration::from_secs(8);
    runtime.maintain_calendar();
    let outcome = write(&runtime, "lost")?;
    assert_eq!(outcome.stage, CalendarWriteStage::OutcomeUnknown);
    assert_eq!(
        outcome.refusal,
        Some(CalendarWriteRefusal::OutcomeUnknown {
            cause: "calendar_reply_timeout".into()
        })
    );
    assert!(
        outcome.detail.contains("no lo repite"),
        "{}",
        outcome.detail
    );
    let sent = drained(&mut receiver);
    assert!(
        sent.contains(&"calendar_snapshot".to_owned()),
        "la agenda no se volvió a leer: {sent:?}"
    );
    assert!(!sent.contains(&"calendar_save".to_owned()), "{sent:?}");

    // The late answer is about a write that is already unknown: ignored.
    ack(&mut runtime, &command, saved(START + HOUR)).await?;
    assert_eq!(
        write(&runtime, "lost")?.stage,
        CalendarWriteStage::OutcomeUnknown
    );
    for _ in 0..3 {
        runtime.maintain_calendar();
    }
    assert!(!drained(&mut receiver).contains(&"calendar_save".to_owned()));
    Ok(())
}

/// The helper itself can say the outcome is unknown (its EventKit deadline).
#[tokio::test]
async fn p9_21_the_helpers_own_unknown_outcome_keeps_its_cause() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    seed(&mut runtime, &mut receiver).await?;
    ask(&mut runtime, delete_request("slow")).await?;
    let command = next_command(&mut receiver).await?;
    runtime
        .native_event(json!({"event":"response","id":command["id"],"ok":false,
            "error":{"code":"calendar_write_outcome_unknown","message":"plazo",
                "detail":{"cause":"write_deadline"}}}))
        .await?;
    let outcome = write(&runtime, "slow")?;
    assert_eq!(outcome.stage, CalendarWriteStage::OutcomeUnknown);
    assert_eq!(
        outcome.refusal,
        Some(CalendarWriteRefusal::OutcomeUnknown {
            cause: "write_deadline".into()
        })
    );
    Ok(())
}

/// A helper that restarts while a write is with EventKit leaves its outcome
/// unknown; the new helper is not sent the write again.
#[tokio::test]
async fn p9_21_a_restart_during_a_write_is_unknown_and_not_resent() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    seed(&mut runtime, &mut receiver).await?;
    ask(&mut runtime, move_request("restart", CalendarSpan::Future)).await?;
    let _command = next_command(&mut receiver).await?;
    runtime
        .native_event(json!({"event":"ready","capabilities":["calendar"]}))
        .await?;
    runtime
        .native_event(host_state(3, false, status("full_access")))
        .await?;
    let outcome = write(&runtime, "restart")?;
    assert_eq!(outcome.stage, CalendarWriteStage::OutcomeUnknown);
    assert_eq!(
        outcome.refusal,
        Some(CalendarWriteRefusal::OutcomeUnknown {
            cause: "calendar_helper_restarted".into()
        })
    );
    runtime.maintain_calendar();
    assert!(!drained(&mut receiver).contains(&"calendar_save".to_owned()));
    Ok(())
}

/// Her privacy stop refuses a write that had not been sent — the store is
/// known to be unchanged — and refuses new ones without sending them.
#[tokio::test]
async fn p9_21_privacy_stop_refuses_unsent_writes_by_name() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    seed(&mut runtime, &mut receiver).await?;
    ask(&mut runtime, move_request("first", CalendarSpan::This)).await?;
    let _first = next_command(&mut receiver).await?;
    // Waits for the single EventKit slot.
    ask(&mut runtime, delete_request("queued")).await?;
    assert_eq!(write(&runtime, "queued")?.stage, CalendarWriteStage::Queued);
    runtime
        .privacy_stop(orbit_core::privacy_stop::MarsStop::Order)
        .await;
    assert_eq!(
        write(&runtime, "first")?.stage,
        CalendarWriteStage::OutcomeUnknown
    );
    let queued = write(&runtime, "queued")?;
    assert_eq!(queued.stage, CalendarWriteStage::Refused);
    assert_eq!(
        queued.refusal,
        Some(CalendarWriteRefusal::NotWritten {
            cause: "privacy_stopped".into()
        })
    );
    assert!(ask(&mut runtime, create_request("after")).await.is_err());
    assert_eq!(write(&runtime, "after")?.stage, CalendarWriteStage::Refused);
    let sent = drained(&mut receiver);
    assert!(!sent.contains(&"calendar_save".to_owned()), "{sent:?}");
    assert!(!sent.contains(&"calendar_delete".to_owned()), "{sent:?}");
    Ok(())
}

/// A write while the agenda on screen is being read window by window: the
/// store changed under the read, so the windows already joined are stale and
/// the read starts again from its first window — it never keeps old days.
#[tokio::test]
async fn p9_21_a_change_during_a_read_starts_the_read_again_from_its_first_window() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    seed(&mut runtime, &mut receiver).await?;
    let range = CalendarWindow {
        start_unix_ms: OCTOBER_MS,
        end_unix_ms: OCTOBER_MS + 62 * DAY_MS,
    };
    runtime
        .command(Command::Calendar {
            request: Request::Read { window: range },
        })
        .await?;
    let first = next_command(&mut receiver).await?;
    assert_eq!(first["calendar_query"]["start_unix_ms"], OCTOBER_MS);
    ack(
        &mut runtime,
        &first,
        page(
            &first["calendar_query"],
            vec![event(1, START, "calendar-0000")],
            1,
            "44444444-4444-4444-8444-444444444441",
        ),
    )
    .await?;
    let second = next_command(&mut receiver).await?;
    assert_eq!(
        second["calendar_query"]["start_unix_ms"],
        OCTOBER_MS + READ_WINDOW_MS
    );
    // The store changed (a write from Orbit, or anywhere) before the second
    // window answered.
    runtime
        .native_event(json!({"event":"calendar_invalidated","code":"calendar_invalidated"}))
        .await?;
    ack(
        &mut runtime,
        &second,
        page(
            &second["calendar_query"],
            vec![],
            0,
            "44444444-4444-4444-8444-444444444442",
        ),
    )
    .await?;
    let again = next_command(&mut receiver).await?;
    assert_eq!(again["command"], "calendar_snapshot");
    assert_eq!(
        again["calendar_query"]["start_unix_ms"], OCTOBER_MS,
        "the first window, read before the change, would have stayed on screen"
    );
    Ok(())
}
