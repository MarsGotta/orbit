//! REF-10 (and CAL-03, CAL-06): Deimos reads any range of the agenda in one
//! question, through the real `device_request`, answered in pages.
//!
//! The helper is synthetic and serves a fixed agenda; the hub frames are read
//! off the channel the link would carry. Each case walks the pages exactly as
//! `server/orbit_mcp/calendar.py` does: the first request carries the range,
//! each next one the cursor the previous page returned.
use crate::daemon::turn::said;
use anyhow::{Context, Result, bail};
use orbit_core::device_bridge::INVITEES_CONTINUED;
use orbit_core::notice::NoticeCode;
use tokio::sync::mpsc;

use super::tests::{
    DAY_MS, HOST, OCTOBER_MS, ack, calendars_page, event, host_state, page, reject, seed, status,
};
use super::*;
use crate::daemon::audio::tests::fixture;

/// The one document both sides of the bridge read (CAL-06).
const CONTRACT: &str =
    include_str!("../../../../../contracts/fixtures/device-bridge-request-v1.json");

struct Mac {
    _directory: tempfile::TempDir,
    runtime: Runtime,
    helper: mpsc::Receiver<Value>,
    frames: mpsc::Receiver<Value>,
    /// The agenda the synthetic helper serves, every calendar.
    agenda: Vec<Value>,
    /// Events per helper page, standing in for `NATIVE_PAGE_EVENT_BYTES`.
    helper_page: usize,
    windows_read: usize,
    /// Events the helper itself could not carry, as it names them on the
    /// first page of each window (decision 4).
    helper_left_out: Vec<Value>,
    /// Every event the answers named as left out, joined over the walk.
    left_out: Vec<Value>,
}

#[tokio::test]
async fn ref207_a4_deimos_uses_the_same_pure_calendar_access_decision() -> Result<()> {
    let mut mac = mac(vec![]).await?;
    mac.runtime.features.calendar.view.authorization = CalendarAuthorization::Denied;
    let core = orbit_core::topics::agenda::access(CalendarAuthorization::Denied, false);
    assert_eq!(mac.runtime.calendar_access(), core);
    let refusal = match mac.runtime.calendar_bridge_read(
        "denied-fixture",
        OCTOBER_MS,
        OCTOBER_MS + DAY_MS,
        None,
    ) {
        Err(refusal) => refusal,
        Ok(()) => bail!("a denied macOS permission opened the bridge read"),
    };
    assert_eq!(
        refusal,
        orbit_core::refusal::Refusal::PermissionDenied {
            permission: orbit_core::refusal::Permission::Calendar
        }
    );
    assert!(matches!(
        core,
        orbit_core::topics::Knowledge::Absent {
            refusal: orbit_core::refusal::Refusal::PermissionDenied { .. }
        }
    ));
    for (authorization, expected_code) in [
        (
            CalendarAuthorization::Unknown,
            "calendar_permission_unknown",
        ),
        (
            CalendarAuthorization::Unavailable,
            "calendar_backend_unavailable",
        ),
    ] {
        mac.runtime.features.calendar.view.authorization = authorization;
        let core = orbit_core::topics::agenda::access(authorization, false);
        assert_eq!(mac.runtime.calendar_access(), core);
        let result = mac.runtime.calendar_bridge_read(
            "other-fixture",
            OCTOBER_MS,
            OCTOBER_MS + DAY_MS,
            None,
        );
        assert_eq!(
            result.err(),
            Some(Runtime::bridge_refusal_code(expected_code))
        );
    }
    Ok(())
}

#[tokio::test]
async fn ref303_a3_full_hub_names_calendar_reply_capacity_without_success_notice() -> Result<()> {
    let mut mac = mac(vec![event(1, OCTOBER_MS, "work")]).await?;
    for _ in 0..64 {
        assert!(
            mac.runtime
                .send_frame(Ok(json!({"fixture":"occupied"})))
                .delivered_to_queue()
        );
    }
    mac.runtime
        .device_request_raw(&json!({
            "request_id":"full-calendar", "capability":"calendar_read",
            "parameters":{"start_ms":OCTOBER_MS,"end_ms":OCTOBER_MS + DAY_MS}
        }))
        .await?;
    mac.serve().await?;
    // SIM-21: not delivered is said; the full channel and its 64 frames
    // are in daemon.log under the same op.
    assert_eq!(
        mac.runtime.state.notice_code(),
        Some(NoticeCode::BridgeReplyUndelivered),
        "{:?}",
        mac.runtime.state.durable.notice
    );
    let told = said::told(&mac.runtime);
    assert!(told.contains("64") && told.contains("lleno"), "{told}");
    assert_eq!(mac.frames.try_recv()?["fixture"], "occupied");
    Ok(())
}

