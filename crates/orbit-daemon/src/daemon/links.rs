//! Native and hub send paths.
use super::*;
use orbit_core::notice::{Notice, NoticeCode, NoticeKey};
use orbit_core::wire::native::NativeCommandHeader;
use serde::Deserialize;

#[derive(Deserialize)]
struct HubMethod<'a> {
    method: &'a str,
}

impl Runtime {
    pub(super) async fn start_track(
        &mut self,
        source: &str,
        stream: &str,
        audio: Option<orbit_core::protocol::MeetingAudio>,
    ) -> Result<()> {
        let pid = match audio {
            Some(orbit_core::protocol::MeetingAudio::Application { application_pid }) => {
                Some(application_pid)
            }
            Some(orbit_core::protocol::MeetingAudio::System)
            | Some(orbit_core::protocol::MeetingAudio::MicrophoneOnly)
            | None => None,
        };
        let mut payload = if source == "remote_app" {
            match audio {
                Some(orbit_core::protocol::MeetingAudio::System) => {
                    json!({"source":source,"stream_id":stream,"capture_scope":"system"})
                }
                Some(orbit_core::protocol::MeetingAudio::Application { .. }) => {
                    json!({"source":source,"stream_id":stream,"application_pid":pid,"capture_scope":"application"})
                }
                Some(orbit_core::protocol::MeetingAudio::MicrophoneOnly) | None => {
                    bail!("La pista remota necesita un ámbito de captura")
                }
            }
        } else {
            json!({"source":source,"stream_id":stream,
                "purpose": if audio.is_some() { "meeting" } else { "voice_note" }})
        };
        // A meeting resumed with «Silenciar» on starts muted.
        let muted = source == "mic_local" && audio.is_some() && self.microphone_starts_muted();
        if muted {
            payload["muted"] = json!(true);
        }
        let command = native::command("audio_start", payload)?;
        self.record_pending_native(&command);
        let header = NativeCommandHeader::from_value(&command)?;
        if muted {
            self.await_muted_start(header.id);
        }
        self.capture.pending_starts.push(header.id.into());
        self.native
            .send(command)
            .await
            .context("Helper nativo desconectado")?;
        Ok(())
    }
    /// Records that `command` was sent to the helper under `id`, so a late
    /// refusal can be classified by command name without parsing the id. Drops
    /// the oldest entry at capacity so the table stays bounded.
    pub(super) fn record_pending_native(&mut self, command: &Value) {
        let Ok(header) = NativeCommandHeader::from_value(command) else {
            return;
        };
        let (id, name) = (header.id, header.command);
        if self.pending_native.len() >= MAX_PENDING_NATIVE
            && !self.pending_native.contains_key(id)
            && let Some(oldest) = self.pending_native.keys().next().cloned()
        {
            self.pending_native.remove(&oldest);
        }
        self.pending_native.insert(id.to_owned(), name.to_owned());
    }
    pub(super) async fn native_send(&mut self, name: &str, fields: Value) -> Result<()> {
        if !self.state.capabilities().native_helper() {
            return Ok(());
        }
        let command = native::command(name, fields)?;
        self.native_send_command(command).await
    }

    /// Sends a built command with `native_send`'s deadline, for a caller
    /// that needs its id first (`audio_wake_phrase`).
    pub(super) async fn native_send_command(&mut self, command: Value) -> Result<()> {
        if !self.state.capabilities().native_helper() {
            return Ok(());
        }
        let name = NativeCommandHeader::from_value(&command)?
            .command
            .to_owned();
        let name = name.as_str();
        self.record_pending_native(&command);
        // Every helper send carries the 500 ms deadline. A stalled
        // helper must not stall the actor; the deadline classifies the loss
        // with the same taxonomy the rest of the actor already speaks.
        match tokio::time::timeout(Duration::from_millis(500), self.native.send(command)).await {
            Ok(Ok(())) => Ok(()),
            Err(_elapsed) => match name {
                "audio_stop" | "capture_stop" | "privacy_stop" => {
                    // The helper did not take a stop within the deadline. A
                    // full channel proves backpressure, not that Orbit.app is
                    // absent, so capability stays as observed. The boxed await
                    // breaks the `contain → effect → native_send →
                    // contain` future-size recursion.
                    let message = format!(
                        "El helper no aceptó la orden {name} en 500 ms; el canal sigue presente, pero esa orden no se entregó."
                    );
                    let cause = if name == "privacy_stop" {
                        orbit_core::privacy_stop::StopCause::PrivacyStopUndelivered
                    } else {
                        orbit_core::privacy_stop::StopCause::CaptureStopUndelivered
                    };
                    Box::pin(self.contain(cause, &message)).await;
                    Err(anyhow::anyhow!(message))
                }
                other => {
                    // Non-critical command: additive notice, capture keeps
                    // going. Mirrors how a saturated hub command degrades.
                    let message = format!(
                        "El helper no confirmó {other} en 500 ms; la orden se descarta sin detener la captura"
                    );
                    self.say(
                        Notice::new(NoticeCode::HelperCommandUnconfirmed)
                            .with_token(NoticeKey::Step, other),
                    );
                    Err(anyhow::anyhow!(message))
                }
            },
            Ok(Err(_closed)) => Err(anyhow::anyhow!("Helper nativo desconectado")),
        }
    }
    pub(super) async fn send(&mut self, frame: Value) -> Result<()> {
        self.send_with_stop_window(frame, false).await
    }

    async fn send_with_stop_window(&mut self, frame: Value, stop_window: bool) -> Result<()> {
        // `chat.interrupt` withdraws an already live turn; it is the one
        // outbound command the user's newly latched stop must still deliver.
        let interrupt =
            HubMethod::deserialize(&frame).is_ok_and(|header| header.method == "chat.interrupt");
        if self.state.privacy_stopped() && !(stop_window && interrupt) {
            bail!("Todo está parado");
        }
        let result = self.send_frame(Ok(frame));
        if result.delivered_to_queue() {
            Ok(())
        } else {
            let cause = result.cause();
            self.log("hub_send_failed", &[("cause", &cause.to_string())]);
            self.say(NoticeCode::HubSendFailed);
            bail!("{cause}")
        }
    }
    pub(super) fn recording_active(&self) -> bool {
        self.state
            .durable
            .recording
            .as_ref()
            .is_some_and(|r| matches!(r.phase, CapturePhase::Recording | CapturePhase::Starting))
    }

    pub(super) async fn interrupt_turn(&mut self, turn_id: Option<&str>, stop_window: bool) {
        if let Some(turn_id) = turn_id {
            self.remember_interrupted_turn(turn_id);
            if self.hub.is_some() {
                let command_id = id();
                let frame = hub::command(
                    &command_id,
                    "chat.interrupt",
                    json!({"session_id":self.state.session_id(),"turn_id":turn_id}),
                );
                let result = match frame {
                    Ok(frame) => self.send_with_stop_window(frame, stop_window).await,
                    Err(error) => Err(error),
                };
                if let Err(error) = result {
                    self.interrupt_not_delivered(turn_id, &error.to_string());
                } else {
                    self.await_interrupt_ack(&command_id, turn_id);
                }
            }
        }
    }
}
