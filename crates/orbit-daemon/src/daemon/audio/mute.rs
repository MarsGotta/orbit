//! «Silenciar»: the microphone's holder muted, as the helper confirmed it.
//!
//! Mute is a property of whoever holds the microphone, never a lifecycle of
//! its own: the meeting recording or the call with the assistant. The
//! helper replaces the stream's samples with silence before they leave it
//! (same frames, same timing), so what is said while muted is never stored,
//! transcribed or sent, «Oye Deimos» hears nothing, and the recording's
//! tracks keep their time. The capture lane keeps serving those frames: who
//! holds the microphone does not change.
//!
//! The daemon shows «silenciado» only once the helper's answer says so. An
//! answer that does not (a refusal, a helper that does not know the order)
//! is said with its code and the microphone stays as it was shown. A meeting
//! keeps its spans in its durable state and its closure; a call keeps the
//! fact until it hangs up. Each new recording or call starts unmuted.
use super::super::{Runtime, now};
use anyhow::{Context, Result};
use orbit_core::microphone_mute;
use orbit_core::notice::{Notice, NoticeCode, NoticeKey};
use orbit_core::refusal::{Operation, Refusal};
use orbit_core::wire::native::{NativeCommandHeader, NativeResponseFields, decode_result_body};
use orbit_core::{CallPhase, CapturePhase};
use serde::Deserialize;
use serde_json::json;

/// What the daemon knows of the mute beyond the recording's spans.
#[derive(Default)]
pub(in crate::daemon) struct MicrophoneMute {
    /// The call whose microphone the helper confirmed muted.
    call: Option<String>,
    /// The order the helper has not answered yet.
    order: Option<MuteOrder>,
    /// A meeting's microphone started muted (a resume), until its receipt
    /// confirms it.
    start: Option<String>,
}

struct MuteOrder {
    id: String,
    muted: bool,
    stream: String,
    holder: Holder,
}

/// Who holds the microphone that is muted: a recording or a call, by id.
#[derive(Clone, PartialEq, Eq)]
enum Holder {
    Meeting(String),
    Call(String),
}

/// The helper's answer to `audio_mute`: the state it applied.
#[derive(Deserialize)]
struct MuteReceipt {
    stream_id: String,
    muted: bool,
}

/// The helper's answer to a microphone `audio_start`: `muted` only from a
/// helper that started it muted.
#[derive(Deserialize)]
struct StartReceipt {
    #[serde(default)]
    muted: Option<bool>,
}

impl Runtime {
    /// The live holder of the microphone: the call while it lasts, otherwise
    /// a recording not yet ended.
    fn mute_holder(&self) -> Option<Holder> {
        if self.capture.call_runtime.is_some() {
            return self
                .state
                .durable
                .call
                .as_ref()
                .map(|call| Holder::Call(call.id.clone()));
        }
        self.state
            .durable
            .recording
            .as_ref()
            .filter(|recording| recording.ended_at_ms.is_none())
            .map(|recording| Holder::Meeting(recording.id.clone()))
    }

    /// The call's microphone is muted, as the helper confirmed it.
    pub(in crate::daemon) fn call_microphone_muted(&self) -> bool {
        let muted = self.capture.audio.mute.call.as_deref();
        muted.is_some()
            && muted
                == self
                    .state
                    .durable
                    .call
                    .as_ref()
                    .map(|call| call.id.as_str())
    }

    /// The recording's microphone is muted: its last span is still open.
    pub(in crate::daemon) fn meeting_microphone_muted(&self) -> bool {
        self.state
            .durable
            .recording
            .as_ref()
            .is_some_and(microphone_mute::recording_muted)
    }

    /// Whatever holds the microphone now has it muted.
    pub(in crate::daemon) fn microphone_muted(&self) -> bool {
        match self.mute_holder() {
            Some(Holder::Call(_)) => self.call_microphone_muted(),
            Some(Holder::Meeting(_)) => self.meeting_microphone_muted(),
            None => false,
        }
    }