#[tokio::test]
async fn ref303_a3_retired_hub_names_calendar_reply_loss_without_success_row() -> Result<()> {
    let mut mac = mac(vec![event(1, OCTOBER_MS, "work")]).await?;
    mac.runtime
        .hub
        .retire(crate::daemon::hub_link::Retirement::AuthorityRetired);
    mac.runtime
        .device_request_raw(&json!({
            "request_id":"retired-calendar", "capability":"calendar_read",
            "parameters":{"start_ms":OCTOBER_MS,"end_ms":OCTOBER_MS + DAY_MS}
        }))
        .await?;
    mac.serve().await?;
    assert_eq!(
        mac.runtime.state.notice_code(),
        Some(NoticeCode::BridgeReplyUndelivered),
        "{:?}",
        mac.runtime.state.durable.notice
    );
    let told = said::told(&mac.runtime);
    assert!(told.contains("retirada de autoridad"), "{told}");
    Ok(())
}

#[tokio::test]
async fn ref303_round2_calendar_privacy_notice_hides_internal_code() -> Result<()> {
    let mut mac = mac(vec![event(1, OCTOBER_MS, "work")]).await?;
    mac.runtime
        .device_request_raw(&json!({
            "request_id":"privacy-calendar", "capability":"calendar_read",
            "parameters":{"start_ms":OCTOBER_MS,"end_ms":OCTOBER_MS + DAY_MS}
        }))
        .await?;
    mac.runtime
        .deimos_failed_refusal(orbit_core::refusal::Refusal::PrivacyStopped { since_ms: None });
    // Deimos gets the typed cause; the Mac's delivery notice keeps its own code.
    let frame = mac.frames.try_recv()?;
    assert_eq!(frame["params"]["refusal"]["code"], "privacy_stopped");
    let notice = serde_json::to_string(&mac.runtime.state.durable.notice)?;
    assert!(!notice.contains("privacy_stopped"), "{notice}");
    Ok(())
}

#[tokio::test]
async fn ref303b_r2_stop_answers_pending_calendar_read_once() -> Result<()> {
    let mut mac = mac(vec![event(1, OCTOBER_MS, "work")]).await?;
    mac.runtime
        .device_request_raw(&json!({
            "request_id":"calendar-during-stop", "capability":"calendar_read",
            "parameters":{"start_ms":OCTOBER_MS,"end_ms":OCTOBER_MS + DAY_MS}
        }))
        .await?;
    assert!(mac.runtime.features.calendar.deimos.is_some());
    assert!(mac.frames.try_recv().is_err());

    mac.runtime
        .privacy_stop(orbit_core::privacy_stop::MarsStop::Order)
        .await;

    let frame = mac.frames.try_recv()?;
    assert_eq!(frame["method"], "device.reply");
    assert_eq!(frame["params"]["request_id"], "calendar-during-stop");
    assert_eq!(
        frame["params"]["refusal"]["code"], "privacy_stopped",
        "{frame}"
    );
    assert!(
        mac.frames.try_recv().is_err(),
        "la parada respondió dos veces"
    );
    assert_eq!(
        mac.runtime
            .state
            .durable
            .notice_log
            .records()
            .iter()
            .filter(|notice| notice.code == NoticeCode::BridgeReplySent)
            .count(),
        1
    );
    Ok(())
}

#[tokio::test]
async fn closing_answers_pending_calendar_read_with_disconnection() -> Result<()> {
    let mut mac = mac(vec![event(1, OCTOBER_MS, "work")]).await?;
    mac.runtime
        .device_request_raw(&json!({
            "request_id":"calendar-during-close", "capability":"calendar_read",
            "parameters":{"start_ms":OCTOBER_MS,"end_ms":OCTOBER_MS + DAY_MS}
        }))
        .await?;
    assert!(mac.runtime.features.calendar.deimos.is_some());
    mac.runtime
        .close_for_exit(
            orbit_core::privacy_stop::StopCause::Shutdown,
            "Orbit se está cerrando.",
        )
        .await;
    let frame = mac.frames.try_recv().context("device.reply absent")?;
    assert_eq!(frame["params"]["request_id"], "calendar-during-close");
    assert_eq!(
        frame["params"]["refusal"]["code"], "mac_disconnected",
        "{frame}"
    );
    assert!(mac.frames.try_recv().is_err(), "closing replied twice");
    Ok(())
}

#[tokio::test]
async fn revocation_answers_pending_calendar_read_like_privacy_stop() -> Result<()> {
    let mut mac = mac(vec![event(1, OCTOBER_MS, "work")]).await?;
    mac.runtime
        .device_request_raw(&json!({
            "request_id":"calendar-during-revocation", "capability":"calendar_read",
            "parameters":{"start_ms":OCTOBER_MS,"end_ms":OCTOBER_MS + DAY_MS}
        }))
        .await?;
    assert!(mac.runtime.features.calendar.deimos.is_some());
    mac.runtime
        .apply(orbit_core::Event::Connect(orbit_core::Connection::Revoked))
        .await?;
    let frame = mac.frames.try_recv().context("device.reply absent")?;
    assert_eq!(frame["params"]["request_id"], "calendar-during-revocation");
    assert_eq!(
        frame["params"]["refusal"]["code"], "privacy_stopped",
        "{frame}"
    );
    assert!(mac.frames.try_recv().is_err(), "revocation replied twice");
    Ok(())
}

