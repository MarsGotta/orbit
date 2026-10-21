//! A write is refused before it reaches the helper, by
//! field and with its numbers.
use serde_json::json;

use super::*;

type TestResult = Result<(), Box<dyn std::error::Error>>;

/// Wednesday 14 October 2026, 10:00 UTC.
const START: i64 = 1_792_058_400_000;
const HOUR: i64 = 3_600_000;

fn draft() -> CalendarEventDraft {
    CalendarEventDraft {
        calendar_id: None,
        title: "Revisión con Ada".into(),
        start_ms: START,
        end_ms: START + HOUR,
        all_day: false,
        time_zone: Some("Europe/Madrid".into()),
        location: None,
        notes: None,
        url: None,
        availability: None,
        recurrence: None,
        alarms_minutes: None,
    }
}

fn invalid(refusal: Result<(), CalendarWriteRefusal>) -> CalendarInvalidChange {
    match refusal {
        Err(CalendarWriteRefusal::InvalidChange(change)) => *change,
        other => panic!("expected calendar_invalid_change, got {other:?}"),
    }
}

#[test]
fn a_well_formed_draft_and_change_pass() -> TestResult {
    draft().validate().map_err(|refusal| refusal.notice())?;
    CalendarEventChange {
        calendar_item_id: "item-1".into(),
        occurrence_start_ms: START,
        span: CalendarSpan::This,
        changes: CalendarEventChanges {
            start_ms: Some(START + HOUR),
            end_ms: Some(START + 2 * HOUR),
            ..CalendarEventChanges::default()
        },
    }
    .validate()
    .map_err(|refusal| refusal.notice())?;
    Ok(())
}

#[test]
fn an_empty_url_is_refused_on_create_and_clears_an_existing_event() -> TestResult {
    let mut fresh = draft();
    fresh.url = Some(String::new());
    let refusal = invalid(fresh.validate());
    assert_eq!(refusal.field, "url");
    assert_eq!(refusal.problem, CalendarChangeProblem::InvalidUrl);
    fresh.url = None;
    fresh.validate()?;
    CalendarEventChanges {
        url: Some(String::new()),
        ..Default::default()
    }
    .validate(CalendarSpan::This)?;
    Ok(())
}

#[test]
fn an_end_before_the_start_names_both_dates() {
    let mut value = draft();
    value.end_ms = START - 1;
    let change = invalid(value.validate());
    assert_eq!(change.field, "end_ms");
    assert_eq!(change.problem, CalendarChangeProblem::EndBeforeStart);
    assert_eq!(
        (change.start_ms, change.end_ms),
        (Some(START), Some(START - 1))
    );
    let notice = CalendarWriteRefusal::InvalidChange(Box::new(change)).notice();
    assert!(notice.contains(&START.to_string()), "{notice}");
    assert!(notice.contains(&(START - 1).to_string()), "{notice}");

    let moved = CalendarEventChanges {
        start_ms: Some(START),
        end_ms: Some(START - HOUR),
        ..CalendarEventChanges::default()
    };
    assert_eq!(
        invalid(moved.validate(CalendarSpan::This)).problem,
        CalendarChangeProblem::EndBeforeStart
    );
}

#[test]
fn an_empty_title_is_refused_by_name_in_a_draft_and_in_a_change() {
    for blank in ["", "   ", "\n\t"] {
        let mut value = draft();
        value.title = blank.into();
        let change = invalid(value.validate());
        assert_eq!(
            (change.field.as_str(), change.problem),
            ("title", CalendarChangeProblem::Empty)
        );
        let renamed = CalendarEventChanges {
            title: Some(blank.into()),
            ..CalendarEventChanges::default()
        };
        assert_eq!(
            invalid(renamed.validate(CalendarSpan::Future)).problem,
            CalendarChangeProblem::Empty
        );
    }
}

