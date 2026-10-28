//! The helper's answers to the audio's retired orders: the stop of a stream
//! a call or a cut left behind, and a start that arrived after its stream
//! was retired.
use super::super::*;
use orbit_core::notice::{Notice, NoticeCode, NoticeKey};
use orbit_core::wire::native::{NativeResponseFields, decode_result_body};
use serde::Deserialize;

#[derive(Deserialize)]
pub(super) struct RetiredAudioStartResult {
    pub(super) stream_id: String,
}

#[derive(Deserialize)]
pub(super) struct RetiredAudioStopResult {
    pub(super) stream_id: String,
    pub(super) drain_complete: bool,
}

/// The Mac accepts at most 128 UTF-8 bytes in audio_start.stream_id (the
/// current generated UUID is 36). Bound only a late readback before copying
/// it into audio_stop; this is not a capture duration or response-size limit.
pub(super) const MAX_RETIRED_START_STREAM_ID_BYTES: usize = 128;

/// What the helper said to a retired stream's stop, as a code for the log.
fn retired_stop_outcome<'a>(
    response: &'a NativeResponseFields,
    stop: &audio::RetiredStreamStop,
) -> &'a str {
    if response.ok {
        match response.result.as_ref() {
            None => "retired_audio_stop_result_absent",
            Some(result) => {
                match decode_result_body::<RetiredAudioStopResult>("audio_stop", result) {
                    Err(_) => "retired_audio_stop_result_malformed",
                    Ok(receipt) if !receipt.drain_complete => "retired_audio_stop_drain_incomplete",
                    Ok(receipt) if receipt.stream_id != stop.stream => {
                        "retired_audio_stop_stream_mismatch"
                    }
                    Ok(_) => "confirmed",
                }
            }
        }
    } else {
        response
            .error
            .as_ref()
            .map_or("retired_audio_stop_error_absent", |error| {
                error.code.as_str()
            })
    }
}

impl Runtime {
    /// A retired stream's stop was answered: the log says how, and a stop
    /// that may finish the closing call finishes it once the stream is gone.
    pub(super) async fn retired_stop_answered(
        &mut self,
        response: &NativeResponseFields,
        stop: audio::RetiredStreamStop,
    ) -> Result<()> {
        let outcome = retired_stop_outcome(response, &stop);
        if outcome == "confirmed" {
            self.log(
                "retired_audio_stop_confirmed",
                &[("id", &response.id), ("stream_id", &stop.stream)],
            );
        } else if outcome == "stream_not_found" {
            self.log(
                "retired_audio_stop_stream_absent",
                &[("id", &response.id), ("stream_id", &stop.stream)],
            );
        } else if self.state.privacy_stopped()
            && matches!(outcome, "privacy_stopped" | "operation_cancelled_by_stop")
        {
            self.log(
                "retired_audio_stop_cancelled_by_privacy_stop",
                &[
                    ("id", &response.id),
                    ("stream_id", &stop.stream),
                    ("code", outcome),
                ],
            );
        } else {
            self.log(
                "retired_audio_stop_unconfirmed",
                &[
                    ("id", &response.id),
                    ("stream_id", &stop.stream),
                    ("code", outcome),
                ],
            );
            if stop.may_finish_call {
                self.say(
                    Notice::new(NoticeCode::RetiredStopUnconfirmed)
                        .with_token(NoticeKey::Code, outcome),
                );
            }
        }
        if stop.may_finish_call
            && matches!(outcome, "confirmed" | "stream_not_found")
            && self.audio.closing_call_owns_stream(&stop.stream)
        {
            self.finish_call_microphone().await?;
        }
        Ok(())
    }

    /// The closing call's microphone is gone: what it held is sealed and
    /// the drain completes.
    async fn finish_call_microphone(&mut self) -> Result<()> {
        self.audio.mic_stream = None;
        let segments = self.capture.buffer.pause_source("mic_local")?;
        self.seal(segments)?;
        self.complete_audio_drain().await
    }

    /// A call's stop found no stream because it overtook a start the helper
    /// had not answered: that stop is answered, and nothing else happens.
    pub(super) fn overtaken_call_stop_answered(&mut self, response: &NativeResponseFields) -> bool {
        if !response.ok
            && response
                .error
                .as_ref()
                .is_some_and(|error| error.code == "stream_not_found")
            && self.audio.call_stop_overtook_unanswered_start(&response.id)
        {
            self.log(
                "call_stop_overtook_unanswered_start",
                &[("id", &response.id), ("code", "stream_not_found")],
            );
            self.audio.mark_overtaken_stop_answered(&response.id);
            return true;
        }
        false
    }

    /// A retired start was answered: a refusal proves its stream never
    /// opened, a confirmation names a stream that has to be stopped.
    pub(super) async fn retired_start_answered(
        &mut self,
        response: &NativeResponseFields,
    ) -> Result<()> {
        let retired_call_stream = self.audio.retired_call_starts.remove(&response.id);
        let code = response.error.as_ref().map(|error| error.code.as_str());
        self.log(
            "native_audio_start_after_retirement",
            &[
                ("id", &response.id),
                ("cause", "arranque ya retirado"),
                ("code", code.unwrap_or("confirmed")),
            ],
        );
        if let Some(code) = code {
            self.retired_start_refused(response, code, retired_call_stream.as_deref())
                .await
        } else if let Some(result) = response.result.as_ref() {
            self.retired_start_confirmed(response, result, retired_call_stream.as_deref())
                .await
        } else {
            if let Some(expected) = retired_call_stream.as_deref() {
                self.log("retired_call_start_result_absent", &[("id", &response.id)]);
                self.stop_retired_stream_or_log(&response.id, expected)
                    .await;
            }
            Ok(())
        }
    }