#[tokio::test]
async fn ref303b_r2_calendar_refusal_notice_keeps_cause_in_parentheses() -> Result<()> {
    let mut mac = mac(vec![]).await?;
    mac.runtime
        .device_request_raw(&json!({
            "request_id":"calendar-notice", "capability":"calendar_read",
            "parameters":{"start_ms":OCTOBER_MS,"end_ms":OCTOBER_MS + DAY_MS}
        }))
        .await?;
    mac.runtime
        .deimos_failed_refusal(orbit_core::refusal::Refusal::PrivacyStopped { since_ms: None });
    assert_eq!(
        mac.runtime.state.notice_code(),
        Some(NoticeCode::BridgeReplySent)
    );
    let frame = mac.frames.try_recv()?;
    assert_eq!(frame["params"]["refusal"]["code"], "privacy_stopped");
    Ok(())
}

#[tokio::test]
async fn ref403_a4_invalid_calendar_fields_keep_their_names_on_modern_wire() -> Result<()> {
    let mut mac = mac(vec![]).await?;
    for (request_id, parameters, field) in [
        ("missing-window", json!({"start_ms":OCTOBER_MS}), "end_ms"),
        (
            "bad-cursor",
            json!({"start_ms":OCTOBER_MS,"end_ms":OCTOBER_MS + DAY_MS,"after_index":"3"}),
            "after_index",
        ),
    ] {
        let reply = mac.ask(request_id, parameters).await?;
        assert_eq!(reply["refusal"]["code"], "invalid_parameter");
        assert_eq!(reply["refusal"]["expected"]["field"], field);
    }
    Ok(())
}

async fn mac(agenda: Vec<Value>) -> Result<Mac> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut helper) = fixture(directory.path())?;
    said::trace(&mut runtime, directory.path());
    seed(&mut runtime, &mut helper).await?;
    // REF-11: no Orbit-side switch is left, and the computer's own macOS
    // permissions are withheld here on purpose: the calendar answers to its
    // permission alone (CAL-03 A7, REF-10 A1).
    {
        let fixture_value = serde_json::json!(orbit_core::PermissionState::Denied);
        crate::daemon::authority_fixture::set(
            &mut runtime.state,
            "capabilities.screen_state",
            fixture_value,
        );
    }
    {
        let fixture_value = serde_json::json!(orbit_core::PermissionState::Denied);
        crate::daemon::authority_fixture::set(
            &mut runtime.state,
            "capabilities.accessibility_state",
            fixture_value,
        );
    }
    let (hub, frames) = mpsc::channel(64);
    runtime.hub.install(hub);
    Ok(Mac {
        _directory: directory,
        runtime,
        helper,
        frames,
        agenda,
        helper_page: 1_000,
        windows_read: 0,
        helper_left_out: Vec::new(),
        left_out: Vec::new(),
    })
}

#[tokio::test]
async fn malformed_calendar_number_reaches_deimos_with_event_field_and_type() -> Result<()> {
    let mut mac = mac(vec![]).await?;
    mac.runtime
        .device_request_raw(&json!({
            "request_id":"bad-calendar-number", "capability":"calendar_read",
            "parameters":{"start_ms":OCTOBER_MS,"end_ms":OCTOBER_MS + DAY_MS}
        }))
        .await?;
    let command = mac
        .helper
        .try_recv()
        .context("calendar_snapshot no enviada")?;
    assert_eq!(command["command"], "calendar_snapshot");
    let mut result = page(
        &command["calendar_query"],
        vec![],
        0,
        "44444444-4444-4444-8444-444444444444",
    );
    result["observed_at_unix_ms"] = json!("ahora");
    ack(&mut mac.runtime, &command, result).await?;
    let reply = mac
        .frames
        .try_recv()
        .context("negativa de agenda ausente")?;
    assert_eq!(reply["params"]["request_id"], "bad-calendar-number");
    assert_eq!(reply["params"]["refusal"]["code"], "helper_reply_invalid");
    assert_eq!(reply["params"]["refusal"]["field"], "observed_at_unix_ms");
    assert_eq!(reply["params"]["refusal"]["received_type"], "string");
    assert_eq!(
        reply["params"]["refusal"]["expected"]["name"],
        "native_v1/calendar_snapshot"
    );
    Ok(())
}