    /// «Silenciar» or «Reactivar micrófono»: the order goes to the helper;
    /// the state changes when its answer confirms it (`mute_response`).
    pub(in crate::daemon) async fn mute_microphone(&mut self, muted: bool) -> Result<()> {
        let holder = self.mute_holder().context(
            "No hay ninguna grabación ni llamada con el micrófono abierto que silenciar",
        )?;
        let listening = match &holder {
            Holder::Meeting(_) => self
                .state
                .durable
                .recording
                .as_ref()
                .is_some_and(|recording| recording.phase == CapturePhase::Recording),
            Holder::Call(_) => self
                .state
                .durable
                .call
                .as_ref()
                .is_some_and(|call| call.phase != CallPhase::Starting),
        };
        if !listening {
            anyhow::bail!("El micrófono aún no graba: espera a que empiece para silenciarlo");
        }
        if matches!(holder, Holder::Meeting(_)) && self.state.durable.voice_active {
            anyhow::bail!("Termina la nota de voz antes de silenciar el micrófono");
        }
        if self.capture.audio.mute.order.is_some() {
            return Err(Refusal::Busy {
                operation: Operation::Audio,
                retry_after_ms: None,
            }
            .into());
        }
        if self.microphone_muted() == muted {
            return Ok(());
        }
        if muted
            && matches!(holder, Holder::Meeting(_))
            && let Some(recording) = self.state.durable.recording.as_ref()
        {
            microphone_mute::may_mute(&recording.muted_spans)?;
        }
        if !self.state.capabilities().native_helper() {
            return Err(Refusal::HelperNotRunning.into());
        }
        let stream = self
            .capture
            .mic_stream
            .clone()
            .context("El micrófono no tiene ningún flujo abierto")?;
        let command =
            crate::native::command("audio_mute", json!({"stream_id": stream, "muted": muted}))?;
        let id = NativeCommandHeader::from_value(&command)?.id.to_owned();
        self.record_pending_native(&command);
        self.native_send_command(command).await?;
        self.capture.audio.mute.order = Some(MuteOrder {
            id,
            muted,
            stream,
            holder,
        });
        Ok(())
    }

    /// The helper's answer to an `audio_mute` order. Returns whether it was
    /// one. Only an answer that names the stream and the state asked for
    /// changes anything; any other is said with its code.
    pub(in crate::daemon) fn mute_response(&mut self, response: &NativeResponseFields) -> bool {
        let order = match self.capture.audio.mute.order.take() {
            Some(order) if order.id == response.id => order,
            other => {
                self.capture.audio.mute.order = other;
                return false;
            }
        };
        let confirmed = response.ok
            && response
                .result
                .as_ref()
                .and_then(|result| decode_result_body::<MuteReceipt>("audio_mute", result).ok())
                .is_some_and(|receipt| {
                    receipt.muted == order.muted && receipt.stream_id == order.stream
                });
        if !confirmed {
            let code = response
                .error
                .as_ref()
                .map_or("mute_not_confirmed", |error| error.code.as_str());
            self.say(
                Notice::new(NoticeCode::MicrophoneMuteRefused).with_token(NoticeKey::Code, code),
            );
            return true;
        }
        if self.mute_holder().as_ref() != Some(&order.holder) {
            // The recording or the call ended while the helper answered: its
            // microphone is gone with it, and the next one starts unmuted.
            self.log(
                "microphone_mute_after_holder",
                &[("muted", flag(order.muted))],
            );
            return true;
        }
        self.set_mute(&order.holder, order.muted);
        true
    }

    /// The helper went away or started again: what it was asked about the
    /// mute has no answer, and a new order may be sent.
    pub(in crate::daemon) fn forget_mute_orders(&mut self) {
        self.capture.audio.mute.order = None;
        self.capture.audio.mute.start = None;
    }

