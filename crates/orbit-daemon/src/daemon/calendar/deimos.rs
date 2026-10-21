//! Deimos's reads of the agenda: any range, answered in pages. REF-10.
//!
//! Deimos asks the translator once; the translator walks this Mac's pages and
//! joins them (`server/orbit_mcp/calendar.py`). A page is as many whole events
//! as fit one hub frame (`MAX_CALENDAR_PAGE_BYTES`), taken from the windows
//! this walk has read so far, and says where the next one starts. The window
//! being paged is kept in RAM between two requests of the same walk so that
//! every page of it comes from the same EventKit read; it is dropped as soon
//! as the walk moves past it. Nothing else is kept, and nothing is sent to the
//! hub but the answer.
//!
//! A7: every page Deimos receives updates one local activity row for its walk
//! — the range, how many events and when — which decides nothing and asks
//! Mars for nothing.
use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};

use anyhow::Result;
use orbit_core::refusal::{Expected, Parameter, Refusal};
use orbit_core::{
    calendar::{CalendarOccurrence, CalendarWindow, READ_WINDOW_MS, WindowRead},
    device_bridge::{
        CalendarPage, CalendarPosition, INVITEES_CONTINUED, MAX_CALENDAR_PAGE_BYTES, calendar_page,
    },
    topics::Knowledge,
    wire::bridge::{
        BridgeResult, CalendarBridgeEvent, CalendarOmission, CalendarReadResult, CalendarRequested,
        DeviceReplyParams,
    },
};
use serde_json::{Value, json};

use super::*;
use crate::hub;

fn bridge_calendar_refusal(code: &'static str) -> orbit_core::refusal::Refusal {
    Runtime::bridge_refusal_code(code)
}

fn invalid_calendar_event_field(event: &Value) -> String {
    let Some(fields) = event.as_object() else {
        return "structure".into();
    };
    for required in [
        "observation_id",
        "calendar",
        "calendar_item_id",
        "start_unix_ms",
        "end_unix_ms",
        "interpreted_timezone",
        "is_floating",
        "is_all_day",
        "is_detached",
        "title",
        "status",
        "invitees_status",
        "event_url_status",
    ] {
        if !fields.contains_key(required) {
            return required.into();
        }
    }
    let mut occurrence = event.clone();
    if let Some(fields) = occurrence.as_object_mut() {
        fields.remove(INVITEES_CONTINUED);
    }
    let Ok(encoded) = serde_json::to_vec(&occurrence) else {
        return "structure".into();
    };
    let mut decoder = serde_json::Deserializer::from_slice(&encoded);
    let Err(error) = serde_path_to_error::deserialize::<_, CalendarOccurrence>(&mut decoder) else {
        return INVITEES_CONTINUED.into();
    };
    let path = error.path().to_string();
    let message = error.inner().to_string();
    if let Some((_, rest)) = message.split_once("missing field `")
        && let Some((field, _)) = rest.split_once('`')
    {
        return if path == "." {
            field.into()
        } else {
            format!("{path}.{field}")
        };
    }
    if path != "." {
        return path;
    }
    message
        .split_once("unknown field `")
        .map(|(_, rest)| rest)
        .and_then(|rest| rest.split_once('`'))
        .map_or_else(
            || "structure".into(),
            |(field, _)| {
                if field.len() <= 64
                    && field
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
                {
                    field.into()
                } else {
                    "unrecognized_field".into()
                }
            },
        )
}

/// How long one answer may spend reading windows before it goes out as it is.
///
/// Protects: the hub's wait for one device request (30 s,
/// `server/orbit_hub/device_bridge.py` `REQUEST_TIMEOUT_SECONDS`). A sparse
/// range — years with few events — could need more window reads than fit that
/// wait before a page fills. Why ten seconds: a third of it, leaving room for one
/// more window read (at most five) and the round trip. What it is not: a limit
/// on the range. The answer ends early and says where to continue; the
/// translator asks for the rest without Deimos noticing.
pub(super) const PAGE_TIME_BUDGET: Duration = Duration::from_secs(10);