#[test]
fn a_field_past_its_limit_carries_its_bytes_and_the_limit() {
    type Set = fn(&mut CalendarEventDraft, String);
    let cases: [(&str, usize, Set); 4] = [
        ("title", MAX_TITLE_BYTES, |draft, text| draft.title = text),
        ("location", MAX_TITLE_BYTES, |draft, text| {
            draft.location = Some(text);
        }),
        ("notes", MAX_NOTES_BYTES, |draft, text| {
            draft.notes = Some(text)
        }),
        ("url", MAX_URL_BYTES, |draft, text| draft.url = Some(text)),
    ];
    for (field, limit, set) in cases {
        let mut fits = draft();
        set(&mut fits, "a".repeat(limit));
        assert!(fits.validate().is_ok(), "{field} at its limit must pass");
        let mut value = draft();
        set(&mut value, "é".repeat(limit / 2 + 1));
        let change = invalid(value.validate());
        assert_eq!(change.field, field);
        assert_eq!(change.problem, CalendarChangeProblem::TooLarge);
        assert_eq!(change.bytes, Some(((limit / 2 + 1) * 2) as u64), "{field}");
        assert_eq!(change.limit, Some(limit as u64), "{field}");
        let notice = CalendarWriteRefusal::InvalidChange(Box::new(change)).notice();
        assert!(notice.contains(&limit.to_string()), "{notice}");
    }
}

#[test]
fn alarms_past_their_count_or_reach_carry_the_numbers() {
    let mut value = draft();
    value.alarms_minutes = Some(vec![10; MAX_EVENT_ALARMS + 1]);
    let change = invalid(value.validate());
    assert_eq!(change.problem, CalendarChangeProblem::TooMany);
    assert_eq!(change.count, Some(MAX_EVENT_ALARMS as u64 + 1));
    assert_eq!(change.limit, Some(MAX_EVENT_ALARMS as u64));
    value.alarms_minutes = Some(vec![MAX_ALARM_MINUTES + 1]);
    let change = invalid(value.validate());
    assert_eq!(change.value, Some(MAX_ALARM_MINUTES as i64 + 1));
    assert_eq!(change.limit, Some(MAX_ALARM_MINUTES));
}

#[test]
fn dates_and_identifiers_outside_their_shape_are_refused() {
    let mut value = draft();
    value.start_ms = crate::calendar::MAX_UNIX_MS + 1;
    let change = invalid(value.validate());
    assert_eq!(
        (change.field.as_str(), change.problem, change.value),
        (
            "start_ms",
            CalendarChangeProblem::InvalidDate,
            Some(crate::calendar::MAX_UNIX_MS + 1)
        )
    );
    let mut value = draft();
    value.calendar_id = Some("x".repeat(MAX_LABEL_BYTES + 1));
    assert_eq!(
        invalid(value.validate()).problem,
        CalendarChangeProblem::InvalidIdentifier
    );
    let delete = CalendarDelete {
        calendar_item_id: String::new(),
        occurrence_start_ms: START,
        span: CalendarSpan::This,
    };
    assert_eq!(
        invalid(delete.validate()).field,
        "calendar_item_id".to_owned()
    );
}

#[test]
fn a_change_that_sets_nothing_or_repeats_only_this_one_is_refused() {
    let nothing = CalendarEventChanges::default();
    assert_eq!(
        invalid(nothing.validate(CalendarSpan::This)).problem,
        CalendarChangeProblem::NothingToChange
    );
    let weekly = CalendarRecurrenceRule {
        frequency: CalendarFrequency::Weekly,
        interval: 1,
        days_of_week: Some(vec![CalendarRecurrenceDay {
            day: CalendarWeekday::Wednesday,
            week: None,
        }]),
        end: Some(CalendarRecurrenceEnd {
            until_ms: None,
            count: Some(10),
        }),
    };
    let repeat = CalendarEventChanges {
        recurrence: Some(weekly),
        ..CalendarEventChanges::default()
    };
    assert_eq!(
        invalid(repeat.validate(CalendarSpan::This)).problem,
        CalendarChangeProblem::RecurrenceNeedsFutureSpan
    );
    assert!(repeat.validate(CalendarSpan::Future).is_ok());
    let stop = CalendarEventChanges {
        remove_recurrence: Some(true),
        ..CalendarEventChanges::default()
    };
    assert!(stop.validate(CalendarSpan::Future).is_ok());
}

