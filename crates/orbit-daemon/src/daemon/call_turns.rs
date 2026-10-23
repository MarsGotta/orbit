//! A call's turns between the detector and the transcriber: what the user
//! said before carries on in what they say next, and a turn spoken over
//! Deimos's answer stops him only once its words are known.
use super::super::Runtime;
use super::super::audio::VoiceCancellationCause;
use anyhow::Result;
use orbit_core::{Event, voice::TurnDetector};
use serde_json::Value;

/// A call turn spoken while Deimos answered, held in memory while its words
/// are asked for: nothing is stored, shown or interrupted until they come.
pub(in crate::daemon) struct OverAnswerTurn {
    pub(in crate::daemon) operation_id: String,
    pub(in crate::daemon) duration_ms: u64,
    pub(in crate::daemon) voice: crate::capture::VoiceBuffer,
}

/// A call turn's voice, kept while its transcription is in course so that
/// what the user says next, before it reaches Deimos, can carry it.
pub(in crate::daemon) struct CallTurnAudio {
    pub(in crate::daemon) operation_id: String,
    sample_rate: u32,
    channels: u16,
    started_at_ms: i64,
    monotonic_ns: u64,
    pub(in crate::daemon) last_sequence: Option<u64>,
    pcm: Vec<u8>,
}

/// A finished turn's voice, as a later turn carries it.
pub(in crate::daemon) fn call_turn_audio(
    operation_id: String,
    voice: crate::capture::VoiceBuffer,
) -> CallTurnAudio {
    CallTurnAudio {
        operation_id,
        sample_rate: voice.sample_rate,
        channels: voice.channels,
        started_at_ms: voice.started_at_ms,
        monotonic_ns: voice.monotonic_ns,
        last_sequence: voice.last_sequence(),
        pcm: voice.pcm,
    }
}

impl Runtime {
    /// A turn the detector just began. Over Deimos's answer it stops nothing
    /// and is not counted yet; otherwise it retires the turn in course, as it
    /// always did. What the user said just before, still being transcribed,
    /// or a turn in which nothing was heard, goes first in it.
    pub(in crate::daemon) async fn begin_call_turn(
        &mut self,
        interrupt: bool,
        responding: bool,
        began: Option<TurnDetector>,
        preroll: Vec<(Value, i64)>,
    ) -> Result<()> {
        // Over Deimos's answer (he speaks, or works on one) a turn
        // stops nothing yet: it is heard first (`hear_over_answer`).
        let over_answer =
            interrupt && (responding || self.state.durable.assistant.active_request.is_some());
        let joined = if interrupt {
            self.joinable_turn()
        } else {
            None
        };
        if interrupt && (joined.is_some() || !over_answer) {
            self.cancel_voice_transcription(if joined.is_some() {
                VoiceCancellationCause::CallTurnJoined
            } else {
                VoiceCancellationCause::CallTurnReplaced
            })
            .await?;
            // The joined turn goes on in this one: its own note,
            // whose audio this turn now carries, leaves the
            // conversation instead of staying as «Va junto…».
            let open_turn = self.state.durable.assistant.turn.as_ref();
            if let Some(joined) = joined.as_ref()
                && open_turn.is_some_and(|turn| turn.user.id == joined.operation_id)
            {
                self.drop_unheard_voice_note(&joined.operation_id, "joined_next_turn")
                    .await?;
            }
        }
        if interrupt && !over_answer {
            self.capture.buffer.voice = None;
            self.apply(Event::Interrupt).await?;
            // The interrupt retires the turn in course, as Esc does,
            // and that reset the detector this frame just opened a
            // turn in: the turn never ended and the next one found
            // its buffer taken.
            if let (Some(call), Some(began)) = (self.audio.call.as_mut(), began) {
                call.detector = began;
            }
        }
        // A turn in which nothing was heard goes on in this one.
        let joined = joined.or_else(|| {
            self.audio
                .call
                .as_mut()
                .and_then(|call| call.carried.take())
        });
        // Frames already in the carried turn are not heard twice.
        let carried = joined.as_ref().and_then(|turn| turn.last_sequence);
        let preroll: Vec<_> = preroll
            .into_iter()
            .filter(|(frame, _)| {
                carried.is_none_or(|last| {
                    frame["sequence"]
                        .as_u64()
                        .is_some_and(|sequence| sequence > last)
                })
            })
            .collect();
        self.begin_call_voice(&preroll)?;
        if let Some(call) = self.audio.call.as_mut() {
            call.joined = joined;
            call.capturing_over_answer = over_answer;
        }
        // Over Deimos's answer it is not a turn until words are
        // heard in it (`accept_over_answer`).
        if !over_answer {
            self.apply(Event::CallTurnStarted).await?;
        }
        Ok(())
    }

    /// The turn whose transcription is still in course, when the user speaks
    /// again before it reached Deimos: their new words continue it, so its
    /// voice goes first in the new turn instead of being cancelled and lost.
    /// A turn already sent to Deimos is not carried: he has it.
    fn joinable_turn(&mut self) -> Option<CallTurnAudio> {
        self.assistant.voice_task.active()?;
        let operation = self.assistant.voice_operation.clone()?;
        let answered = self.state.durable.assistant.active_request.is_some();
        let call = self.audio.call.as_mut()?;
        // A turn spoken over Deimos's answer never reached him: whatever he
        // is doing, the user's next words carry it.
        if let Some(turn) = call
            .over_answer
            .take_if(|turn| turn.operation_id == operation)
        {
            return Some(call_turn_audio(turn.operation_id, turn.voice));
        }
        if answered {
            return None;
        }
        call.submitted
            .take()
            .filter(|turn| turn.operation_id == operation)
    }