/// Where a read continues, as the translator echoes it from the last page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::daemon) struct Cursor {
    read_id: String,
    after_ms: i64,
    position: CalendarPosition,
}

impl Cursor {
    pub(in crate::daemon) fn from_parameters(
        parameters: &orbit_core::wire::bridge::CalendarReadParameters,
    ) -> Result<Option<Self>, Refusal> {
        let invalid = || Refusal::InvalidParameter {
            parameter: Parameter::Request,
            expected: Expected::CalendarCursor {
                read_id_bytes: parameters.read_id.as_ref().map(|id| id.len() as u64),
                max_read_id_bytes: orbit_core::wire::bridge::MAX_READ_ID_BYTES as u64,
                after_ms: parameters.after_ms.is_some(),
                after_index: parameters.after_index.is_some(),
                after_invitee: parameters.after_invitee.is_some(),
            },
        };
        let Some(read_id) = parameters.read_id.as_ref() else {
            return if parameters.after_ms.is_none()
                && parameters.after_index.is_none()
                && parameters.after_invitee.is_none()
            {
                Ok(None)
            } else {
                Err(invalid())
            };
        };
        if read_id.is_empty() || read_id.len() > orbit_core::wire::bridge::MAX_READ_ID_BYTES {
            return Err(invalid());
        }
        let (Some(after_ms), Some(index)) = (parameters.after_ms, parameters.after_index) else {
            return Err(invalid());
        };
        Ok(Some(Self {
            read_id: read_id.clone(),
            after_ms,
            position: CalendarPosition {
                index,
                invitee: parameters.after_invitee.unwrap_or(0),
            },
        }))
    }
}

/// The window a walk is paging, as EventKit returned it.
struct Cached {
    window: CalendarWindow,
    events: Vec<CalendarOccurrence>,
}

pub(in crate::daemon) struct Walk {
    read_id: String,
    range: CalendarWindow,
    /// The bridge request this walk is answering now; `None` between requests.
    request: Option<String>,
    /// The start of the window the next event comes from, and where in it.
    at_ms: i64,
    position: CalendarPosition,
    window_ms: i64,
    restarts: u8,
    cache: Option<Cached>,
    /// The answer being put together for `request`.
    answer: Vec<Value>,
    used: usize,
    omitted: BTreeMap<String, u32>,
    /// Events of the answer being put together that could not travel, named
    /// (decision 4): by the helper, or by the page they did not fit alone.
    oversized: Vec<orbit_core::calendar::CalendarOversizedEvent>,
    read_at: Option<i64>,
    timezone: Option<String>,
    started: Instant,
    windows_read_now: u32,
}

impl Walk {
    fn new(read_id: String, range: CalendarWindow) -> Self {
        Self {
            read_id,
            range,
            request: None,
            at_ms: range.start_unix_ms,
            position: CalendarPosition::default(),
            window_ms: READ_WINDOW_MS,
            restarts: 0,
            cache: None,
            answer: Vec::new(),
            used: 0,
            omitted: BTreeMap::new(),
            oversized: Vec::new(),
            read_at: None,
            timezone: None,
            started: Instant::now(),
            windows_read_now: 0,
        }
    }

    /// The window this walk waits for, if a request is waiting on it.
    pub(super) fn wanted_window(&self) -> Option<CalendarWindow> {
        if self.request.is_none()
            || self.at_ms >= self.range.end_unix_ms
            || self
                .cache
                .as_ref()
                .is_some_and(|cache| cache.window.start_unix_ms == self.at_ms)
        {
            return None;
        }
        Some(CalendarWindow {
            start_unix_ms: self.at_ms,
            end_unix_ms: self
                .at_ms
                .saturating_add(self.window_ms)
                .min(self.range.end_unix_ms),
        })
    }

    pub(super) fn note_restart(&mut self) -> u8 {
        self.restarts = self.restarts.saturating_add(1);
        self.restarts
    }

