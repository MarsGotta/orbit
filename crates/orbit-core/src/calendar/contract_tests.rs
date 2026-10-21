//! Shared synthetic Swift/Rust wire evidence for one page of an all-calendar
//! read (REF-10), not EventKit or remote authority.

use serde_json::{Value, json};

use super::*;

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

fn fixture() -> TestResult<Value> {
    Ok(serde_json::from_str(include_str!(
        "../../../../contracts/fixtures/calendar-read-page-v1.json"
    ))?)
}

fn snapshot() -> TestResult<Value> {
    Ok(fixture()?["page"].clone())
}

fn decode_snapshot(bytes: &[u8]) -> Result<CalendarReadPage, CalendarError> {
    decode_read_page(bytes)
}

fn wire(value: &Value) -> TestResult<Vec<u8>> {
    Ok(serde_json::to_vec(value)?)
}

fn replace(value: &mut Value, pointer: &str, replacement: Value) -> TestResult {
    *value.pointer_mut(pointer).ok_or("existing fixture path")? = replacement;
    Ok(())
}

#[test]
fn shared_calendar_fixture_preserves_wire_fields_and_full_width_integers() -> TestResult {
    let fixture = fixture()?;
    let decoded = decode_snapshot(&wire(&fixture["page"])?)?;
    assert_eq!(serde_json::to_value(&decoded)?, fixture["page"]);
    assert_eq!(decoded.privacy_generation, 9_007_199_254_740_993);
    assert_eq!(
        decoded.capture_started_monotonic_ns,
        18_446_744_070_000_000_000
    );
    assert_eq!(decoded.read_id, "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa");
    assert_eq!((decoded.after_index, decoded.total), (0, 3));
    assert_eq!(decoded.next_after_index, None);
    assert_eq!(decoded.omitted[0].count, 1);
    assert!(decoded.events[0].title.contains("e\u{301} / é 🪐"));
    assert_eq!(decoded.events[1].start_unix_ms, 1_699_070_400_000);
    assert_eq!(decoded.events[1].end_unix_ms, 1_699_246_800_000);
    assert_eq!(decoded.events[1].invitees, None);
    assert_eq!(
        decoded.events[1].invitees_status,
        CalendarFieldStatus::NotProvided
    );
    assert_eq!(decoded.events[2].invitees, Some(vec![]));
    assert_eq!(decoded.events[2].event_url.as_deref(), Some(""));
    Ok(())
}

#[test]
fn wire_optional_null_unknown_members_and_duplicate_keys_are_rejected() -> TestResult {
    let mut null_next = snapshot()?;
    null_next["next_after_index"] = Value::Null;
    assert_eq!(
        decode_snapshot(&wire(&null_next)?),
        Err(CalendarError::InvalidWire)
    );
    for pointer in [
        "/events/0/event_id",
        "/events/0/original_occurrence_unix_ms",
        "/events/0/event_timezone",
        "/events/0/last_modified_unix_ms",
        "/events/0/invitees",
        "/events/0/event_url",
        "/events/0/invitees/0/name",
        "/events/0/invitees/0/email",
    ] {
        let mut value = snapshot()?;
        replace(&mut value, pointer, Value::Null)?;
        assert!(decode_snapshot(&wire(&value)?).is_err(), "{pointer}");
    }
    for pointer in [
        "",
        "/window",
        "/events/0",
        "/events/0/calendar",
        "/events/0/invitees/0",
        "/omitted/0",
    ] {
        let mut value = snapshot()?;
        value
            .pointer_mut(pointer)
            .ok_or("fixture path")?
            .as_object_mut()
            .ok_or("fixture object")?
            .insert("unknown".into(), json!(true));
        assert!(decode_snapshot(&wire(&value)?).is_err(), "{pointer}");
    }
    let raw = String::from_utf8(wire(&snapshot()?)?)?;
    for (from, to) in [
        (
            "\"provider_sync_status\":\"unknown\"",
            "\"provider_sync_status\":\"unknown\",\"provider_sync_status\":\"unknown\"",
        ),
        ("\"total\":3", "\"total\":3,\"total\":3"),
        ("\"title\":\"\"", "\"title\":\"\",\"ti\\u0074le\":\"\""),
    ] {
        assert!(raw.contains(from));
        assert!(decode_snapshot(raw.replacen(from, to, 1).as_bytes()).is_err());
    }
    Ok(())
}

