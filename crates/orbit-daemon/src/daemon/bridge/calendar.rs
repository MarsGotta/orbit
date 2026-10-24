//! Deimos's agenda reads: EventKit, for these dates, now, for every
//! calendar, answered in pages by the calendar's own reader
//! (`calendar/deimos.rs`). This is only where a read starts, or is refused
//! at once.
use super::BridgeAsk;
use crate::daemon::Runtime;
use orbit_core::refusal::{Expected, Parameter, Refusal};

impl Runtime {
    /// Every way this can fail is named here and now rather than left to the
    /// caller's timeout, because "no answer" about a calendar reads as
    /// "nothing in your diary" and that is the one wrong answer.
    pub(super) async fn serve_calendar_read(&mut self, ask: &BridgeAsk) -> Result<(), Refusal> {
        let (start_ms, end_ms) = ask.calendar_window.ok_or(Refusal::InvalidParameter {
            parameter: Parameter::Request,
            expected: Expected::ValidSelection,
        })?;
        match self.calendar_bridge_read(&ask.id, start_ms, end_ms, ask.calendar_cursor.clone()) {
            Ok(()) => Ok(()),
            Err(refusal) => {
                self.assistant.bridge_requests.take(ask.kind());
                self.refuse_bridge_admission(&ask.id, &refusal)
                    .await
                    .map_err(|_| Refusal::HubUnreachable { since_ms: None })
            }
        }
    }
}
