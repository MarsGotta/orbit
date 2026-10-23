use super::*;
use crate::daemon::{audio::tests::fixture, calendar::tests::seed};
use tokio::sync::mpsc;

#[test]
fn invalid_nested_calendar_field_is_named() {
    let row = json!({
        "observation_id":"synthetic",
        "calendar":{"source_id":"account"},
        "calendar_item_id":"item",
        "start_unix_ms":1,
        "end_unix_ms":2,
        "interpreted_timezone":"UTC",
        "is_floating":false,
        "is_all_day":false,
        "is_detached":false,
        "title":"synthetic",
        "status":"confirmed",
        "invitees_status":"not_requested",
        "event_url_status":"not_requested"
    });
    assert_eq!(invalid_calendar_event_field(&row), "calendar.calendar_id");
}

#[tokio::test]
async fn a_page_encoding_error_uses_its_own_code_on_the_real_walk() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut helper) = fixture(directory.path())?;
    seed(&mut runtime, &mut helper).await?;
    let (hub, mut frames) = mpsc::channel(16);
    runtime.link.outbound.install(hub);
    runtime
        .device_request_raw(&json!({
            "request_id":"encode-failure", "capability":"calendar_read",
            "parameters":{"start_ms":1_790_000_000_000i64,"end_ms":1_790_086_400_000i64},
        }))
        .await?;
    let walk = runtime
        .calendar
        .deimos
        .as_mut()
        .ok_or_else(|| anyhow::anyhow!("the calendar walk did not start"))?;
    walk.cache = Some(Cached {
        window: CalendarWindow {
            start_unix_ms: walk.at_ms,
            end_unix_ms: walk.range.end_unix_ms,
        },
        events: vec![],
    });
    runtime.continue_deimos_with(|_, _, _, _| {
        Err(serde_json::Error::io(std::io::Error::other(
            "synthetic encoding failure",
        )))
    });
    let frame = frames.try_recv()?;
    assert_eq!(frame["params"]["refusal"]["code"], "encode_failed");
    assert_eq!(frame["params"]["refusal"]["resource"], "calendar");
    assert!(!frame.to_string().contains("synthetic encoding failure"));
    Ok(())
}

#[tokio::test]
async fn invalid_calendar_event_answers_before_retiring_the_walk() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut helper) = fixture(directory.path())?;
    seed(&mut runtime, &mut helper).await?;
    let (hub, mut frames) = mpsc::channel(16);
    runtime.link.outbound.install(hub);
    runtime
        .device_request_raw(&json!({
            "request_id":"invalid-event", "capability":"calendar_read",
            "parameters":{"start_ms":1_790_000_000_000i64,"end_ms":1_790_086_400_000i64},
        }))
        .await?;
    let walk = runtime
        .calendar
        .deimos
        .as_mut()
        .ok_or_else(|| anyhow::anyhow!("the calendar walk did not start"))?;
    walk.answer.push(json!({"unexpected":"invalid event"}));
    runtime.send_deimos_page(true);
    let frame = frames.try_recv()?;
    assert_eq!(frame["params"]["request_id"], "invalid-event");
    assert_eq!(
        frame["params"]["refusal"],
        json!({
            "code":"helper_reply_invalid", "field":"observation_id", "received_type":"invalid_calendar_event",
            "expected":{"kind":"calendar_event", "position":0, "rows":1}, "retry":{"kind":"after_fix"}
        })
    );
    assert!(runtime.calendar.deimos.is_none());
    Ok(())
}

#[test]
fn numbered_calendar_causes_keep_their_contract_bounds() {
    use orbit_core::refusal::CalendarReadLimit;
    assert_eq!(
        bridge_calendar_refusal("calendar_event_too_large"),
        Refusal::CalendarReadLimit {
            cause: CalendarReadLimit::Page {
                max_bytes: orbit_core::calendar::MAX_NATIVE_PAGE_BYTES as u64
            }
        }
    );
    assert_eq!(
        bridge_calendar_refusal("calendar_limit_exceeded"),
        Refusal::CalendarReadLimit {
            cause: CalendarReadLimit::Fields {
                max_title_bytes: orbit_core::calendar::MAX_TITLE_BYTES as u64,
                max_label_bytes: orbit_core::calendar::MAX_LABEL_BYTES as u64,
                max_timezone_bytes: orbit_core::calendar::MAX_TIMEZONE_BYTES as u64,
            }
        }
    );
}
