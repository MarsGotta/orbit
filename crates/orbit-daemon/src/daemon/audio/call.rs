//! The call's own audio: its voice turns on the call's microphone, and the
//! stops owed to streams a late call start opened after the call was over.
//! A call opens its own stream with voice processing; no meeting or note
//! shares it (`validate_audio_transition`).
use super::super::Runtime;
use super::drain::{AUDIO_DRAIN_LIMIT, AUDIO_SILENCE_TIMEOUT, FullStop};
use super::voice::VoiceCancellationCause;
use crate::native;
use anyhow::Result;
use orbit_core::CapturePhase;
use orbit_core::wire::native::NativeCommandHeader;
use serde_json::{Value, json};
use std::time::Duration;
use tokio::time::Instant;

/// Bounds the receipt ledger for selective stops after retired starts. The
/// helper command channel holds 64 orders, so 64 unmatched stops already fill
/// one channel's worth of debt; a 65th uses a whole-helper cut with its counts
/// instead of losing an order. This is not a deadline or a capture limit.
pub(in crate::daemon) const MAX_RETIRED_STOPS: usize = 64;

#[derive(Clone, Copy)]
pub(in crate::daemon) enum CallStopVoice {
    KeepTranscription,
    Cancel(VoiceCancellationCause),
}

pub(in crate::daemon) struct RetiredStreamStop {
    pub(in crate::daemon) stream: String,
    pub(in crate::daemon) sent: bool,
    /// A superseded call stop still has a receipt to read, but only the new
    /// selective stop may finish the late-started stream.
    pub(in crate::daemon) may_finish_call: bool,
}

impl Runtime {
    /// A late start can open a stream after its capture was drained. This
    /// stop owns only that stream and its receipt; it must not pause the source
    /// buffer or the microphone of the capture that followed it.
    pub(in crate::daemon) async fn request_retired_stream_stop(
        &mut self,
        stream: &str,
    ) -> Result<()> {
        if self.audio.retired_stops.len() >= MAX_RETIRED_STOPS {
            self.retired_stop_capacity_exhausted().await?;
            return Ok(());
        }
        let command = native::command("audio_stop", json!({"stream_id":stream}))?;
        let command_id = NativeCommandHeader::from_value(&command)?.id.to_owned();
        self.audio.retired_stops.insert(
            command_id.clone(),
            RetiredStreamStop {
                stream: stream.to_owned(),
                sent: false,
                may_finish_call: true,
            },
        );
        self.send_retired_stream_stop(&command_id).await;
        Ok(())
    }

    pub(in crate::daemon) async fn retired_stop_capacity_exhausted(&mut self) -> Result<()> {
        let pending = self.audio.retired_stops.len();
        self.log(
            "retired_stop_capacity_exhausted",
            &[
                ("pending", &pending.to_string()),
                ("max", &MAX_RETIRED_STOPS.to_string()),
            ],
        );
        let cause = format!(
            "retired_stop_capacity_exhausted: {pending} selective stops pending; maximum {MAX_RETIRED_STOPS}. A full stop is requested from the Mac."
        );
        self.contain_coded(
            orbit_core::privacy_stop::StopCause::HelperIntegrityFailure,
            "retired_stop_capacity_exhausted",
            &cause,
        )
        .await;
        if !self.state.privacy_stopped() && self.audio.lifecycle.full_stop.is_none() {
            self.request_full_audio_stop(FullStop::Cut).await?;
        }
        Ok(())
    }

    async fn send_retired_stream_stop(&mut self, command_id: &str) {
        let Some(stream) = self
            .audio
            .retired_stops
            .get(command_id)
            .map(|stop| stop.stream.clone())
        else {
            return;
        };
        let command =
            match native::command("audio_stop", json!({"id":command_id,"stream_id":stream})) {
                Ok(command) => command,
                Err(error) => {
                    self.log(
                        "retired_audio_stop_invalid",
                        &[("id", command_id), ("error", &error.to_string())],
                    );
                    return;
                }
            };
        self.helper.record_pending(&command);
        // The established 500 ms helper-channel budget protects the actor
        // from backpressure. It does not prove the retired stream closed: only
        // its matching drain receipt does, and an unsent stop stays owed.
        match self
            .helper
            .send_within(command, Duration::from_millis(500))
            .await
        {
            Ok(Ok(())) => {
                if let Some(stop) = self.audio.retired_stops.get_mut(command_id) {
                    stop.sent = true;
                }
                self.log(
                    "retired_audio_stop_sent",
                    &[("id", command_id), ("stream_id", &stream)],
                );
            }
            Ok(Err(_closed)) => {
                self.audio.retired_stops.remove(command_id);
                self.log("retired_audio_stop_helper_gone", &[("id", command_id)]);
            }
            Err(_elapsed) => {
                self.log("retired_audio_stop_send_owed", &[("id", command_id)]);
            }
        }
    }

