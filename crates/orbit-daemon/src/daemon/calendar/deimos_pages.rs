//! Deimos's calendar walk, page by page: the next window read, each page sent
//! within its time budget, and the walk continued or ended.
use super::*;

impl Runtime {
    /// Fills the answer from the windows already read, and sends it when it is
    /// full, when the range is done or when it has taken long enough.
    pub(super) fn continue_deimos(&mut self) {
        self.continue_deimos_with(calendar_page);
    }

    // The same walk handles a real page and an encoder failure; injecting only
    // the encoder lets a test reach that branch without fake calendar fields.
    pub(super) fn continue_deimos_with(
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
                let Some(walk) = self.calendar.deimos.as_mut() else {
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
    pub(in crate::daemon) fn deimos_window_read(&mut self, mut read: WindowRead) -> Result<()> {
        let Some(walk) = self.calendar.deimos.as_mut() else {
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

    pub(super) fn send_deimos_page(&mut self, last: bool) {
        let Some(walk) = self.calendar.deimos.as_mut() else {
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
            // When EventKit was actually read, not when anything was kept.
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
            self.calendar.deimos = None;
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
            "the agenda reply",
        );
        if self
            .assistant
            .bridge_ask(crate::daemon::BridgeKind::Calendar)
            .is_some_and(|ask| ask.id == request_id)
        {
            self.assistant
                .end_bridge_ask(crate::daemon::BridgeKind::Calendar);
        }
    }
}
