//! A helper event on the loop, and how a failure handling it is contained.
use super::*;

impl Runtime {
    /// The actor loop's native branch: one helper event, and what a failure
    /// handling it does. Only a failure of the capture itself cuts what is
    /// live; any other vertical's failure is said and the recording goes on
    /// (a data fault never stops Orbit).
    pub(in crate::daemon) async fn handle_native_event(&mut self, event: Value) {
        let about = self.native_event_label(&event);
        let capture = self.native_event_touches_capture(&event);
        let audio = matches!(event["event"].as_str(), Some("audio" | "gap"));
        let source = event["source"].as_str().map(str::to_owned);
        if let Err(error) = self.native_event(event).await {
            if audio {
                self.drop_audio_event(source.as_deref(), &error.to_string());
            } else if capture {
                self.contain(
                    StopCause::CaptureEventFailed,
                    &format!("Fallo al tratar un evento de captura del Mac ({about}): {error}."),
                )
                .await;
            } else {
                self.contain(
                    StopCause::NativeEventFailed,
                    &format!(
                        "Fallo al tratar un evento del Mac que no es de la captura ({about}): \
                         {error}. No se corta nada."
                    ),
                )
                .await;
            }
        }
    }

    /// Whether a helper event is about the capture itself: audio and its
    /// gaps, the helper's lifecycle and stops, a capture hotkey, or the
    /// answer to a capture order. Read before the event is handled, while
    /// the order it answers is still pending.
    pub(in crate::daemon) fn native_event_touches_capture(&self, event: &Value) -> bool {
        let Ok(header) = NativeEventHeader::from_value(event) else {
            return false;
        };
        match header.event {
            "audio"
            | "gap"
            | "ready"
            | "state"
            | "privacy_stop"
            | "capture_stopped"
            | "helper_disconnected"
            | "local_rearm"
            | "hotkey" => true,
            "response" => NativeResponseIdRef::from_value(event)
                .ok()
                .is_some_and(|response| {
                    event
                        .get("_bridge_command")
                        .and_then(Value::as_str)
                        .or_else(|| self.capture_response_command(response.id))
                        .is_some_and(capture_command)
                }),
            _ => false,
        }
    }

    /// Resolve a capture response from the capture owners first. The generic
    /// `pending_native` index is capped at 64 diagnostic entries and evicts;
    /// absence there is unknown, never evidence that another vertical owns an
    /// ACK still named by a capture registry.
    pub(crate) fn capture_response_command(&self, command_id: &str) -> Option<&str> {
        if self
            .audio
            .pending_starts
            .iter()
            .any(|pending| pending == command_id)
            || self.audio.retired_call_starts.contains_key(command_id)
            || self.audio.call_start_request_pending(command_id)
        {
            return Some("audio_start");
        }
        self.audio
            .lifecycle
            .response_command(command_id)
            .or_else(|| self.helper.pending.get(command_id).map(String::as_str))
    }

    /// A short name for a helper event in a notice: its kind, or the order a
    /// response answers. The helper's own strings are only echoed when they
    /// look like a code, never as free text.
    pub(in crate::daemon) fn native_event_label(&self, event: &Value) -> String {
        let code = |value: &str| {
            if !value.is_empty()
                && value.len() <= 40
                && value
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte == b'_')
            {
                value.to_owned()
            } else {
                "sin nombre".to_owned()
            }
        };
        let Ok(header) = NativeEventHeader::from_value(event) else {
            return "sin nombre".into();
        };
        match header.event {
            "response" => {
                let answered = NativeResponseIdRef::from_value(event)
                    .ok()
                    .and_then(|response| {
                        event
                            .get("_bridge_command")
                            .and_then(Value::as_str)
                            .or_else(|| self.capture_response_command(response.id))
                    })
                    .map_or_else(|| "una orden desconocida".to_owned(), code);
                format!("respuesta a {answered}")
            }
            other => code(other),
        }
    }
}