    pub(in crate::daemon) async fn retry_retired_stream_stops(&mut self) {
        if !self.state.capabilities().native_helper() {
            let retired_call_starts = self.audio.retired_call_starts.len();
            let retired_stops = self.audio.retired_stops.len();
            if retired_call_starts > 0 || retired_stops > 0 {
                self.log(
                    "retired_capture_debts_helper_unavailable",
                    &[
                        ("retired_call_starts", &retired_call_starts.to_string()),
                        ("retired_stops", &retired_stops.to_string()),
                    ],
                );
                self.audio.retired_call_starts.clear();
                self.audio.retired_stops.clear();
            }
            return;
        }
        let owed = self
            .audio
            .retired_stops
            .iter()
            .find(|(_, stop)| !stop.sent)
            .map(|(id, _)| id.clone());
        if let Some(id) = owed {
            self.send_retired_stream_stop(&id).await;
        }
    }

    /// An audio_stop that preceded a call's audio_start may have returned
    /// stream_not_found. Its matching late start needs a fresh selective stop;
    /// the old pending receipt cannot stand for that later stream.
    pub(in crate::daemon) async fn retire_pending_stream_stop(
        &mut self,
        stream: &str,
        start_failed: bool,
    ) -> Result<bool> {
        let ids: Vec<String> = self
            .audio
            .lifecycle
            .stops
            .iter()
            .filter(|(_, stop)| stop.stream == stream)
            .map(|(id, _)| id.clone())
            .collect();
        for id in ids {
            let Some(stop) = self.audio.lifecycle.stops.remove(&id) else {
                continue;
            };
            if !stop.answered {
                if self.audio.retired_stops.len() >= MAX_RETIRED_STOPS {
                    self.retired_stop_capacity_exhausted().await?;
                    return Ok(false);
                }
                self.audio.retired_stops.insert(
                    id.clone(),
                    RetiredStreamStop {
                        stream: stream.to_owned(),
                        sent: true,
                        may_finish_call: false,
                    },
                );
            }
            self.log(
                if start_failed {
                    "call_stop_retired_after_failed_start"
                } else {
                    "call_stop_overtaken_by_late_start"
                },
                &[("id", &id), ("stream_id", stream)],
            );
        }
        Ok(true)
    }

    pub(in crate::daemon) fn begin_call_voice(&mut self, preroll: &[(Value, i64)]) -> Result<()> {
        self.capture.begin_voice()?;
        self.audio.lifecycle.voice.submit = true;
        for (frame, time) in preroll {
            self.capture.ingest_voice_frame(frame, *time)?;
        }
        Ok(())
    }

    pub(in crate::daemon) async fn finish_call_voice(&mut self, submit: bool) -> Result<()> {
        if submit {
            self.dispatch_voice_transcription().await
        } else {
            // A subminimum candidate is not a message, but it may own the
            // beginning of an earlier unheard turn. Put that whole prefix
            // back rather than leaving `joined` to be overwritten at onset.
            self.prepend_joined_call_turn();
            if let Some(voice) = self.capture.take_voice() {
                self.carry_into_next_turn(super::super::call::call_turn_audio(
                    super::super::id(),
                    voice,
                ));
            }
            self.audio.take_capture_over_answer();
            self.resume_call_reading().await
        }
    }

    /// A call whose drain reached its fifteen seconds while the helper still
    /// speaks, with no meeting draining beside it: its stop is abandoned and
    /// the audio received is kept.
    pub(super) fn live_call_reached_drain_limit(&self) -> bool {
        let now = Instant::now();
        let hard_limit_reached = self
            .audio
            .lifecycle
            .started_at
            .and_then(|started| started.checked_add(AUDIO_DRAIN_LIMIT))
            .is_some_and(|limit| now >= limit);
        let helper_is_live = self
            .audio
            .lifecycle
            .last_liveness
            .and_then(|seen| seen.checked_add(AUDIO_SILENCE_TIMEOUT))
            .is_some_and(|silent_at| now < silent_at);
        let meeting_is_draining =
            self.state
                .durable
                .capture
                .recording
                .as_ref()
                .is_some_and(|recording| {
                    matches!(
                        recording.phase,
                        CapturePhase::Starting
                            | CapturePhase::Recording
                            | CapturePhase::Paused
                            | CapturePhase::Finalizing
                    )
                });
        hard_limit_reached && helper_is_live && self.audio.call.is_some() && !meeting_is_draining
    }
}

impl crate::daemon::audio::AudioHandle {
    pub(in crate::daemon) fn mark_overtaken_stop_answered(&mut self, command_id: &str) {
        if let Some(stop) = self.lifecycle.stops.get_mut(command_id) {
            stop.answered = true;
        }
    }

    pub(in crate::daemon) fn call_stop_overtook_unanswered_start(&self, command_id: &str) -> bool {
        self.lifecycle.stops.get(command_id).is_some_and(|stop| {
            stop.source == "mic_local"
                && self.closing_call_owns_stream(&stop.stream)
                && self
                    .retired_call_starts
                    .values()
                    .any(|stream| stream == &stop.stream)
        })
    }
}