impl Mac {
    async fn ask(&mut self, id: &str, parameters: Value) -> Result<Value> {
        self.runtime
            .device_request_raw(&json!({
                "request_id": id,
                "capability": "calendar_read",
                "parameters": parameters,
            }))
            .await?;
        self.serve().await?;
        let frame = self
            .frames
            .try_recv()
            .context("el Mac tiene que contestar; el silencio es la respuesta equivocada")?;
        assert_eq!(frame["method"], "device.reply");
        assert_eq!(frame["params"]["request_id"], id);
        let bytes = serde_json::to_string(&frame)?.len();
        assert!(
            bytes < crate::hub::MAX_OUTGOING_FRAME_BYTES,
            "una respuesta de {bytes} bytes no sale del Mac"
        );
        Ok(frame["params"].clone())
    }

    /// Answers every helper command waiting, as the synthetic helper.
    async fn serve(&mut self) -> Result<()> {
        while let Ok(command) = self.helper.try_recv() {
            match command["command"].as_str() {
                Some("calendar_snapshot") => {
                    let query = &command["calendar_query"];
                    let (start, end) = (
                        query["start_unix_ms"].as_i64().unwrap_or(0),
                        query["end_unix_ms"].as_i64().unwrap_or(0),
                    );
                    let inside: Vec<Value> = self
                        .agenda
                        .iter()
                        .filter(|event| {
                            let (from, to) = (
                                event["start_unix_ms"].as_i64().unwrap_or(0),
                                event["end_unix_ms"].as_i64().unwrap_or(0),
                            );
                            from < end && (to > start || from == to && from >= start)
                        })
                        .cloned()
                        .collect();
                    let after = query["after_index"].as_u64().unwrap_or(0) as usize;
                    let slice: Vec<Value> = inside
                        .iter()
                        .skip(after)
                        .take(self.helper_page)
                        .cloned()
                        .collect();
                    if after == 0 {
                        self.windows_read += 1;
                    }
                    let mut value = page(
                        query,
                        slice,
                        inside.len(),
                        "44444444-4444-4444-8444-444444444444",
                    );
                    // Pages of one window follow each other in the helper's clock.
                    if !self.helper_left_out.is_empty() {
                        value["oversized"] = json!(self.helper_left_out);
                    }
                    let offset = MACH_STEP * (after as u64 + 1);
                    value["capture_started_monotonic_ns"] = json!(super::tests::MACH + offset);
                    value["capture_completed_monotonic_ns"] =
                        json!(super::tests::MACH + offset + 100);
                    ack(&mut self.runtime, &command, value).await?;
                }
                Some("calendar_sources") => {
                    ack(&mut self.runtime, &command, calendars_page(3)).await?;
                }
                Some("calendar_disconnect") => {}
                other => bail!("orden inesperada al helper: {other:?}"),
            }
        }
        Ok(())
    }

    /// The whole walk, as the translator does it. Returns the joined events.
    async fn walk(&mut self, start_ms: i64, end_ms: i64) -> Result<(Vec<Value>, usize)> {
        let mut parameters = json!({"start_ms": start_ms, "end_ms": end_ms});
        let mut events: Vec<Value> = Vec::new();
        let mut pages = 0;
        loop {
            pages += 1;
            let reply = self
                .ask(&format!("walk-{pages}"), parameters.clone())
                .await?;
            assert!(reply["refusal"].is_null(), "{reply}");
            let result = &reply["result"];
            for words in [
                "range_fully_read",
                "unread_from_unix_ms",
                "max_events_per_read",
                "max_window_ms",
            ] {
                assert!(result.get(words).is_none(), "{words} in {result}");
            }
            if let Some(left_out) = result.get("oversized").and_then(Value::as_array) {
                self.left_out.extend(left_out.iter().cloned());
            }
            for item in result["events"].as_array().context("events")? {
                let continues = events
                    .last()
                    .is_some_and(|last| last.get(INVITEES_CONTINUED) == Some(&Value::Bool(true)));
                if continues {
                    let Some(last) = events.last_mut() else {
                        bail!("fragmento sin cabeza")
                    };
                    assert_eq!(last["observation_id"], item["observation_id"]);
                    let more = item["invitees"].as_array().cloned().unwrap_or_default();
                    if let Some(list) = last["invitees"].as_array_mut() {
                        list.extend(more);
                    }
                    if item.get(INVITEES_CONTINUED).is_none()
                        && let Some(fields) = last.as_object_mut()
                    {
                        fields.remove(INVITEES_CONTINUED);
                    }
                } else {
                    events.push(item.clone());
                }
            }
            let Some(after_ms) = result["next_after_ms"].as_i64() else {
                return Ok((events, pages));
            };
            parameters = json!({
                "start_ms": start_ms,
                "end_ms": end_ms,
                "read_id": result["read_id"],
                "after_ms": after_ms,
                "after_index": result["next_after_index"],
            });
            if let Some(invitee) = result.get("next_after_invitee") {
                parameters["after_invitee"] = invitee.clone();
            }
            assert!(pages < 10_000, "a walk that never ends");
        }
    }
}

const MACH_STEP: u64 = 1_000;