    /// Nothing was heard in a call turn: its audio waits for what the user
    /// says next, which carries it (`carried`); the person may not have
    /// finished. While they already speak, it goes first in that turn.
    pub(in crate::daemon) fn carry_into_next_turn(&mut self, audio: CallTurnAudio) {
        let capturing = self.capture.buffer.voice.is_some();
        if let Some(call) = self
            .audio
            .call
            .as_mut()
            .filter(|call| call.closing_at_ms.is_none())
        {
            if capturing && call.joined.is_none() {
                call.joined = Some(audio);
            } else if !capturing {
                call.carried = Some(audio);
            }
        }
    }

    /// The words of a turn spoken over Deimos's answer are known: he stops,
    /// as a barge-in always stopped him, now that it is speech. The turn the
    /// user may be saying meanwhile survives the interrupt, which retires
    /// the call's turn in course.
    pub(in crate::daemon) async fn interrupt_for_accepted_turn(&mut self) -> Result<()> {
        let answering = self.state.durable.assistant.active_request.is_some()
            || self.assistant.speech_task.active().is_some()
            || self
                .audio
                .call
                .as_ref()
                .is_some_and(|call| call.playback_active);
        if !answering {
            return Ok(());
        }
        let kept = self.audio.call.as_mut().map(|call| {
            (
                call.detector.clone(),
                std::mem::take(&mut call.preroll),
                call.preroll_ns,
                call.joined.take(),
                call.carried.take(),
                call.capturing_over_answer,
            )
        });
        let voice = self.capture.buffer.voice.take();
        let submit = self.audio.lifecycle.voice.submit;
        self.apply(Event::Interrupt).await?;
        if let (Some(call), Some((detector, preroll, preroll_ns, joined, carried, over))) =
            (self.audio.call.as_mut(), kept)
        {
            call.detector = detector;
            call.preroll = preroll;
            call.preroll_ns = preroll_ns;
            call.joined = joined;
            call.carried = carried;
            call.capturing_over_answer = over;
        }
        self.capture.buffer.voice = voice;
        self.audio.lifecycle.voice.submit = submit;
        Ok(())
    }

    /// Puts the carried turn's voice before the turn about to be sent, when
    /// both are the same format and the whole still fits one voice turn's
    /// bounds (60 s, and the encrypted chunk). Otherwise the carried voice is
    /// dropped with its numbers in the log.
    pub(in crate::daemon) fn prepend_joined_call_turn(&mut self) {
        let Some(joined) = self.audio.call.as_mut().and_then(|call| call.joined.take()) else {
            return;
        };
        let Some(voice) = self.capture.buffer.voice.as_mut() else {
            return;
        };
        let bytes = joined.pcm.len().saturating_add(voice.pcm.len());
        let max_bytes = (voice.sample_rate as usize)
            .saturating_mul(2 * 60)
            .min(orbit_store::MAX_CHUNK_BYTES - 44);
        if joined.sample_rate != voice.sample_rate
            || joined.channels != voice.channels
            || bytes > max_bytes
        {
            let (rate, channels) = (voice.sample_rate, voice.channels);
            self.log(
                "call_turn_join_refused",
                &[
                    ("bytes", &bytes.to_string()),
                    ("max_bytes", &max_bytes.to_string()),
                    ("sample_rate", &format!("{}/{rate}", joined.sample_rate)),
                    ("channels", &format!("{}/{channels}", joined.channels)),
                ],
            );
            return;
        }
        let mut pcm = joined.pcm;
        pcm.append(&mut voice.pcm);
        voice.pcm = pcm;
        voice.started_at_ms = joined.started_at_ms;
        voice.monotonic_ns = joined.monotonic_ns;
    }
}

impl crate::daemon::audio::AudioHandle {
    /// Takes whether the turn just finished began over Deimos's answer.
    pub(in crate::daemon) fn take_capture_over_answer(&mut self) -> bool {
        self.call
            .as_mut()
            .is_some_and(|call| std::mem::take(&mut call.capturing_over_answer))
    }

    /// Holds a turn spoken over Deimos's answer while its words are asked for.
    pub(in crate::daemon) fn keep_over_answer_turn(
        &mut self,
        operation_id: &str,
        duration_ms: u64,
        voice: crate::capture::VoiceBuffer,
    ) {
        if let Some(call) = self.call.as_mut() {
            call.over_answer = Some(OverAnswerTurn {
                operation_id: operation_id.to_owned(),
                duration_ms,
                voice,
            });
        }
    }

    /// The turn spoken over Deimos's answer whose words were `operation_id`.
    pub(in crate::daemon) fn take_over_answer_turn(
        &mut self,
        operation_id: &str,
    ) -> Option<OverAnswerTurn> {
        self.call
            .as_mut()?
            .over_answer
            .take_if(|turn| turn.operation_id == operation_id)
    }

    /// The call turn `operation_id` sent to transcription, while kept.
    pub(in crate::daemon) fn take_submitted_call_turn(
        &mut self,
        operation_id: &str,
    ) -> Option<CallTurnAudio> {
        self.call
            .as_mut()?
            .submitted
            .take_if(|turn| turn.operation_id == operation_id)
    }

    /// Keeps a call turn's voice while it is transcribed (see `joinable_turn`).
    pub(in crate::daemon) fn keep_submitted_call_turn(
        &mut self,
        operation_id: &str,
        voice: crate::capture::VoiceBuffer,
    ) {
        if let Some(call) = self
            .call
            .as_mut()
            .filter(|call| call.closing_at_ms.is_none())
        {
            call.submitted = Some(call_turn_audio(operation_id.to_owned(), voice));
        }
    }
}