#[test]
fn numeric_and_boolean_contracts_do_not_coerce_or_round() -> TestResult {
    for pointer in [
        "/privacy_generation",
        "/after_index",
        "/total",
        "/window/start_unix_ms",
        "/window/end_unix_ms",
        "/capture_started_monotonic_ns",
        "/capture_completed_monotonic_ns",
        "/observed_at_unix_ms",
        "/events/0/original_occurrence_unix_ms",
        "/events/0/start_unix_ms",
        "/events/0/end_unix_ms",
        "/events/0/last_modified_unix_ms",
    ] {
        for replacement in [json!(true), json!(1.0), json!("1")] {
            let mut value = snapshot()?;
            replace(&mut value, pointer, replacement)?;
            assert!(decode_snapshot(&wire(&value)?).is_err(), "{pointer}");
        }
    }
    for pointer in [
        "/events/0/is_all_day",
        "/events/0/is_detached",
        "/events/0/is_floating",
        "/events/0/invitees/0/is_current_user",
    ] {
        for replacement in [json!(0), json!("true")] {
            let mut value = snapshot()?;
            replace(&mut value, pointer, replacement)?;
            assert!(decode_snapshot(&wire(&value)?).is_err(), "{pointer}");
        }
    }
    Ok(())
}

#[test]
fn negative_zero_wire_is_rejected_before_integer_canonicalization() -> TestResult {
    for (pointer, field) in [
        ("/privacy_generation", "privacy_generation"),
        (
            "/capture_started_monotonic_ns",
            "capture_started_monotonic_ns",
        ),
        ("/observed_at_unix_ms", "observed_at_unix_ms"),
    ] {
        let mut value = snapshot()?;
        value["capture_started_monotonic_ns"] = json!(0);
        value["capture_completed_monotonic_ns"] = json!(1);
        replace(&mut value, pointer, json!(0))?;
        let raw = String::from_utf8(wire(&value)?)?;
        let decoded = decode_snapshot(raw.as_bytes())?;
        let token = format!("\"{field}\":0");
        assert_eq!(raw.matches(&token).count(), 1);
        let negative_zero = raw.replacen(&token, &format!("\"{field}\":-0"), 1);
        assert_eq!(
            decode_snapshot(negative_zero.as_bytes()),
            Err(CalendarError::InvalidWire),
            "{pointer}"
        );
        assert_eq!(serde_json::to_value(decoded)?, value);
    }
    Ok(())
}

#[test]
fn clocks_and_date_window_have_exact_inclusive_exclusive_limits() -> TestResult {
    for (start, end, valid) in [
        (0, 0, true),
        (0, READ_LIFETIME_NS - 1, true),
        (0, READ_LIFETIME_NS, false),
        (2, 1, false),
        (u64::MAX, u64::MAX, true),
    ] {
        let mut value = snapshot()?;
        value["capture_started_monotonic_ns"] = json!(start);
        value["capture_completed_monotonic_ns"] = json!(end);
        assert_eq!(decode_snapshot(&wire(&value)?).is_ok(), valid);
    }
    for (start, end, valid) in [
        (MIN_UNIX_MS, MIN_UNIX_MS + 1, true),
        (MAX_UNIX_MS - 1, MAX_UNIX_MS, true),
        // REF-10: a page covers whatever window it was asked for; the width is
        // bounded when the helper is asked (`CalendarReadQuery`), not here.
        (0, EVENTKIT_MAX_QUERY_MS, true),
        (0, 0, false),
        (1, 0, false),
        (MIN_UNIX_MS - 1, 0, false),
        (MAX_UNIX_MS, MAX_UNIX_MS + 1, false),
        (i64::MIN, i64::MAX, false),
    ] {
        let mut value = snapshot()?;
        value["events"] = json!([]);
        value["total"] = json!(0);
        value["window"]["start_unix_ms"] = json!(start);
        value["window"]["end_unix_ms"] = json!(end);
        assert_eq!(decode_snapshot(&wire(&value)?).is_ok(), valid);
    }
    Ok(())
}

