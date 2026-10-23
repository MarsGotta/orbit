use super::*;
use crate::projection::test_support::*;

#[test]
fn projects_next_with_previous_series_context() {
    let mut state = briefing_fixture(vec![
        briefing_event(
            "daily-today",
            "Daily LedgerBox",
            at_local(9, 45),
            at_local(10, 0),
            &["alina.perez", "bruno.soler", "dario.campos"],
            Some("https://meet.google.com/sample-meet"),
        ),
        briefing_event(
            "investor-daily",
            "Daily customer portal",
            at_local(15, 0),
            at_local(15, 20),
            &["alina.perez"],
            None,
        ),
    ]);
    state.recordings = vec![
        recording_summary(
            "rec-daily-0915",
            "Daily LedgerBox 2026-09-15",
            at_local(9, 45) - 86_400_000,
        ),
        recording_summary(
            "rec-retro-0914",
            "[LedgerBox] Retrospectiva",
            at_local(9, 45) - 2 * 86_400_000,
        ),
    ];

    let morning = projected_briefing(&state, at_local(9, 0));
    assert_eq!(morning["summary"]["meetings_today"], 2);
    let next = &morning["next"];
    assert_eq!(next["event_id"], "observation-daily-today");
    assert_eq!(next["title"], "Daily LedgerBox");
    assert_eq!(next["start_ms"], at_local(9, 45));
    assert_eq!(next["end_ms"], at_local(10, 0));
    assert_eq!(next["url"], "https://meet.google.com/sample-meet");
    assert_eq!(next["series_key"], "daily ledgerbox");
    assert_eq!(next["attendees"][0]["name"], "alina.perez");
    assert_eq!(next["attendees"][0]["initial"], "A");
    assert_eq!(next["attendees"][1]["name"], "bruno.soler");
    assert_eq!(next["attendees"][1]["initial"], "B");
    assert_eq!(next["attendees"][2]["name"], "dario.campos");
    assert_eq!(next["previous"]["recording_id"], "rec-daily-0915");
    assert_eq!(next["recording"], "none");

    let evening = projected_briefing(&state, at_local(15, 30));
    assert_eq!(evening["next"], Value::Null);
}

#[test]
fn projects_focus_minutes_from_the_free_gaps_of_the_day() {
    let state = briefing_fixture(vec![
        briefing_event(
            "platform-office",
            "platform office",
            at_local(10, 0),
            at_local(10, 40),
            &["alina.perez"],
            None,
        ),
        briefing_event(
            "investor-daily",
            "Daily customer portal",
            at_local(15, 0),
            at_local(15, 20),
            &["alina.perez"],
            None,
        ),
    ]);

    let briefing = projected_briefing(&state, at_local(9, 0));
    assert_eq!(briefing["summary"]["focus_minutes"], 520);
}

// Every hour, "today", day cut and focus window of the
// projection is read on the user's clock, in Madrid. The instants are UTC and
// the comments give the wall clock in Madrid, as the tz database has it.

fn half_hour_event(event_id: &str, start_ms: i64) -> CalendarOccurrence {
    briefing_event(
        event_id,
        &format!("Reunión {event_id}"),
        start_ms,
        start_ms + HALF_HOUR_MS,
        &["alina.perez"],
        None,
    )
}

fn agenda_rows(briefing: &Value) -> Vec<(String, i64)> {
    briefing["agenda"]
        .as_array()
        .expect("the agenda is a list")
        .iter()
        .map(|row| {
            (
                row["event_id"].as_str().unwrap_or_default().to_owned(),
                row["start_ms"].as_i64().expect("agenda start"),
            )
        })
        .collect()
}

#[test]
fn today_starts_at_local_midnight() {
    let state = briefing_fixture(vec![
        half_hour_event("tuesday-late", UTC_2026_09_15_T2130Z),
        half_hour_event("wednesday-early", UTC_2026_09_15_T2300Z),
        half_hour_event("wednesday-ten", UTC_2026_09_16_T0800Z),
    ]);

    let briefing = projected_briefing(&state, UTC_2026_09_15_T2230Z);

    assert_eq!(
        agenda_rows(&briefing),
        vec![
            (
                "observation-wednesday-early".to_owned(),
                UTC_2026_09_15_T2300Z
            ),
            (
                "observation-wednesday-ten".to_owned(),
                UTC_2026_09_16_T0800Z
            ),
        ],
        "23:30 on Tuesday is yesterday; 01:00 on Wednesday is today"
    );
    assert_eq!(briefing["summary"]["meetings_today"], 2);
}