    /// The muted state the helper confirmed, kept where its holder keeps it.
    fn set_mute(&mut self, holder: &Holder, muted: bool) {
        match holder {
            Holder::Call(id) => self.capture.audio.mute.call = muted.then(|| id.clone()),
            Holder::Meeting(_) => {
                let Some(recording) = self.state.durable.recording.as_mut() else {
                    return;
                };
                let at_ms = now().saturating_sub(recording.started_at_ms);
                if muted {
                    if let Err(refused) = microphone_mute::mute(&mut recording.muted_spans, at_ms) {
                        // Checked before the order left; only a second order
                        // could reach it, and one waits for the other.
                        self.log(
                            "microphone_mute_span_refused",
                            &[("spans", &refused.spans.to_string())],
                        );
                    }
                } else {
                    microphone_mute::unmute(&mut recording.muted_spans, at_ms);
                }
                if let Err(error) = self.store_actor.defer_snapshot(&self.state) {
                    self.log(
                        "microphone_mute_not_saved",
                        &[("error", &error.to_string())],
                    );
                }
            }
        }
        self.log(
            if muted {
                "microphone_muted"
            } else {
                "microphone_unmuted"
            },
            &[],
        );
    }

    /// The muted spans of `recording_id`'s closure, each with its end (the
    /// open one ends with the recording, `length_ms` from its start); `None`
    /// when it was never muted, so its closure reads as before.
    pub(in crate::daemon) fn closure_muted_spans(
        &self,
        recording_id: &str,
        length_ms: i64,
    ) -> Option<Vec<microphone_mute::MutedSpan>> {
        self.state
            .durable
            .recording
            .as_ref()
            .filter(|recording| recording.id == recording_id && !recording.muted_spans.is_empty())
            .map(|recording| microphone_mute::closed(&recording.muted_spans, length_ms))
    }

    /// Whether a meeting's microphone starting now starts muted: the
    /// recording was muted when it paused or was cut.
    pub(in crate::daemon) fn microphone_starts_muted(&self) -> bool {
        self.capture.call_runtime.is_none() && self.meeting_microphone_muted()
    }

    /// A muted start was sent under `id`; its receipt must say `muted:true`.
    pub(in crate::daemon) fn await_muted_start(&mut self, id: &str) {
        self.capture.audio.mute.start = Some(id.to_owned());
    }

    /// The receipt of a muted microphone start. A helper that started it
    /// without saying `muted:true` sends her voice: the recording is shown
    /// unmuted from now on, and she is told why.
    pub(in crate::daemon) fn muted_start_receipt(&mut self, response: &NativeResponseFields) {
        if self.capture.audio.mute.start.as_deref() != Some(response.id.as_str()) {
            return;
        }
        self.capture.audio.mute.start = None;
        let Some(result) = response.result.as_ref().filter(|_| response.ok) else {
            return;
        };
        let started_muted = decode_result_body::<StartReceipt>("audio_start", result)
            .ok()
            .and_then(|receipt| receipt.muted)
            == Some(true);
        if started_muted {
            return;
        }
        if let Some(holder) = self.mute_holder() {
            self.set_mute(&holder, false);
        }
        self.say(
            Notice::new(NoticeCode::MicrophoneMuteRefused)
                .with_token(NoticeKey::Code, "muted_start_not_confirmed"),
        );
    }
}

fn flag(value: bool) -> &'static str {
    if value { "true" } else { "false" }
}

#[cfg(test)]
mod receipt_fixture_tests {
    use super::*;

    #[test]
    fn the_mute_result_fixture_uses_the_mute_reader() -> Result<()> {
        let result: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../../contracts/fixtures/native/results/audio_mute.json"
        ))?;
        let receipt: MuteReceipt = decode_result_body("audio_mute", &result)?;
        assert_eq!(receipt.stream_id, "stream-1");
        assert!(receipt.muted);
        Ok(())
    }
}