#[test]
fn overlap_is_half_open_without_clipping_or_guessing_timezone() -> TestResult {
    for (start, end, valid) in [
        (10, 10, true),
        (20, 20, false),
        (9, 9, false),
        (0, 10, false),
        (0, 11, true),
        (19, 30, true),
        (20, 30, false),
        (0, 30, true),
        (15, 14, false),
    ] {
        let mut value = snapshot()?;
        let mut point = value["events"][2].clone();
        point["start_unix_ms"] = json!(start);
        point["end_unix_ms"] = json!(end);
        value["events"] = json!([point]);
        value["total"] = json!(1);
        value["window"]["start_unix_ms"] = json!(10);
        value["window"]["end_unix_ms"] = json!(20);
        let result = decode_snapshot(&wire(&value)?);
        assert_eq!(result.is_ok(), valid);
        if let Ok(result) = result {
            assert_eq!(
                (result.events[0].start_unix_ms, result.events[0].end_unix_ms),
                (start, end)
            );
        }
    }
    let mut value = snapshot()?;
    value["events"][0]["interpreted_timezone"] = json!("Europe/Madrid");
    assert!(decode_snapshot(&wire(&value)?).is_err());
    value["events"][0]["event_timezone"] = json!("Europe/Madrid");
    assert!(decode_snapshot(&wire(&value)?).is_ok());
    Ok(())
}

/// REF-10: every page asks for invitees and the URL, so their status is one
/// of `not_provided` or `provided`, and presence has to agree with it.
#[test]
fn optional_categories_are_always_requested_and_presence_must_agree() -> TestResult {
    for field in ["invitees", "event_url"] {
        for (status, present, valid) in [
            ("not_provided", false, true),
            ("provided", true, true),
            ("not_requested", false, false),
            ("not_requested", true, false),
            ("provided", false, false),
            ("not_provided", true, false),
        ] {
            let mut value = snapshot()?;
            let mut point = value["events"][2].clone();
            point[format!("{field}_status")] = json!(status);
            if !present {
                point.as_object_mut().ok_or("fixture object")?.remove(field);
            }
            value["events"] = json!([point]);
            value["total"] = json!(1);
            assert_eq!(
                decode_snapshot(&wire(&value)?).is_ok(),
                valid,
                "{field}/{status}/{present}"
            );
        }
    }
    Ok(())
}

#[test]
fn mailto_candidates_are_exact_utf8_and_not_a_person_identity() -> TestResult {
    for (url, candidate) in [
        (
            "MAILTO:%C3%81lex%2BOrbit@example.invalid",
            Some("Álex+Orbit@example.invalid"),
        ),
        ("mailto:a+b@X.invalid", Some("a+b@X.invalid")),
        ("mailto:a%2540b@x.invalid", Some("a%40b@x.invalid")),
        ("urn:synthetic:room", None),
        ("mailto://a@b", None),
        ("mailto:a@b?subject=x", None),
        ("mailto:a@b#x", None),
        ("mailto:a@b,c@d", None),
        ("mailto:%FF@b", None),
        ("mailto:%ED%A0%80@b", None),
        ("mailto:a%0Ab@c", None),
        ("mailto:a%20b@c", None),
        ("mailto:a%G0@b", None),
        ("mailto:a%@b", None),
        ("mailto:a%1@b", None),
        ("mailto:a@b@c", None),
        ("mailto:@b", None),
        ("mailto:a@", None),
        ("mailto:a%3Ab@c", None),
        ("mailto:a\u{85}b@c", None),
    ] {
        assert_eq!(
            super::mailbox::candidate(url).as_deref(),
            candidate,
            "{url}"
        );
    }
    let mut value = snapshot()?;
    value["events"][0]["invitees"][0]
        .as_object_mut()
        .ok_or("fixture object")?
        .remove("email");
    let parsed = decode_snapshot(&wire(&value)?)?;
    assert!(
        parsed.events[0]
            .invitees
            .as_ref()
            .ok_or("fixture invitees")?[0]
            .email
            .is_none()
    );
    Ok(())
}

