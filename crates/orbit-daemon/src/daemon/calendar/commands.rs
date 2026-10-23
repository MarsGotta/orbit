use anyhow::{Result, bail};
use orbit_core::calendar_control::CalendarCommand as Request;

use super::*;

impl Runtime {
    /// What the app (or the CLI) asks of the agenda: the permission's state,
    /// the macOS permission itself, or a read of the range on screen. There is
    /// nothing to list, select or disconnect, and a read waits its turn
    /// instead of being refused because another one is running.
    pub(in crate::daemon) fn calendar_command(&mut self, request: Request) -> Result<()> {
        // A write is refused by field and with its numbers, or queued.
        let refusal = request.write_refusal();
        match request {
            Request::Save { request_id, save } => {
                return self.calendar_write_command(
                    request_id,
                    super::WriteRequest::Save(save),
                    refusal,
                );
            }
            Request::Delete { request_id, delete } => {
                return self.calendar_write_command(
                    request_id,
                    super::WriteRequest::Delete(delete),
                    refusal,
                );
            }
            Request::RequestPermission | Request::Read { .. } => {}
        }
        request.validate()?;
        if !self.state.capabilities().native_helper()
            || !self.features.native_host.calendar_backend_available()
        {
            bail!("El lector de calendarios del Mac no está disponible ahora mismo");
        }
        let calendar = &mut self.features.calendar;
        match request {
            // Handled above.
            Request::Save { .. } | Request::Delete { .. } => {}
            Request::RequestPermission => calendar.wanted_permission = true,
            Request::Read { window } => {
                if self.state.privacy_stopped() {
                    bail!("Todo está parado: Orbit no lee tu agenda hasta que lo reactives");
                }
                // Remembered, so the agenda reappears by itself when the
                // permission arrives or the calendar changes.
                calendar.complete = None;
                calendar.read_failure = None;
                calendar.screen_range = Some(window);
                if calendar.view.calendars.is_none() {
                    calendar.wanted_inventory = true;
                }
                if orbit_core::device_bridge::CalendarPermission::from_authorization(
                    calendar.view.authorization,
                ) == orbit_core::device_bridge::CalendarPermission::NotGranted
                {
                    calendar.screen = None;
                } else {
                    calendar.screen = Some(ScreenRead::new(window));
                }
            }
        }
        self.pump_calendar();
        Ok(())
    }
}