    /// EventKit ran out of time on a window `span` wide: ask for half of it.
    /// `false` when it is already as narrow as it gets.
    pub(super) fn halve_window(&mut self, span: i64) -> bool {
        if span <= MIN_WINDOW_MS {
            return false;
        }
        self.window_ms = (span / 2).max(MIN_WINDOW_MS);
        true
    }
}

enum Step {
    Answer { last: bool },
    Wait,
    Fail,
}

impl Runtime {
    /// A question from Deimos about `[start_ms, end_ms)`: the first page of a
    /// new walk, or the next page of one this Mac is walking.
    pub(in crate::daemon) fn calendar_bridge_read(
        &mut self,
        request_id: &str,
        start_ms: i64,
        end_ms: i64,
        cursor: Option<Cursor>,
    ) -> std::result::Result<(), Refusal> {
        // A6/A8 again at the moment of reading: the permission can change
        // between admission and here, and «no events» is the wrong way to say so.
        match self.calendar_access() {
            Knowledge::Present { .. } => {}
            Knowledge::Absent { refusal } => return Err(refusal),
            Knowledge::Unknown { .. } => {
                return Err(Refusal::PermissionUnknown {
                    permission: orbit_core::refusal::Permission::Calendar,
                });
            }
        }
        let range = CalendarWindow {
            start_unix_ms: start_ms,
            end_unix_ms: end_ms,
        };
        if range.validate().is_err() {
            return Err(Refusal::InvalidParameter {
                parameter: Parameter::Request,
                expected: Expected::CalendarRange {
                    start_ms,
                    end_ms,
                    min_ms: orbit_core::calendar::MIN_UNIX_MS,
                    max_ms: orbit_core::calendar::MAX_UNIX_MS,
                },
            });
        }
        match cursor {
            None => {
                // A new walk replaces one that was abandoned half way.
                self.features.calendar.cut_deimos = None;
                self.features.calendar.deimos = Some(Walk::new(super::super::id(), range));
            }
            Some(cursor) => {
                let matches = self.features.calendar.deimos.as_ref().is_some_and(|walk| {
                    walk.request.is_none()
                        && walk.read_id == cursor.read_id
                        && walk.range == range
                        && walk.at_ms == cursor.after_ms
                        && walk.position == cursor.position
                });
                if !matches {
                    if self.features.calendar.cut_deimos.as_deref() == Some(&cursor.read_id) {
                        return Err(bridge_calendar_refusal("capture_stopped"));
                    }
                    return Err(bridge_calendar_refusal("calendar_read_restarted"));
                }
            }
        }
        if let Some(walk) = self.features.calendar.deimos.as_mut() {
            walk.request = Some(request_id.to_owned());
            walk.answer.clear();
            walk.used = 0;
            walk.omitted.clear();
            walk.oversized.clear();
            walk.read_at = None;
            walk.timezone = None;
            walk.started = Instant::now();
            walk.windows_read_now = 0;
        }
        self.continue_deimos();
        self.pump_calendar();
        Ok(())
    }

    /// Fills the answer from the windows already read, and sends it when it is
    /// full, when the range is done or when it has taken long enough.
    fn continue_deimos(&mut self) {
        self.continue_deimos_with(calendar_page);
    }