#[test]
fn malformed_unicode_and_nonfinite_json_are_not_canonicalized() -> TestResult {
    for input in [
        b"\xff".as_slice(),
        b"null",
        b"NaN",
        b"Infinity",
        b"1e9999",
        b"{} {}",
        br#""\ud800""#,
    ] {
        assert!(decode_snapshot(input).is_err());
    }
    let raw = String::from_utf8(wire(&snapshot()?)?)?;
    let invalid = raw.replacen("\"title\":\"\"", r#""title":"\ud800""#, 1);
    assert!(decode_snapshot(invalid.as_bytes()).is_err());
    let valid = raw.replacen("\"title\":\"\"", r#""title":"\ud83e\ude90""#, 1);
    assert_eq!(decode_snapshot(valid.as_bytes())?.events[2].title, "🪐");
    Ok(())
}

/// The only size limit on a page is the native message it travels in; any
/// number of events below it is valid. Nothing is truncated to fit it.
#[test]
fn a_page_is_bounded_by_its_message_not_by_a_count_of_events() -> TestResult {
    let mut value = snapshot()?;
    let point = value["events"][2].clone();
    value["events"] = Value::Array(
        (0..1_000)
            .map(|index| {
                let mut event = point.clone();
                event["observation_id"] = json!(uuid::Uuid::from_u128(index).to_string());
                event
            })
            .collect(),
    );
    value["total"] = json!(1_000);
    assert!(
        decode_snapshot(&wire(&value)?).is_ok(),
        "a thousand events is a page"
    );
    let mut remaining = MAX_NATIVE_PAGE_BYTES
        .checked_sub(wire(&value)?.len())
        .ok_or("fixture size")?;
    for event in value["events"].as_array_mut().ok_or("fixture array")? {
        let added = remaining.min(2048);
        event["title"] = json!("x".repeat(added));
        remaining -= added;
    }
    assert_eq!(remaining, 0);
    assert_eq!(wire(&value)?.len(), MAX_NATIVE_PAGE_BYTES);
    assert!(decode_snapshot(&wire(&value)?).is_ok());
    value["events"][999]["title"] =
        json!("x".repeat(value["events"][999]["title"].as_str().map_or(0, str::len) + 1));
    assert_eq!(wire(&value)?.len(), MAX_NATIVE_PAGE_BYTES + 1);
    assert_eq!(
        decode_snapshot(&wire(&value)?),
        Err(CalendarError::LimitExceeded)
    );
    Ok(())
}

#[test]
fn syntactic_snapshot_neither_updates_provider_sync_nor_grants_local_scope() -> TestResult {
    let mut value = snapshot()?;
    value["events"] = json!([]);
    value["total"] = json!(0);
    let parsed = decode_snapshot(&wire(&value)?)?;
    assert_eq!(parsed.provider_sync_status, "unknown");
    assert_eq!(parsed.enumeration, "bounded_local_read");
    let facts = CalendarReadFacts {
        native_instance_id: &parsed.native_instance_id,
        privacy_generation: parsed.privacy_generation,
        authorization: CalendarAuthorization::Denied,
        privacy_stopped: false,
        elapsed_since_request_ns: Some(0),
    };
    let query = CalendarReadQuery {
        start_unix_ms: parsed.window.start_unix_ms,
        end_unix_ms: parsed.window.end_unix_ms,
        after_index: 0,
    };
    assert_eq!(
        parsed.validate_response(&query, &facts),
        Err(CalendarError::PermissionRequired)
    );
    value["provider_sync_status"] = json!("synced");
    assert!(decode_snapshot(&wire(&value)?).is_err());
    Ok(())
}

/// The failure codes the helper's `CalendarFailure` enum declares, read from
/// its Swift source: the raw value in quotes, or the case name without one.
fn swift_failure_codes() -> TestResult<Vec<String>> {
    let swift =
        include_str!("../../../../native/macos/Sources/OrbitNativeProtocol/CalendarModels.swift");
    let body = swift
        .split("public enum CalendarFailure")
        .nth(1)
        .and_then(|rest| rest.split("\n}\n").next())
        .ok_or("CalendarFailure enum in CalendarModels.swift")?;
    let mut codes = Vec::new();
    for line in body.lines() {
        let Some(case) = line.trim().strip_prefix("case ") else {
            continue;
        };
        match case.split('"').nth(1) {
            Some(raw) => codes.push(raw.to_owned()),
            None => codes.push(case.trim().to_owned()),
        }
    }
    Ok(codes)
}

/// REF-10: every failure the helper can put in a status decodes here. A
/// status carrying a code this side does not know is refused whole, and the
/// permission it reported with it: `calendar_event_too_large` was such a code.
#[test]
fn every_failure_the_helper_names_is_one_this_side_decodes() -> TestResult {
    let codes = swift_failure_codes()?;
    assert!(
        codes.len() >= 11 && codes.iter().any(|code| code == "calendar_event_too_large"),
        "{codes:?}"
    );
    for code in codes {
        let status = format!(
            r#"{{"authorization":"full_access","physical_read_pending":false,"failure":"{code}","read_only":true}}"#
        );
        let decoded =
            decode_status(status.as_bytes()).map_err(|error| format!("{code}: {error:?}"))?;
        assert_eq!(
            serde_json::to_value(decoded.failure)?,
            Value::String(code.clone()),
            "{code}"
        );
    }
    Ok(())
}