#[test]
fn a_malformed_rule_is_refused() {
    let mut value = draft();
    value.recurrence = Some(CalendarRecurrenceRule {
        frequency: CalendarFrequency::Daily,
        interval: 0,
        days_of_week: None,
        end: None,
    });
    assert_eq!(invalid(value.validate()).field, "recurrence");
    value.recurrence = Some(CalendarRecurrenceRule {
        frequency: CalendarFrequency::Monthly,
        interval: 1,
        days_of_week: Some(vec![CalendarRecurrenceDay {
            day: CalendarWeekday::Friday,
            week: Some(0),
        }]),
        end: None,
    });
    assert_eq!(invalid(value.validate()).field, "recurrence");
    value.recurrence = Some(CalendarRecurrenceRule {
        frequency: CalendarFrequency::Daily,
        interval: 1,
        days_of_week: None,
        end: Some(CalendarRecurrenceEnd {
            until_ms: Some(START - HOUR),
            count: None,
        }),
    });
    let change = invalid(value.validate());
    assert_eq!(change.problem, CalendarChangeProblem::EndBeforeStart);
    assert_eq!(change.end_ms, Some(START - HOUR));
}

#[test]
fn every_refusal_round_trips_through_its_native_code_and_detail() -> TestResult {
    let refusals = [
        CalendarWriteRefusal::EventNotFound {
            calendar_item_id: "item-1".into(),
            occurrence_start_ms: START,
        },
        CalendarWriteRefusal::NotEditable {
            reason: CalendarNotEditable::OrganizedBySomeoneElse,
            calendar_id: "work".into(),
        },
        CalendarWriteRefusal::InvalidChange(Box::new(CalendarInvalidChange::too_large(
            "notes",
            40_000,
            MAX_NOTES_BYTES,
        ))),
        CalendarWriteRefusal::PermissionDenied {
            authorization: CalendarAuthorization::WriteOnly,
        },
        CalendarWriteRefusal::SaveFailed {
            error_domain: "EKErrorDomain".into(),
            error_code: 12,
        },
        CalendarWriteRefusal::OutcomeUnknown {
            cause: "write_deadline".into(),
        },
    ];
    for refusal in refusals {
        let detail = refusal.detail();
        assert!(detail.get("code").is_none(), "{detail}");
        let read = CalendarWriteRefusal::from_native(refusal.code(), Some(&detail))?;
        assert_eq!(read, refusal);
    }
    // A code that is not a write refusal was refused before EventKit was asked.
    assert_eq!(
        CalendarWriteRefusal::from_native("calendar_busy", None)?,
        CalendarWriteRefusal::NotWritten {
            cause: "calendar_busy".into()
        }
    );
    // A refusal whose detail does not match its code is a broken reply, not a fact.
    assert!(
        CalendarWriteRefusal::from_native(
            "calendar_event_not_found",
            Some(&json!({"field": "title"}))
        )
        .is_err()
    );
    assert!(
        CalendarWriteRefusal::from_native(
            "calendar_permission_denied",
            Some(&json!({"authorization": "full_access"}))
        )
        .is_err()
    );
    Ok(())
}

#[test]
fn the_wire_shapes_are_the_documented_ones() -> TestResult {
    let create = CalendarSave::Create(Box::new(draft()));
    assert_eq!(
        serde_json::to_value(&create)?,
        json!({"create": {"title": "Revisión con Ada", "start_ms": START,
            "end_ms": START + HOUR, "all_day": false, "time_zone": "Europe/Madrid"}})
    );
    let change: CalendarSave = serde_json::from_value(json!({"change": {
        "calendar_item_id": "item-1", "occurrence_start_ms": START, "span": "future",
        "changes": {"start_ms": START + HOUR, "end_ms": START + 2 * HOUR}}}))?;
    assert!(change.validate().is_ok());
    for broken in [
        json!({"create": draft(), "change": {}}),
        json!({"change": {"calendar_item_id": "item-1", "occurrence_start_ms": START,
            "span": "all", "changes": {"title": "x"}}}),
        json!({"change": {"calendar_item_id": "item-1", "occurrence_start_ms": START,
            "span": "this", "changes": {"title": null}}}),
    ] {
        assert!(
            serde_json::from_value::<CalendarSave>(broken.clone()).is_err(),
            "{broken}"
        );
    }
    Ok(())
}