fn realistic(index: usize, start: i64, guests: usize) -> Value {
    let mut value = event(
        index,
        start,
        ["work", "personal", "family", "holidays"][index % 4],
    );
    value["title"] = json!(format!(
        "Reunión de seguimiento del proyecto {} con el equipo — revisión semanal",
        index % 17
    ));
    value["event_timezone"] = json!("Europe/Madrid");
    value["interpreted_timezone"] = json!("Europe/Madrid");
    value["is_floating"] = json!(false);
    value["event_id"] = json!(format!("A1B2C3D4-0000-4000-8000-{index:012}:EVT"));
    value["calendar_item_id"] = json!(format!("5E6F7A8B-{index:04}-4C2D-9E0F-123456789ABC"));
    value["last_modified_unix_ms"] = json!(start - 7 * DAY_MS);
    value["invitees_status"] = json!("provided");
    value["invitees"] = json!(
        (0..guests)
            .map(|guest| json!({
                "name": format!("Persona Invitada {guest}"),
                "participant_url": format!("mailto:persona.invitada{guest}@empresa.example"),
                "email": format!("persona.invitada{guest}@empresa.example"),
                "role": "required",
                "response_status": "accepted",
                "participant_type": "person",
                "is_current_user": guest == 0,
            }))
            .collect::<Vec<_>>()
    );
    if index.is_multiple_of(3) {
        value["event_url"] = json!(format!("https://meet.example.com/abc-defg-{index:03}"));
        value["event_url_status"] = json!("provided");
    }
    value
}

/// REF-10 A2: a month with a busy agenda — three hundred realistic events —
/// in one question, every event once, in frames the link carries.
#[tokio::test]
async fn ref10_a2_a_busy_month_arrives_whole_in_one_question() -> Result<()> {
    let agenda: Vec<Value> = (0..300)
        .map(|index| {
            realistic(
                index,
                OCTOBER_MS + (index as i64) * 2 * 3_600_000,
                index % 6,
            )
        })
        .collect();
    let mut mac = mac(agenda.clone()).await?;
    let (events, pages) = mac.walk(OCTOBER_MS, OCTOBER_MS + 31 * DAY_MS).await?;
    assert_eq!(events.len(), 300);
    let bytes: usize = agenda
        .iter()
        .map(|event| serde_json::to_vec(event).map_or(0, |value| value.len()))
        .sum();
    assert_eq!(pages, 4, "{pages} páginas para {bytes} bytes");
    assert_eq!(mac.windows_read, 1, "a month is one window");
    Ok(())
}

/// REF-10 A2: a whole year, still one question; windows and pages are Orbit's
/// business, and the answer never says it stopped short.
#[tokio::test]
async fn ref10_a2_a_year_is_one_question_too() -> Result<()> {
    let agenda: Vec<Value> = (0..400)
        .map(|index| {
            event(
                index,
                OCTOBER_MS + (index as i64) * DAY_MS * 365 / 400,
                "work",
            )
        })
        .collect();
    let mut mac = mac(agenda).await?;
    let (events, _) = mac.walk(OCTOBER_MS, OCTOBER_MS + 365 * DAY_MS).await?;
    assert_eq!(events.len(), 400);
    assert_eq!(mac.windows_read, 12, "365 días en ventanas de 31");
    Ok(())
}

/// REF-10 A3: one day with more events than fit one frame arrives whole: the
/// helper pages the window and the daemon pages the answer.
#[tokio::test]
async fn ref10_a3_a_day_fuller_than_one_frame_arrives_whole() -> Result<()> {
    let agenda: Vec<Value> = (0..900)
        .map(|index| realistic(index, OCTOBER_MS + (index as i64) * 60_000, 2))
        .collect();
    let mut mac = mac(agenda).await?;
    mac.helper_page = 250;
    let (events, pages) = mac.walk(OCTOBER_MS, OCTOBER_MS + DAY_MS).await?;
    assert_eq!(events.len(), 900);
    assert!(pages > 3, "{pages}");
    Ok(())
}

/// REF-10 A3: a meeting whose own invitees do not fit one frame travels in
/// fragments and is joined; no guest is dropped.
#[tokio::test]
async fn ref10_a3_a_meeting_with_thousands_of_guests_is_not_trimmed() -> Result<()> {
    let mut agenda = vec![realistic(0, OCTOBER_MS, 1_500)];
    agenda.push(realistic(1, OCTOBER_MS + 3_600_000, 1));
    let mut mac = mac(agenda).await?;
    let (events, pages) = mac.walk(OCTOBER_MS, OCTOBER_MS + DAY_MS).await?;
    assert_eq!(events.len(), 2);
    assert_eq!(events[0]["invitees"].as_array().map(Vec::len), Some(1_500));
    assert!(events[0].get(INVITEES_CONTINUED).is_none());
    assert!(pages > 1);
    Ok(())
}