    // The same walk handles a real page and an encoder failure; injecting only
    // the encoder lets a test reach that branch without fake calendar fields.
    fn continue_deimos_with(
        &mut self,
        mut encode_page: impl FnMut(
            &[CalendarOccurrence],
            CalendarPosition,
            usize,
            bool,
        ) -> std::result::Result<CalendarPage, serde_json::Error>,
    ) {
        loop {
            let step = {
                let Some(walk) = self.features.calendar.deimos.as_mut() else {
                    return;
                };
                if walk.request.is_none() {
                    return;
                }
                if walk.at_ms >= walk.range.end_unix_ms {
                    Step::Answer { last: true }
                } else {
                    match walk.cache.as_ref() {
                        Some(cache) if cache.window.start_unix_ms == walk.at_ms => {
                            match encode_page(
                                &cache.events,
                                walk.position,
                                MAX_CALENDAR_PAGE_BYTES.saturating_sub(walk.used),
                                !walk.answer.is_empty(),
                            ) {
                                Err(_) => Step::Fail,
                                Ok(slice) => {
                                    walk.used = walk.used.saturating_add(slice.used);
                                    walk.answer.extend(slice.events);
                                    walk.oversized.extend(slice.oversized);
                                    match slice.next {
                                        Some(next) => {
                                            walk.position = next;
                                            Step::Answer { last: false }
                                        }
                                        None => {
                                            walk.at_ms = cache.window.end_unix_ms;
                                            walk.position = CalendarPosition::default();
                                            walk.cache = None;
                                            continue;
                                        }
                                    }
                                }
                            }
                        }
                        _ if walk.windows_read_now > 0
                            && walk.started.elapsed() >= PAGE_TIME_BUDGET =>
                        {
                            Step::Answer { last: false }
                        }
                        _ => Step::Wait,
                    }
                }
            };
            match step {
                Step::Answer { last } => self.send_deimos_page(last),
                Step::Wait => {}
                Step::Fail => self.deimos_failed_refusal(Refusal::EncodeFailed {
                    resource: orbit_core::refusal::Resource::Calendar,
                }),
            }
            return;
        }
    }

    /// One window of the walk, read whole. Events that began before it were
    /// already sent with the window where they start.
    pub(super) fn deimos_window_read(&mut self, mut read: WindowRead) -> Result<()> {
        let Some(walk) = self.features.calendar.deimos.as_mut() else {
            return Ok(());
        };
        if walk.request.is_none() || read.window.start_unix_ms != walk.at_ms {
            return Ok(());
        }
        if read.window.start_unix_ms > walk.range.start_unix_ms {
            read.keep_from(read.window.start_unix_ms);
        }
        walk.read_at.get_or_insert(read.observed_at_unix_ms);
        walk.timezone
            .get_or_insert_with(|| read.system_timezone.clone());
        walk.oversized.append(&mut read.oversized);
        for omission in read.omitted {
            let count = walk.omitted.entry(omission.reason).or_insert(0);
            *count = count.saturating_add(omission.count);
        }
        walk.restarts = 0;
        walk.windows_read_now = walk.windows_read_now.saturating_add(1);
        walk.cache = Some(Cached {
            window: read.window,
            events: read.events,
        });
        self.continue_deimos();
        Ok(())
    }

    fn send_deimos_page(&mut self, last: bool) {
        let Some(walk) = self.features.calendar.deimos.as_mut() else {
            return;
        };
        let events = std::mem::take(&mut walk.answer);
        let mut typed_events = Vec::with_capacity(events.len());
        for (index, event) in events.iter().enumerate() {
            match serde_json::from_value::<CalendarBridgeEvent>(event.clone()) {
                Ok(typed) => typed_events.push(typed),
                Err(_) => {
                    let field = invalid_calendar_event_field(event);
                    self.deimos_failed_refusal(orbit_core::refusal::Refusal::HelperReplyInvalid {
                        field,
                        expected: orbit_core::refusal::Expected::CalendarEvent {
                            position: index as u64,
                            rows: events.len() as u64,
                        },
                        received_type: "invalid_calendar_event".into(),
                    });
                    return;
                }
            }
        }
        let Some(request_id) = walk.request.take() else {
            return;
        };
        let oversized = std::mem::take(&mut walk.oversized);
        let result = CalendarReadResult {
            read_id: walk.read_id.clone(),
            requested: CalendarRequested {
                start_unix_ms: walk.range.start_unix_ms,
                end_unix_ms: walk.range.end_unix_ms,
            },
            // A3: when EventKit was actually read, not when anything was kept.
            read_at_unix_ms: walk.read_at,
            system_timezone: walk.timezone.clone(),
            provider_sync_status: "unknown".into(),
            enumeration: "bounded_local_read".into(),
            events_returned: events.len(),
            events: typed_events,
            omitted: walk
                .omitted
                .iter()
                .map(|(reason, count)| CalendarOmission {
                    reason: reason.clone(),
                    count: *count,
                })
                .collect(),
            oversized: (!oversized.is_empty()).then_some(oversized),
            next_after_ms: (!last).then_some(walk.at_ms),
            next_after_index: (!last).then_some(walk.position.index),
            next_after_invitee: (!last && walk.position.invitee > 0)
                .then_some(walk.position.invitee),
        };
        if last {
            self.features.calendar.deimos = None;
        }
        self.send_bridge_now(
            hub::command(
                &request_id,
                "device.reply",
                json!(DeviceReplyParams::answered(
                    request_id.clone(),
                    BridgeResult::CalendarRead(result)
                )),
            ),
            "la respuesta sobre tu agenda",
        );
        if self
            .conversation
            .bridge_request
            .as_ref()
            .is_some_and(|ask| ask.id == request_id)
        {
            self.conversation.bridge_request = None;
        }
    }