/// Rust side: every calendar fixture the helper and the
/// daemon share decodes through the production types and passes validation —
/// the read with every field, `calendar_save` (create and change),
/// `calendar_delete`, both results and one refusal per code.
#[test]
fn every_calendar_fixture_decodes_and_validates() -> TestResult {
    use crate::calendar::{CalendarInventoryPage, CalendarReadPage};
    use crate::wire::native::{NativeCommand, NativeEvent};
    let page: CalendarReadPage = serde_json::from_str(include_str!(
        "../../../../../contracts/fixtures/native/results/calendar_snapshot.json"
    ))?;
    page.validate()?;
    let full = page.events.first().ok_or("no event in the read fixture")?;
    assert!(full.location.is_some() && full.notes.is_some() && full.organizer.is_some());
    assert!(full.availability.is_some() && full.recurrence.is_some() && full.can_edit.is_some());
    let invitation = page
        .events
        .get(1)
        .ok_or("no invitation in the read fixture")?;
    assert_eq!(
        invitation.notes_left_out,
        Some(CalendarFieldLeftOut {
            bytes: 40_000,
            limit: MAX_NOTES_BYTES as u64
        })
    );
    assert_eq!(invitation.can_edit, Some(false));
    assert_eq!(
        invitation
            .recurrence
            .as_ref()
            .and_then(|rule| rule.rules_count),
        Some(2)
    );
    let listing: CalendarInventoryPage = serde_json::from_str(include_str!(
        "../../../../../contracts/fixtures/native/results/calendar_sources.json"
    ))?;
    listing.validate()?;
    let calendar = listing.calendars.first().ok_or("no calendar listed")?;
    assert_eq!(calendar.color.as_deref(), Some("#1BADF8"));
    assert_eq!(
        (calendar.can_edit, calendar.is_default),
        (Some(true), Some(true))
    );

    for raw in [
        include_str!("../../../../../contracts/fixtures/native/commands/calendar_save.json"),
        include_str!(
            "../../../../../contracts/fixtures/native/calendar/commands/calendar_save-change.json"
        ),
    ] {
        let NativeCommand::CalendarSave { calendar_save, .. } = serde_json::from_str(raw)? else {
            return Err("calendar_save fixture is another command".into());
        };
        calendar_save
            .validate()
            .map_err(|refusal| refusal.notice())?;
    }
    let NativeCommand::CalendarDelete {
        calendar_delete, ..
    } = serde_json::from_str(include_str!(
        "../../../../../contracts/fixtures/native/commands/calendar_delete.json"
    ))?
    else {
        return Err("calendar_delete fixture is another command".into());
    };
    calendar_delete
        .validate()
        .map_err(|refusal| refusal.notice())?;

    let saved: CalendarSaveResult = serde_json::from_str(include_str!(
        "../../../../../contracts/fixtures/native/results/calendar_save.json"
    ))?;
    saved.validate()?;
    let deleted: CalendarDeleteResult = serde_json::from_str(include_str!(
        "../../../../../contracts/fixtures/native/results/calendar_delete.json"
    ))?;
    deleted.validate_response(
        &calendar_delete,
        &CalendarWriteFacts {
            native_instance_id: &deleted.native_instance_id,
            privacy_generation: deleted.privacy_generation,
        },
    )?;

    for (code, raw) in [
        (
            "calendar_event_not_found",
            include_str!(
                "../../../../../contracts/fixtures/native/calendar/refusals/calendar_event_not_found.json"
            ),
        ),
        (
            "calendar_not_editable",
            include_str!(
                "../../../../../contracts/fixtures/native/calendar/refusals/calendar_not_editable.json"
            ),
        ),
        (
            "calendar_invalid_change",
            include_str!(
                "../../../../../contracts/fixtures/native/calendar/refusals/calendar_invalid_change.json"
            ),
        ),
        (
            "calendar_permission_denied",
            include_str!(
                "../../../../../contracts/fixtures/native/calendar/refusals/calendar_permission_denied.json"
            ),
        ),
        (
            "calendar_save_failed",
            include_str!(
                "../../../../../contracts/fixtures/native/calendar/refusals/calendar_save_failed.json"
            ),
        ),
        (
            "calendar_write_outcome_unknown",
            include_str!(
                "../../../../../contracts/fixtures/native/calendar/refusals/calendar_write_outcome_unknown.json"
            ),
        ),
    ] {
        let NativeEvent::Response {
            ok: false,
            error: Some(error),
            ..
        } = serde_json::from_str(raw)?
        else {
            return Err(format!("{code}: not a refusal").into());
        };
        assert_eq!(error.code, code);
        let refusal = CalendarWriteRefusal::from_native(&error.code, error.detail.as_ref())?;
        assert_eq!(refusal.code(), code);
        assert!(!refusal.notice().is_empty());
    }
    Ok(())
}