    async fn retired_start_refused(
        &mut self,
        response: &NativeResponseFields,
        code: &str,
        retired_call_stream: Option<&str>,
    ) -> Result<()> {
        if let Some(stream) = retired_call_stream
            && self
                .audio
                .closing_call_start_owns_stream(&response.id, stream)
        {
            // The stop found no stream because it overtook the start;
            // this failed start proves the expected stream never opened.
            self.retire_pending_stream_stop(stream, true).await?;
            if !self.state.privacy_stopped() && self.audio.lifecycle.full_stop.is_none() {
                self.replace_drain_wait(
                    Notice::new(NoticeCode::AudioStartRefused).with_token(NoticeKey::Code, code),
                );
            }
            self.finish_call_microphone().await?;
            if self.audio.call.is_none() {
                self.say(NoticeCode::CallClosed);
            }
        }
        if self.audio.lifecycle.full_stop.is_some()
            && !self.state.privacy_stopped()
            && !matches!(code, "operation_cancelled_by_stop" | "privacy_stopped")
        {
            self.say(Notice::new(NoticeCode::OtherTrackStopped).with_token(NoticeKey::Code, code));
            self.store_actor.snapshot(&self.state).await?;
        }
        Ok(())
    }

    async fn retired_start_confirmed(
        &mut self,
        response: &NativeResponseFields,
        result: &Value,
        retired_call_stream: Option<&str>,
    ) -> Result<()> {
        let start = match decode_result_body::<RetiredAudioStartResult>("audio_start", result) {
            Ok(start) => start,
            Err(error) => {
                self.log(
                    "retired_audio_start_invalid_result",
                    &[("id", &response.id), ("error", &error.to_string())],
                );
                if let Some(expected) = retired_call_stream {
                    self.stop_retired_stream_or_log(&response.id, expected)
                        .await;
                }
                return Ok(());
            }
        };
        if start.stream_id.is_empty() {
            self.log("retired_audio_start_empty_stream", &[("id", &response.id)]);
        } else if start.stream_id.len() > MAX_RETIRED_START_STREAM_ID_BYTES {
            self.log(
                "retired_audio_start_stream_too_long",
                &[
                    ("id", &response.id),
                    ("bytes", &start.stream_id.len().to_string()),
                    ("max_bytes", &MAX_RETIRED_START_STREAM_ID_BYTES.to_string()),
                ],
            );
        } else if let Some(expected) =
            retired_call_stream.filter(|expected| *expected != start.stream_id)
        {
            self.retired_call_stream_mismatch(response, expected, &start.stream_id)
                .await?;
        } else if (self.audio.mic_stream.as_deref() == Some(&start.stream_id)
            && !self
                .audio
                .closing_call_start_owns_stream(&response.id, &start.stream_id))
            || self.audio.remote_stream.as_deref() == Some(&start.stream_id)
        {
            self.log("retired_audio_start_live_stream", &[("id", &response.id)]);
        } else {
            if self
                .audio
                .closing_call_start_owns_stream(&response.id, &start.stream_id)
                && !self
                    .retire_pending_stream_stop(&start.stream_id, false)
                    .await?
            {
                return Ok(());
            }
            self.stop_retired_stream_or_log(&response.id, &start.stream_id)
                .await;
        }
        Ok(())
    }

    /// The Mac confirmed a stream other than the retired call's: the
    /// capture is cut so no uncertain stream stays open.
    async fn retired_call_stream_mismatch(
        &mut self,
        response: &NativeResponseFields,
        expected: &str,
        observed: &str,
    ) -> Result<()> {
        self.log(
            "retired_call_start_stream_mismatch",
            &[
                ("id", &response.id),
                ("expected", expected),
                ("observed", observed),
            ],
        );
        self.contain(
            orbit_core::privacy_stop::StopCause::HelperIntegrityFailure,
            "The Mac confirmed a different stream from the retired call start; capture is cut to close any uncertain stream.",
        )
        .await;
        if !self.state.privacy_stopped() && self.audio.lifecycle.full_stop.is_none() {
            self.request_full_audio_stop(audio::FullStop::Cut).await?;
        }
        Ok(())
    }

    /// Asks the helper to stop a retired stream; a failed request is logged.
    async fn stop_retired_stream_or_log(&mut self, id: &str, stream: &str) {
        if let Err(error) = self.request_retired_stream_stop(stream).await {
            self.log(
                "retired_audio_stop_request_failed",
                &[("id", id), ("error", &error.to_string())],
            );
        }
    }
}

impl crate::daemon::audio::AudioHandle {
    /// An `audio_start` answered after its start was retired, or one this
    /// capture never asked for.
    pub(super) fn retired_start(
        &self,
        command_name: &str,
        response: &NativeResponseFields,
    ) -> bool {
        command_name == "audio_start"
            && (self.retired_call_starts.contains_key(&response.id)
                || !self.pending_starts.iter().any(|id| id == &response.id))
    }
}