    /// A read ended by name: whoever waits is told why, and the walk goes.
    pub(super) fn deimos_failed_refusal(&mut self, refusal: orbit_core::refusal::Refusal) {
        let Some(walk) = self.features.calendar.deimos.take() else {
            return;
        };
        let Some(request_id) = walk.request else {
            return;
        };
        if self
            .conversation
            .bridge_request
            .as_ref()
            .is_some_and(|ask| ask.id == request_id)
        {
            self.send_bridge_now(
                hub::command(
                    &request_id,
                    "device.reply",
                    self.bridge_refusal_frame(&request_id, &refusal),
                ),
                refusal.code().name(),
            );
            self.conversation.bridge_request = None;
        }
    }

    /// Drops the walk, answering a question still waiting on it with `code`.
    pub(super) fn drop_deimos_walk(&mut self, code: &'static str, refusal: Refusal) {
        if let Some(walk) = self.features.calendar.deimos.as_ref() {
            self.features.calendar.cut_deimos =
                (code == "capture_stopped").then(|| walk.read_id.clone());
        }
        let waiting = self
            .features
            .calendar
            .deimos
            .as_ref()
            .is_some_and(|walk| walk.request.is_some());
        if waiting {
            self.deimos_failed_refusal(refusal);
        }
        self.features.calendar.deimos = None;
    }

    /// Queue admission is observable here; Deimos reception is not.
    fn send_bridge_now(
        &mut self,
        frame: anyhow::Result<Value>,
        subject: &str,
    ) -> crate::daemon::hub_link::SendResult {
        let result = self.send_frame(frame);
        self.note_bridge_send(&result, subject);
        result
    }
}

#[cfg(test)]
mod ref403_tests {
    use super::*;
    use crate::daemon::{audio::tests::fixture, calendar::tests::seed};
    use tokio::sync::mpsc;

    #[test]
    fn ref405b_invalid_nested_calendar_field_is_named() {
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
    async fn a6_a_page_encoding_error_uses_its_own_code_on_the_real_walk() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let (mut runtime, mut helper) = fixture(directory.path())?;
        seed(&mut runtime, &mut helper).await?;
        let (hub, mut frames) = mpsc::channel(16);
        runtime.hub.install(hub);
        runtime
            .device_request_raw(&json!({
                "request_id":"encode-failure", "capability":"calendar_read",
                "parameters":{"start_ms":1_790_000_000_000i64,"end_ms":1_790_086_400_000i64},
            }))
            .await?;
        let walk = runtime
            .features
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
    async fn ref405b_invalid_calendar_event_answers_before_retiring_the_walk() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let (mut runtime, mut helper) = fixture(directory.path())?;
        seed(&mut runtime, &mut helper).await?;
        let (hub, mut frames) = mpsc::channel(16);
        runtime.hub.install(hub);
        runtime
            .device_request_raw(&json!({
                "request_id":"invalid-event", "capability":"calendar_read",
                "parameters":{"start_ms":1_790_000_000_000i64,"end_ms":1_790_086_400_000i64},
            }))
            .await?;
        let walk = runtime
            .features
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
        assert!(runtime.features.calendar.deimos.is_none());
        Ok(())
    }

    #[test]
    fn a1_numbered_calendar_causes_keep_their_contract_bounds() {
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
}