#[tokio::test]
async fn ref10_a8_each_missing_condition_is_named() -> Result<()> {
    for (authorization, code) in [
        (Some("denied"), "permission_denied"),
        (Some("not_determined"), "permission_denied"),
        (None, "permission_unknown"),
    ] {
        let mut mac = mac(vec![]).await?;
        match authorization {
            Some(value) => {
                mac.runtime
                    .native_event(host_state(2, false, status(value)))
                    .await?;
            }
            None => {
                mac.runtime
                    .native_event(json!({"event":"ready","capabilities":["calendar"]}))
                    .await?;
                mac.runtime
                    .native_event(json!({"event":"state","native_instance_id":HOST,"privacy_generation":2,"privacy_stopped":false}))
                    .await?;
            }
        }
        while mac.helper.try_recv().is_ok() {}
        let reply = mac
            .ask(
                "sin-permiso",
                json!({"start_ms": OCTOBER_MS, "end_ms": OCTOBER_MS + DAY_MS}),
            )
            .await?;
        assert_eq!(reply["refusal"]["code"], code, "{reply}");
        assert_eq!(reply["refusal"]["permission"], "calendar");
        assert!(reply["result"].is_null());
        assert!(mac.helper.try_recv().is_err(), "nothing was read");
    }
    Ok(())
}
/// REF-10 A9: withdrawing the permission while a read waits refuses it by
/// name, and the next question gets no events.
#[tokio::test]
async fn ref10_a9_withdrawing_the_permission_cuts_the_read() -> Result<()> {
    let mut mac = mac(vec![event(1, OCTOBER_MS, "work")]).await?;
    mac.runtime
        .device_request_raw(
            &json!({"request_id": "en-curso", "capability": "calendar_read",
            "parameters": {"start_ms": OCTOBER_MS, "end_ms": OCTOBER_MS + DAY_MS}}),
        )
        .await?;
    let read = mac.helper.try_recv()?;
    assert_eq!(read["command"], "calendar_snapshot");
    mac.runtime
        .native_event(host_state(2, false, status("denied")))
        .await?;
    let frame = mac.frames.try_recv()?;
    assert_eq!(frame["params"]["request_id"], "en-curso");
    assert_eq!(
        frame["params"]["refusal"]["code"], "permission_denied",
        "{frame}"
    );
    assert_eq!(
        frame["params"]["refusal"]["permission"], "calendar",
        "{frame}"
    );
    // The helper's late answer delivers nothing to anyone.
    reject(&mut mac.runtime, &read, "calendar_permission_required").await?;
    assert!(mac.frames.try_recv().is_err());
    let next = mac
        .ask(
            "despues",
            json!({"start_ms": OCTOBER_MS, "end_ms": OCTOBER_MS + DAY_MS}),
        )
        .await?;
    assert_eq!(next["refusal"]["code"], "permission_denied");
    assert_eq!(next["refusal"]["permission"], "calendar");
    Ok(())
}

/// A cursor the Mac does not recognise — its reader restarted, or another
/// read replaced the walk — is refused by a code the translator restarts on;
/// a malformed one names its fields.
#[tokio::test]
async fn a_lost_walk_and_a_malformed_cursor_are_refused_by_name() -> Result<()> {
    let mut mac = mac(vec![event(1, OCTOBER_MS, "work")]).await?;
    let lost = mac
        .ask("perdida", json!({"start_ms": OCTOBER_MS, "end_ms": OCTOBER_MS + DAY_MS,
            "read_id": "7a1b0000-0000-4000-8000-00000000dead", "after_ms": OCTOBER_MS, "after_index": 3}))
        .await?;
    assert_eq!(lost["refusal"]["code"], "read_interrupted");
    assert_eq!(lost["refusal"]["cause"]["kind"], "calendar_read_restarted");
    let malformed = mac
        .ask(
            "rota",
            json!({"start_ms": OCTOBER_MS, "end_ms": OCTOBER_MS + DAY_MS, "after_index": "3"}),
        )
        .await?;
    assert_eq!(malformed["refusal"]["code"], "invalid_parameter");
    assert_eq!(
        malformed["refusal"]["expected"],
        json!({"kind":"wire_field", "field":"after_index", "expected":"nonnegative_integer", "received_type":"string"})
    );
    Ok(())
}

/// REF-12 third review: the helper cutting capture is not renamed on the
/// device bridge. The translator knows this code and decides whether to retry.
#[tokio::test]
async fn a_capture_cut_in_flight_reaches_deimos_as_capture_stopped() -> Result<()> {
    let mut mac = mac(vec![event(1, OCTOBER_MS, "work")]).await?;
    mac.runtime
        .device_request_raw(&json!({
            "request_id": "capture-cut",
            "capability": "calendar_read",
            "parameters": {"start_ms": OCTOBER_MS, "end_ms": OCTOBER_MS + DAY_MS},
        }))
        .await?;
    mac.runtime
        .native_event(
            json!({"event":"capture_stopped","reason":"session_inactive",
            "privacy_stopped":false,"drain_complete":false}),
        )
        .await?;
    let frame = mac.frames.try_recv().context("device.reply absent")?;
    assert_eq!(frame["method"], "device.reply");
    assert_eq!(frame["params"]["request_id"], "capture-cut");
    assert_eq!(
        frame["params"]["refusal"]["code"], "read_interrupted",
        "{frame}"
    );
    assert_eq!(
        frame["params"]["refusal"]["cause"]["kind"], "capture_stopped",
        "{frame}"
    );
    Ok(())
}

