//! Deimos's reads of the agenda: any range, answered in pages.
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
//! Every page Deimos receives updates one local activity row for its walk
//! — the range, how many events and when — which decides nothing and asks
//! the user for nothing.
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

#[path = "deimos_pages.rs"]
mod pages;

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
        // The access check again at the moment of reading: the permission can change
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
                self.calendar.cut_deimos = None;
                self.calendar.deimos = Some(Walk::new(super::super::id(), range));
            }
            Some(cursor) => {
                let matches = self.calendar.deimos.as_ref().is_some_and(|walk| {
                    walk.request.is_none()
                        && walk.read_id == cursor.read_id
                        && walk.range == range
                        && walk.at_ms == cursor.after_ms
                        && walk.position == cursor.position
                });
                if !matches {
                    if self.calendar.cut_deimos.as_deref() == Some(&cursor.read_id) {
                        return Err(bridge_calendar_refusal("capture_stopped"));
                    }
                    return Err(bridge_calendar_refusal("calendar_read_restarted"));
                }
            }
        }
        if let Some(walk) = self.calendar.deimos.as_mut() {
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

    /// A read ended by name: whoever waits is told why, and the walk goes.
    pub(super) fn deimos_failed_refusal(&mut self, refusal: orbit_core::refusal::Refusal) {
        let Some(walk) = self.calendar.deimos.take() else {
            return;
        };
        let Some(request_id) = walk.request else {
            return;
        };
        if self
            .assistant
            .bridge_ask(crate::daemon::BridgeKind::Calendar)
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
            self.assistant
                .end_bridge_ask(crate::daemon::BridgeKind::Calendar);
        }
    }

    /// Drops the walk, answering a question still waiting on it with `code`.
    pub(super) fn drop_deimos_walk(&mut self, code: &'static str, refusal: Refusal) {
        if let Some(walk) = self.calendar.deimos.as_ref() {
            self.calendar.cut_deimos = (code == "capture_stopped").then(|| walk.read_id.clone());
        }
        let waiting = self
            .calendar
            .deimos
            .as_ref()
            .is_some_and(|walk| walk.request.is_some());
        if waiting {
            self.deimos_failed_refusal(refusal);
        }
        self.calendar.deimos = None;
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
#[path = "deimos_tests.rs"]
mod field_validation_tests;