#[test]
fn the_focus_window_is_nine_to_seven_on_the_users_clock() {
    let state = briefing_fixture(vec![half_hour_event("nine", UTC_2026_09_16_T0700Z)]);

    let briefing = projected_briefing(&state, UTC_2026_09_16_T0500Z);

    // 09:00-19:00 is 600 minutes; the event and its 10-minute buffer take
    // 09:00-09:40.
    assert_eq!(briefing["summary"]["focus_minutes"], 560);
}

#[test]
fn the_day_the_clocks_go_back_has_twenty_five_hours() {
    let state = briefing_fixture(vec![
        half_hour_event("after-midnight", UTC_2026_10_24_T2230Z),
        half_hour_event("first-half-past-two", UTC_2026_10_25_T0030Z),
        half_hour_event("second-half-past-two", UTC_2026_10_25_T0130Z),
        half_hour_event("ten", UTC_2026_10_25_T0900Z),
        half_hour_event("monday", UTC_2026_10_25_T2330Z),
    ]);

    let briefing = projected_briefing(&state, UTC_2026_10_25_T2230Z);

    assert_eq!(
        agenda_rows(&briefing),
        vec![
            (
                "observation-after-midnight".to_owned(),
                UTC_2026_10_24_T2230Z
            ),
            (
                "observation-first-half-past-two".to_owned(),
                UTC_2026_10_25_T0030Z
            ),
            (
                "observation-second-half-past-two".to_owned(),
                UTC_2026_10_25_T0130Z
            ),
            ("observation-ten".to_owned(), UTC_2026_10_25_T0900Z),
        ],
        "Sunday runs from 00:00 CEST to 24:00 CET and 02:30 happens twice"
    );
    assert_eq!(briefing["summary"]["meetings_today"], 4);
    assert_eq!(briefing["summary"]["focus_minutes"], 560);
}

fn series_answer(before_ms: i64, previous: Value) -> crate::SeriesPreviousRead {
    crate::SeriesPreviousRead {
        calendar_item_id: "item-design-review".into(),
        before_ms,
        answer: Ok(serde_json::from_value(json!({
            "calendar_item_id": "item-design-review", "before_ms": before_ms,
            "previous": previous,
        }))
        .expect("series answer fixture")),
    }
}

fn design_review() -> BriefingFixture {
    let mut state = briefing_fixture(vec![
        briefing_event(
            "design-review",
            "Revisión de diseño",
            at_local(11, 0),
            at_local(11, 45),
            &["alina.perez", "bruno.soler"],
            None,
        ),
        briefing_event(
            "lunch",
            "Comida con Ribera",
            at_local(14, 0),
            at_local(15, 0),
            &["alina.perez"],
            None,
        ),
    ]);
    // Same title, another meeting: the title match the series replaces.
    state.recordings = vec![recording_summary(
        "rec-by-title",
        "Revisión de diseño",
        at_local(11, 0) - 86_400_000,
    )];
    state
}

/// «Pendiente de la última vez»: the tasks of the series' earlier note, done
/// or not, on the revision a task is marked done on; its note is the earlier
/// one of the card.
#[test]
fn the_next_meeting_brings_the_tasks_of_its_series_earlier_note() {
    let mut state = design_review();
    state.series = Some(series_answer(
        at_local(11, 0),
        json!({"recording_id": "rec-series", "title": "Revisión de diseño",
            "started_at_ms": at_local(11, 0) - 7 * 86_400_000,
            "note": {"state": "ready", "revision": 9, "short_summary": "Se cerró el flujo de alta.",
                "action_items": [
                    {"id": "a000000000001", "owner": "Lucía", "text": "Enviar el acta",
                        "due": "viernes", "done": false},
                    {"id": "a000000000002", "owner": null, "text": "Revisar los iconos",
                        "due": null, "done": true}]}}),
    ));
    let briefing = projected_briefing(&state, at_local(9, 0));
    let next = &briefing["next"];
    assert_eq!(next["previous"]["recording_id"], "rec-series");
    assert_eq!(next["previous"]["summary"], "Se cerró el flujo de alta.");
    assert_eq!(
        next["previous_actions"],
        json!({"state": "known", "recording_id": "rec-series", "note_revision": 9, "items": [
            {"id": "a000000000001", "owner": "Lucía", "text": "Enviar el acta",
                "due": "viernes", "done": false},
            {"id": "a000000000002", "owner": null, "text": "Revisar los iconos",
                "due": null, "done": true}]})
    );
    // Only the next meeting carries it.
    assert!(briefing["upcoming"][1].get("previous_actions").is_none());
}