#[tokio::test]
async fn a_capture_cut_between_pages_names_the_cut_not_a_reader_restart() -> Result<()> {
    let agenda: Vec<Value> = (0..300)
        .map(|index| realistic(index, OCTOBER_MS + index as i64 * 60_000, 2))
        .collect();
    let mut mac = mac(agenda).await?;
    let first = mac
        .ask(
            "first",
            json!({"start_ms": OCTOBER_MS,
        "end_ms": OCTOBER_MS + DAY_MS}),
        )
        .await?;
    let page = &first["result"];
    let after_ms = page["next_after_ms"]
        .as_i64()
        .context("first page must continue")?;
    mac.runtime
        .native_event(json!({"event":"capture_stopped",
        "reason":"session_inactive","privacy_stopped":false,"drain_complete":true}))
        .await?;
    let next = mac
        .ask(
            "second",
            json!({"start_ms": OCTOBER_MS,
        "end_ms": OCTOBER_MS + DAY_MS,"read_id":page["read_id"],
        "after_ms":after_ms,"after_index":page["next_after_index"]}),
        )
        .await?;
    assert_eq!(next["refusal"]["code"], "read_interrupted", "{next}");
    assert_eq!(
        next["refusal"]["cause"]["kind"], "capture_stopped",
        "{next}"
    );
    Ok(())
}

