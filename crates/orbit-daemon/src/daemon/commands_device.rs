//! The app's orders to the device: pairing, status, error reports, the
//! user's stop, exit, rearm and the system's permissions.
use super::*;

impl Runtime {
    /// Routes an order to the device.
    pub(super) async fn device_order(&mut self, command: Command) -> Result<()> {
        match command {
            Command::Pair {
                hub_url,
                code,
                device_name,
            } => self.pair_command(hub_url, code, device_name).await?,
            Command::Status => self.status_command()?,
            Command::ErrorReports { enabled } => self.set_error_reports(enabled)?,
            // Already in the app's report log: it only goes to the client.
            Command::Report { report } => self
                .error_reports
                .report(&report, std::time::Instant::now()),
            Command::PrivacyStop => {
                self.privacy_stop(orbit_core::privacy_stop::UserStop::Order)
                    .await
            }
            Command::Shutdown => self.shutdown_command().await?,
            Command::Rearm => self.rearm_command().await?,
            Command::Permissions { request } => self.permissions_command(request).await?,
            off_loop_orders!()
            | assistant_orders!()
            | recording_orders!()
            | note_orders!()
            | agenda_orders!() => return Err(super::misrouted()),
        }
        Ok(())
    }

    /// Pairing retires what the old pairing was asked first.
    pub(super) async fn pair_command(
        &mut self,
        hub_url: String,
        code: String,
        device_name: String,
    ) -> Result<()> {
        self.retire_device_request_reads().await?;
        self.pair_device(hub_url, code, device_name).await?;
        Ok(())
    }

    /// Apply receipts that already exist, but never wait here for a future
    /// disk receipt: this command runs inside the select loop that owes the
    /// IPC response. The projection still reports only durable seals; a later
    /// Status observes later receipts.
    pub(super) fn status_command(&mut self) -> Result<()> {
        self.collect_ready_seals(sealing::OPPORTUNISTIC_RECEIPT_WAIT, false)?;
        Ok(())
    }

    /// The service asked Orbit to exit: the capture is cut and Orbit leaves
    /// only once nothing is left to store.
    pub(super) async fn shutdown_command(&mut self) -> Result<()> {
        self.close_for_exit(
            orbit_core::privacy_stop::StopCause::Shutdown,
            "Orbit se está cerrando a petición del servicio.",
        )
        .await;
        self.retry_seals()?;
        // The single pending list is the only gate; every vertical
        // that still holds work is named through `has_any_pending`.
        let pending = self.has_any_pending();
        if !pending.is_empty() {
            bail!(
                "Orbit no sale todavía: queda pendiente de guardar {}; la captura ya está cortada y lo recibido se conserva",
                pending.join(", ")
            );
        }
        Ok(())
    }

    /// The user's rearm, from their own socket on this Mac.
    pub(super) async fn rearm_command(&mut self) -> Result<()> {
        if self.state.capabilities().native_helper() {
            // Hand it to the helper rather than refusing: this command
            // arrived on this user's own 0600 socket on this Mac, which
            // is the presence the latch actually requires, and the menu
            // bar is only one way to supply it. The helper owns the
            // durable latch, so it answers with a `local_rearm` event
            // and that single path rearms the daemon. Remote traffic
            // still cannot reach this socket.
            self.native_send("rearm", json!({})).await?;
            return Ok(());
        }
        self.apply(Event::LocalRearm).await?;
        self.connect();
        Ok(())
    }

    /// The system's permissions; the calendar's is asked of the calendar alone.
    pub(super) async fn permissions_command(&mut self, request: Vec<String>) -> Result<()> {
        if request
            .iter()
            .any(|permission| permission == crate::permissions::CALENDARS)
        {
            if request != [crate::permissions::CALENDARS] {
                bail!("Solicita el permiso de calendario por separado de los demás permisos");
            }
            self.calendar_command(
                orbit_core::calendar_control::CalendarCommand::RequestPermission,
            )?;
            return Ok(());
        }
        self.native_send("permissions", json!({"request":request}))
            .await?;
        Ok(())
    }
}