/// No earlier meeting is a fact; an earlier note not written, a read for
/// another meeting, none yet, or a failed one are unknown, never «nothing».
#[test]
fn the_series_says_none_apart_from_not_known() {
    let mut state = design_review();
    state.series = Some(series_answer(at_local(11, 0), Value::Null));
    let next = projected_briefing(&state, at_local(9, 0))["next"].clone();
    assert_eq!(
        next["previous_actions"],
        json!({"state": "no_previous_meeting"})
    );
    // The title match still opens the note of that name.
    assert_eq!(next["previous"]["recording_id"], "rec-by-title");

    state.series = Some(series_answer(
        at_local(11, 0),
        json!({"recording_id": "rec-series", "title": "Revisión de diseño",
            "started_at_ms": at_local(11, 0) - 7 * 86_400_000,
            "note": {"state": "generating", "revision": 2, "short_summary": null,
                "action_items": null}}),
    ));
    let next = projected_briefing(&state, at_local(9, 0))["next"].clone();
    assert_eq!(
        next["previous_actions"],
        json!({"state": "note_not_written", "recording_id": "rec-series",
            "note_state": "generating"})
    );

    state.series = Some(series_answer(
        at_local(11, 0),
        json!({"recording_id": "rec-series", "title": "Revisión de diseño",
            "started_at_ms": at_local(11, 0) - 7 * 86_400_000, "note": null}),
    ));
    let next = projected_briefing(&state, at_local(9, 0))["next"].clone();
    assert_eq!(
        next["previous_actions"],
        json!({"state": "note_not_written", "recording_id": "rec-series", "note_state": null})
    );

    // A read for the meeting before is not this one's.
    state.series = Some(series_answer(at_local(10, 0), Value::Null));
    let next = projected_briefing(&state, at_local(9, 0))["next"].clone();
    assert_eq!(
        next["previous_actions"],
        json!({"state": "unknown", "cause": "series_not_read"})
    );

    state.series = Some(crate::SeriesPreviousRead {
        calendar_item_id: "item-design-review".into(),
        before_ms: at_local(11, 0),
        answer: Err("El hub respondió HTTP 404".into()),
    });
    let next = projected_briefing(&state, at_local(9, 0))["next"].clone();
    assert_eq!(
        next["previous_actions"],
        json!({"state": "unknown", "cause": "El hub respondió HTTP 404"})
    );
    assert_eq!(next["previous"]["recording_id"], "rec-by-title");
}

/// «Recordatorio personal» as the hub keeps it for every Mac: the next
/// meeting carries it once it was read for that meeting; a read of another
/// meeting, none yet or one that failed is unknown, and the Mac's own copy
/// stands.
#[test]
fn the_next_meeting_carries_its_reminder_from_the_hub() {
    use orbit_core::reminders::Reminder;
    let mut state = design_review();
    let next = projected_briefing(&state, at_local(9, 0))["next"].clone();
    assert!(next.get("reminder").is_none(), "not asked for yet");
    state.reminder = Some(crate::ReminderRead {
        series_key: "revisión de diseño".into(),
        answer: Ok(Some(Reminder {
            text: "Llevar las cifras".into(),
            edited_at_ms: 42,
        })),
    });
    let briefing = projected_briefing(&state, at_local(9, 0));
    assert_eq!(
        briefing["next"]["reminder"],
        json!({"state": "known", "text": "Llevar las cifras", "edited_at_ms": 42})
    );
    assert!(briefing["upcoming"][1].get("reminder").is_none());
    state.reminder = Some(crate::ReminderRead {
        series_key: "revisión de diseño".into(),
        answer: Ok(None),
    });
    let next = projected_briefing(&state, at_local(9, 0))["next"].clone();
    assert_eq!(next["reminder"], json!({"state": "none"}));
    state.reminder = Some(crate::ReminderRead {
        series_key: "revisión de diseño".into(),
        answer: Err("El hub respondió HTTP 404".into()),
    });
    let next = projected_briefing(&state, at_local(9, 0))["next"].clone();
    assert_eq!(
        next["reminder"],
        json!({"state": "unknown", "cause": "El hub respondió HTTP 404"})
    );
    // Read for another meeting: not this one's.
    state.reminder = Some(crate::ReminderRead {
        series_key: "comida con ribera".into(),
        answer: Ok(None),
    });
    let next = projected_briefing(&state, at_local(9, 0))["next"].clone();
    assert!(next.get("reminder").is_none());
}