/// The store changed while a window was read in pages: that window is read
/// again from its first page and the answer holds one consistent read.
#[tokio::test]
async fn a_store_change_mid_window_reads_it_again_before_answering() -> Result<()> {
    let agenda: Vec<Value> = (0..10)
        .map(|index| event(index, OCTOBER_MS + index as i64 * 60_000, "work"))
        .collect();
    let mut mac = mac(agenda).await?;
    mac.helper_page = 4;
    mac.runtime
        .device_request_raw(
            &json!({"request_id": "cambio", "capability": "calendar_read",
            "parameters": {"start_ms": OCTOBER_MS, "end_ms": OCTOBER_MS + DAY_MS}}),
        )
        .await?;
    // First page of the window, then the store changes, then the rest.
    let first = mac.helper.try_recv()?;
    let query = first["calendar_query"].clone();
    let slice: Vec<Value> = mac.agenda[..4].to_vec();
    ack(
        &mut mac.runtime,
        &first,
        page(&query, slice, 10, "44444444-4444-4444-8444-444444444444"),
    )
    .await?;
    mac.runtime
        .native_event(json!({"event":"calendar_invalidated","code":"calendar_invalidated"}))
        .await?;
    mac.serve().await?;
    let frame = mac.frames.try_recv()?;
    let events = frame["params"]["result"]["events"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert_eq!(events.len(), 10);
    assert_eq!(
        mac.windows_read, 1,
        "the window was read again from its first page"
    );
    Ok(())
}

/// CAL-06, kept: the range the translator sends is the one this Mac reads,
/// and the continuation it sends parses — types included.
#[tokio::test]
async fn cal06_the_contracted_parameters_are_read_as_the_translator_sends_them() -> Result<()> {
    let document: Value = serde_json::from_str(CONTRACT)?;
    assert_eq!(document["schema"], "orbit.device-bridge-request.v1");
    let first = document["requests"]
        .as_array()
        .and_then(|entries| {
            entries
                .iter()
                .find(|entry| entry["capability"] == "calendar_read")
        })
        .context("el contrato no dice nada de calendar_read")?["parameters"]
        .clone();
    let following: Vec<Value> = document["continuations"]
        .as_array()
        .context("el contrato no dice cómo sigue una lectura")?
        .iter()
        .filter(|entry| entry["capability"] == "calendar_read")
        .map(|entry| entry["parameters"].clone())
        .collect();
    // After whole events, and inside one event's invitees (REF-10 A3).
    assert_eq!(following.len(), 2);
    assert!(
        following
            .iter()
            .any(|entry| entry.get("after_invitee").is_some())
    );
    let typed_first: orbit_core::wire::bridge::CalendarReadParameters =
        serde_json::from_value(first.clone())?;
    assert!(
        Cursor::from_parameters(&typed_first)
            .map_err(anyhow::Error::msg)?
            .is_none()
    );
    for entry in &following {
        let parameters: orbit_core::wire::bridge::CalendarReadParameters =
            serde_json::from_value(entry.clone())?;
        assert!(
            Cursor::from_parameters(&parameters)
                .map_err(anyhow::Error::msg)?
                .is_some()
        );
    }

    let mut mac = mac(vec![]).await?;
    mac.runtime
        .device_request_raw(
            &json!({"request_id": "cal06", "capability": "calendar_read",
            "parameters": first.clone()}),
        )
        .await?;
    let read = mac.helper.try_recv()?;
    assert_eq!(read["calendar_query"]["start_unix_ms"], first["start_ms"]);
    assert_eq!(read["calendar_query"]["end_unix_ms"], first["end_ms"]);
    Ok(())
}

/// Decision 4 (revisora, REF-10): a field is not a reason to leave an event out
/// while the event fits a page. A 5 KB title among fifty ordinary events used
/// to stop the whole read with `calendar_limit_exceeded` (the bound was 2,048
/// bytes); now it arrives with the other fifty.
#[tokio::test]
async fn ref10_d4_a_five_kilobyte_title_among_fifty_events_arrives_with_them() -> Result<()> {
    let mut agenda: Vec<Value> = (0..50)
        .map(|index| realistic(index, OCTOBER_MS + (index as i64) * 3_600_000, 2))
        .collect();
    let mut long = realistic(50, OCTOBER_MS + 50 * 3_600_000, 2);
    long["title"] = json!("Revisión trimestral ".repeat(256));
    let title_bytes = long["title"].as_str().map_or(0, str::len);
    assert!(title_bytes > 5_000, "{title_bytes}");
    agenda.push(long);
    let mut mac = mac(agenda).await?;
    let (events, _) = mac.walk(OCTOBER_MS, OCTOBER_MS + 31 * DAY_MS).await?;
    assert_eq!(events.len(), 51);
    assert!(
        events
            .iter()
            .any(|event| event["title"].as_str().map(str::len) == Some(title_bytes))
    );
    assert!(mac.left_out.is_empty(), "{:?}", mac.left_out);
    Ok(())
}

/// Decision 4: an event that cannot fit a page on its own — here its title
/// and its URL together pass the 96 KiB a bridge answer carries — is left
/// out, and the answer says so itself: its dates, its identifier, which field
/// and how many bytes against how many fit. The other fifty arrive.
#[tokio::test]
async fn ref10_d4_an_event_that_cannot_fit_a_page_alone_is_named_and_the_rest_arrive() -> Result<()>
{
    let mut agenda: Vec<Value> = (0..50)
        .map(|index| realistic(index, OCTOBER_MS + (index as i64) * 3_600_000, 2))
        .collect();
    let start = OCTOBER_MS + 20 * 3_600_000 + 60_000;
    let mut huge = realistic(99, start, 1);
    huge["title"] = json!("t".repeat(60 * 1024));
    huge["event_url"] = json!(format!("https://example.com/{}", "u".repeat(50 * 1024)));
    huge["event_url_status"] = json!("provided");
    let identifier = huge["calendar_item_id"].clone();
    agenda.push(huge);
    let mut mac = mac(agenda).await?;
    let (events, _) = mac.walk(OCTOBER_MS, OCTOBER_MS + 31 * DAY_MS).await?;
    assert_eq!(events.len(), 50);
    assert!(
        events
            .iter()
            .all(|event| event["calendar_item_id"] != identifier)
    );
    assert_eq!(mac.left_out.len(), 1, "{:?}", mac.left_out);
    let named = &mac.left_out[0];
    assert_eq!(named["calendar_item_id"], identifier);
    assert_eq!(named["start_unix_ms"], start);
    assert_eq!(named["field"], "title");
    let (bytes, limit) = (
        named["bytes"].as_u64().unwrap_or(0),
        named["limit"].as_u64().unwrap_or(u64::MAX),
    );
    assert!(bytes > limit, "{named}");
    assert_eq!(
        limit,
        u64::try_from(orbit_core::device_bridge::MAX_CALENDAR_PAGE_BYTES)?
    );
    Ok(())
}

/// Decision 4: what the helper itself could not carry (a field past its bound)
/// reaches Deimos named the same way, beside the events that did travel.
#[tokio::test]
async fn ref10_d4_what_the_helper_left_out_reaches_the_answer_by_name() -> Result<()> {
    let agenda: Vec<Value> = (0..50)
        .map(|index| realistic(index, OCTOBER_MS + (index as i64) * 3_600_000, 2))
        .collect();
    let mut mac = mac(agenda).await?;
    mac.helper_left_out = vec![json!({
        "calendar": {"calendar_id": "work", "source_id": "account"},
        "calendar_item_id": "item-500",
        "start_unix_ms": OCTOBER_MS + 3 * DAY_MS,
        "end_unix_ms": OCTOBER_MS + 3 * DAY_MS + 3_600_000,
        "field": "title",
        "bytes": 120_000,
        "limit": 98_304,
    })];
    let (events, _) = mac.walk(OCTOBER_MS, OCTOBER_MS + 31 * DAY_MS).await?;
    assert_eq!(events.len(), 50);
    assert_eq!(mac.left_out, mac.helper_left_out);
    Ok(())
}
