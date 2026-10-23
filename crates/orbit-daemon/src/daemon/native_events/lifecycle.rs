//! The helper's own lifecycle: it came up, it stopped capturing or latched
//! the user's stop, it went away, or it reported an error of its own.
use super::super::*;
use orbit_core::notice::{Notice, NoticeCode, NoticeKey};
use serde::Deserialize;

#[derive(Deserialize)]
struct NativeStopFields<'a> {
    reason: Option<&'a str>,
    privacy_stopped: bool,
}

#[derive(Deserialize)]
struct NativeStateStop {
    privacy_stopped: bool,
}

#[derive(Deserialize)]
struct NativeErrorCode<'a> {
    code: &'a str,
    os_domain: Option<&'a str>,
    os_status: Option<i64>,
}

#[derive(Deserialize)]
struct NativeHelperDisconnect<'a> {
    #[serde(default)]
    cause: Option<&'a str>,
}

/// The bridge's audio cause uses fixed labels and a schema field shorter than
/// 64 bytes; 256 bytes leaves room for its count. This bounds only the notice
/// copied from an internal disconnect event, not native wire input or capture.
const MAX_NATIVE_AUDIO_CAUSE_BYTES: usize = 256;

impl Runtime {
    pub(super) async fn on_native_ready(&mut self, event: &Value) -> Result<()> {
        self.cancel_sources_with_cause("helper_restarted", "El helper se reinició antes de entregar la respuesta anterior; no se conoce aún la instancia nueva.").await?;
        // A request into the folders the old helper held has no answer.
        self.files_helper_restarted();
        self.end_helper_waits();
        let retired_call_starts = self.audio.retired_call_starts.len();
        let retired_stops = self.audio.retired_stops.len();
        if retired_call_starts > 0 || retired_stops > 0 {
            self.log(
                "retired_capture_debts_helper_restarted",
                &[
                    ("retired_call_starts", &retired_call_starts.to_string()),
                    ("retired_stops", &retired_stops.to_string()),
                ],
            );
            self.audio.retired_call_starts.clear();
            self.audio.retired_stops.clear();
        }
        self.state = decide(&self.state, Event::HelperReady, now())?.state;
        // A new helper plays nothing the old one was playing.
        self.assistant.read_aloud = None;
        // A new helper says where its «Oye Deimos» detector is after `ready`,
        // and listens on nothing until asked again.
        self.question.voice_trigger_unknown();
        self.question.wake_helper_ready();
        // A new helper says who holds the microphone after `ready`; until
        // then it is not known.
        self.call_signal_lost().await;
        self.observe_voice_capabilities(event)?;
        // The user's stop reaches a helper that came back without its
        // latch, so both say the same. A revocation
        // never writes the helper's durable latch.
        if orbit_core::privacy_stop::latches_helper(&self.state) {
            self.native_send("privacy_stop", json!({})).await?;
        }
        self.native_send("status", json!({})).await?;
        // Ajustes says again whether each folder opens.
        self.check_assistant_folders();
        Ok(())
    }

    pub(super) async fn on_native_state(&mut self, event: &Value) -> Result<()> {
        self.observe_voice_capabilities(event)?;
        // The helper's `state` is the one exact word about its
        // durable latch: set means the user's stop is held there, and
        // both sides must say the same.
        let state = NativeStateStop::deserialize(event)?;
        if state.privacy_stopped && !self.state.privacy_stopped() {
            self.privacy_stop(orbit_core::privacy_stop::UserStop::HelperLatch { code: None })
                .await;
        }
        Ok(())
    }

    pub(super) async fn on_native_capture_stopped(&mut self, event: &Value) -> Result<()> {
        let stop = NativeStopFields::deserialize(event)?;
        let code = stop.reason.unwrap_or("unreported");
        let echo = orbit_core::privacy_stop::DAEMON_ECHO_CODES.contains(&code);
        if orbit_core::privacy_stop::latches(code) && !echo {
            if !self.state.privacy_stopped() {
                self.privacy_stop(orbit_core::privacy_stop::UserStop::Helper {
                    code: code.to_owned(),
                })
                .await;
            }
        } else if stop.privacy_stopped || orbit_core::privacy_stop::latches(code) {
            if !self.state.privacy_stopped() {
                self.privacy_stop(orbit_core::privacy_stop::UserStop::HelperLatch {
                    code: Some(code.to_owned()),
                })
                .await;
            }
        } else {
            self.helper_capture_stop(code).await;
            // The call's application closed after its call.
            self.call_capture_cut(code).await;
        }
        Ok(())
    }

    pub(super) async fn on_native_local_rearm(&mut self) -> Result<()> {
        self.apply(Event::LocalRearm).await?;
        self.calendar.read_failure = None;
        self.connect();
        Ok(())
    }

    pub(super) async fn on_native_helper_disconnected(&mut self, event: &Value) -> Result<()> {
        let cause = NativeHelperDisconnect::deserialize(event)?;
        // Gone, and nothing has started again yet.
        self.files_helper_lost(orbit_core::refusal::Refusal::HelperNotRunning);
        self.end_helper_waits();
        // The line that explains a menu bar icon vanishing without a
        // crash report, which is otherwise invisible after the fact.
        self.log(
            "helper_disconnected",
            &[(
                "recording",
                if self.state.durable.capture.recording.is_some() {
                    "true"
                } else {
                    "false"
                },
            )],
        );
        self.state = decide(&self.state, Event::HelperLost, now())?.state;
        self.call_signal_lost().await;
        self.question.voice_trigger_unknown();
        self.question.helper_listens = false;
        self.clear_audio_sources();
        for source in ["mic_local", "remote_app"] {
            self.note_bound_audio_gap(source)?;
        }
        let text = match cause.cause {
            Some(detail)
                if detail.starts_with("native_audio_invalid event=audio field=")
                    && detail.len() <= MAX_NATIVE_AUDIO_CAUSE_BYTES =>
            {
                format!(
                    "Orbit cortó el enlace con su helper nativo en este Mac porque el audio \
                     llegaba mal formado ({detail})."
                )
            }
            Some(_) | None => "Se perdió el helper nativo de Orbit en este Mac.".to_owned(),
        };
        self.contain(orbit_core::privacy_stop::StopCause::HelperLost, &text)
            .await;
        Ok(())
    }

    pub(super) async fn on_native_error(&mut self, event: &Value) -> Result<()> {
        let error = NativeErrorCode::deserialize(event)?;
        let mut notice =
            Notice::new(NoticeCode::HelperError).with_token(NoticeKey::Code, error.code);
        if let Some(domain) = error.os_domain {
            notice = notice.with_token(NoticeKey::Cause, &domain.to_ascii_lowercase());
        }
        if let Some(status) = error.os_status {
            notice = notice.with(NoticeKey::Status, status);
        }
        self.say(notice);
        Ok(())
    }
}
