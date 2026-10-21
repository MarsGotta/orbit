//! One explicitly authorized native producer feeds both the continuous original
//! recorder and bounded derived voice turns. Native processing is observed, not
//! inferred from PCM energy and not certified as effective AEC by these checks.
use super::audio::{CallStopVoice, VoiceCancellationCause};
use super::{Runtime, id, now};
use anyhow::{Context, Result, bail};
use orbit_core::notice::{Notice, NoticeCode, NoticeKey};
use orbit_core::wire::native::{NativeCommandHeader, decode_result_body};
use orbit_core::{
    CallPhase, CallState, Event,
    voice::{StopReason, TurnAction, TurnDetector, VoiceObservation, VoicePolicy, energy_exceeds},
};
use orbit_store::{Job, JobState};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};

const MICROPHONE_SOURCE: &str = "mic_local";

/// How long a call goes without any microphone frame before the daemon stops
/// it as `no_audio`.
///
/// What it protects: a call whose microphone stopped delivering (the tap
/// gone, the device changed, the helper alive but sending nothing) must not
/// stay open looking as if it listened. Why 3 s: it is the window in which
/// the helper declares the same loss on its side (`no_capture_frames` in
/// `AudioCapture.swift`); the two are independent and need not agree. What it
/// is not: Mars's silence, which is still audio and has its own minute
/// (`VoicePolicy::idle_timeout_ms`), and not a budget for the daemon's loop:
/// frames waiting in the loop's queue are audio arriving, so a loop that was
/// held reads them before this is judged (SIM-55).
const CALL_AUDIO_SILENCE_LIMIT: Duration = Duration::from_secs(3);

/// Keeps unanswered call-start identities after hangup so a late success can
/// still be stopped after the diagnostic command index evicts it. 64 matches
/// one native command channel's capacity, and the 64/65 synthetic test checks
/// admission at that boundary. It is not a time limit or permission gate.
const MAX_RETIRED_CALL_STARTS: usize = 64;

pub(super) struct CallRuntime {
    authorization: CallState,
    pub(in crate::daemon) detector: TurnDetector,
    start_request: String,
    pipeline_id: Option<String>,
    preroll: VecDeque<(Value, i64, u64)>,
    preroll_ns: u64,
    started: Instant,
    pub(in crate::daemon) last_audio: Instant,
    pub(in crate::daemon) closing_at_ms: Option<i64>,
    gaps: u64,
    /// What the call heard, said when it stops (2026-10-17: a call that
    /// opened heard no turn in a minute, and nothing said whether audio came
    /// or how loud). Frames that passed the processing evidence, and the
    /// loudest frame's RMS against the speech threshold.
    pub(in crate::daemon) frames_heard: u64,
    loudest_rms: u32,
    /// Frames over the speech threshold, and the longest run of them: a turn
    /// opens after `onset_ms` of voice in a row.
    voiced_frames: u64,
    voiced_run_ns: u64,
    longest_voiced_run_ns: u64,
    /// Whether the last frame said Deimos's voice was playing.
    playback_active: bool,
    /// The last turn sent to transcription, and the one the turn being
    /// captured now carries (SIM-57).
    submitted: Option<CallTurnAudio>,
    joined: Option<CallTurnAudio>,
    /// What the main loop knew when the current tick began (SIM-55): the
    /// helper events it had not read yet, and how long the tick before took.
    queued_native_events: usize,
    last_tick: Duration,
}

/// A call turn's voice, kept while its transcription is in course so that
/// what Mars says next, before it reaches Deimos, can carry it (SIM-57).
pub(super) struct CallTurnAudio {
    operation_id: String,
    sample_rate: u32,
    channels: u16,
    started_at_ms: i64,
    monotonic_ns: u64,
    last_sequence: Option<u64>,
    pcm: Vec<u8>,
}

#[derive(Deserialize)]
struct CallStartReceipt {
    stream_id: String,
    processing: String,
    processing_enabled: bool,
    processing_bypassed: bool,
    agc_enabled: bool,
    playback_reference: String,
    pipeline_id: String,
}

#[derive(Deserialize)]
struct VoiceCapabilities {
    #[serde(default)]
    supported_audio_processing: Option<Vec<String>>,
    #[serde(default)]
    voice_processing_available: Option<bool>,
    #[serde(default)]
    playback_reference_available: Option<bool>,
}

#[derive(Deserialize)]
struct CallFrameFacts {
    frames: u32,
    sample_rate: u32,
    monotonic_ns: u64,
    playback_active: bool,
}

#[derive(Deserialize)]
struct ProcessingEvidence<'a> {
    processing: &'a str,
    processing_enabled: bool,
    processing_bypassed: bool,
    agc_enabled: bool,
    playback_reference: &'a str,
    pipeline_id: &'a str,
}

#[cfg(test)]
#[path = "call_overflow_tests.rs"]
mod overflow_tests;

impl Runtime {
    pub(super) fn note_call_gap(&mut self, recording_id: &str) {
        if let Some(call) = self
            .capture
            .call_runtime
            .as_mut()
            .filter(|call| call.authorization.id == recording_id)
        {
            call.gaps = call.gaps.saturating_add(1);
        }
    }

    pub(super) async fn start_call_audio(&mut self) -> Result<()> {
        if self.capture.retired_call_starts.len() >= MAX_RETIRED_CALL_STARTS {
            let pending = self.capture.retired_call_starts.len();
            let cause = format!(
                "retired_call_start_capacity_exhausted: {pending} arranques de llamada sin respuesta; máximo {MAX_RETIRED_CALL_STARTS}."
            );
            self.contain(
                orbit_core::privacy_stop::StopCause::HelperIntegrityFailure,
                &cause,
            )
            .await;
            anyhow::bail!(cause);
        }
        let authorization = self
            .state
            .durable
            .call
            .clone()
            .context("Llamada sin autorización local")?;
        if self.capture.call_runtime.is_some()
            || self.capture.mic_stream.is_some()
            || self.capture.remote_stream.is_some()
            || self.capture.buffer.voice.is_some()
            || self.conversation.voice_task.active().is_some()
        {
            bail!(
                "La ruta de audio anterior aún está activa; finalízala antes de iniciar una llamada"
            );
        }
        let epoch = id();
        self.capture
            .buffer
            .begin_recording_with_epoch(authorization.id.clone(), epoch.clone())?;
        if self.capture.pending_seals.is_empty() && !self.capture.buffer.has_pending_audio() {
            self.capture.reserve_exhausted = false;
        }
        let epoch_job = Job {
            id: format!("capture-epoch-{epoch}"),
            kind: orbit_store::QueueKind::CaptureEpoch.into(),
            state: JobState::Succeeded,
            created_at: now(),
            payload: json!({"recording_id":authorization.id,
                "capture_epoch_id":epoch,"expected_tracks":[MICROPHONE_SOURCE],"clock_domain":"mach_absolute_time",
                "origin":"explicit_call_start"}),
        };
        let manifest_job = Job {
            id: format!("recording-{}", authorization.id),
            kind: orbit_store::QueueKind::RecordingManifest.into(),
            state: JobState::Succeeded,
            created_at: now(),
            payload: json!({"recording_id":authorization.id,"recording_kind":"voice_call",
                "session_id":self.state.session_id(),"started_at_ms":authorization.started_at_ms,
                "expires_at_ms":authorization.expires_at_ms,"processing_requested":"voice_processing",
                "original_includes_silence":true,"aec_hardware_validated":false}),
        };
        self.store_actor
            .run(move |store| {
                store.enqueue_job(&epoch_job)?;
                store.enqueue_job(&manifest_job)?;
                Ok(())
            })
            .await?;
        let stream = id();
        let command = crate::native::command(
            "audio_start",
            json!({"source":MICROPHONE_SOURCE,
            "stream_id":stream,"processing":"voice_processing"}),
        )?;
        let request = NativeCommandHeader::from_value(&command)?.id.to_owned();
        self.capture.call_runtime = Some(CallRuntime {
            detector: TurnDetector::authorized(
                self.state.generation(),
                authorization.started_at_ms,
                authorization.expires_at_ms,
            ),
            authorization,
            start_request: request.clone(),
            pipeline_id: None,
            preroll: VecDeque::new(),
            preroll_ns: 0,
            started: Instant::now(),
            last_audio: Instant::now(),
            closing_at_ms: None,
            gaps: 0,
            frames_heard: 0,
            loudest_rms: 0,
            voiced_frames: 0,
            voiced_run_ns: 0,
            longest_voiced_run_ns: 0,
            playback_active: false,
            submitted: None,
            joined: None,
            queued_native_events: 0,
            last_tick: Duration::ZERO,
        });
        self.capture.pending_starts.push(request);
        self.capture.mic_stream = Some(stream);
        self.record_pending_native(&command);
        tokio::time::timeout(Duration::from_millis(500), self.native.send(command)).await??;
        Ok(())
    }

    pub(super) fn mark_call_stopping(&mut self) {
        if let Some(call) = self.capture.call_runtime.as_mut() {
            call.closing_at_ms.get_or_insert_with(now);
            call.preroll.clear();
            call.preroll_ns = 0;
            if self
                .capture
                .pending_starts
                .iter()
                .any(|id| id == &call.start_request)
                && let Some(stream) = self.capture.mic_stream.as_ref()
            {
                self.capture
                    .retired_call_starts
                    .insert(call.start_request.clone(), stream.clone());
            }
            self.capture
                .pending_starts
                .retain(|id| id != &call.start_request);
        }
    }

    pub(super) fn closing_call_owns_stream(&self, stream: &str) -> bool {
        self.capture.call_runtime.as_ref().is_some_and(|call| {
            call.closing_at_ms.is_some() && self.capture.mic_stream.as_deref() == Some(stream)
        })
    }

    pub(super) fn closing_call_start_owns_stream(&self, command_id: &str, stream: &str) -> bool {
        self.call_start_request_pending(command_id) && self.closing_call_owns_stream(stream)
    }

    pub(super) fn call_start_request_pending(&self, command_id: &str) -> bool {
        self.capture
            .call_runtime
            .as_ref()
            .is_some_and(|call| call.start_request == command_id)
    }

    pub(super) async fn stop_call_audio(&mut self, voice: CallStopVoice) -> Result<()> {
        if let Some(call) = self.capture.call_runtime.as_ref() {
            let threshold = VoicePolicy::default().rms_threshold.to_string();
            self.log(
                "call_audio_heard",
                &[
                    ("frames", &call.frames_heard.to_string()),
                    ("loudest_rms", &call.loudest_rms.to_string()),
                    ("voiced_frames", &call.voiced_frames.to_string()),
                    (
                        "longest_voiced_ms",
                        &(call.longest_voiced_run_ns / 1_000_000).to_string(),
                    ),
                    ("speech_rms_threshold", &threshold),
                    ("gaps", &call.gaps.to_string()),
                ],
            );
        }
        self.mark_call_stopping();
        if let CallStopVoice::Cancel(cause) = voice {
            self.cancel_voice_transcription(cause).await?;
        }
        if self.state.privacy_stopped() {
            return Ok(());
        }
        self.capture.buffer.voice = None;
        self.stop_recording_audio().await
    }

    pub(super) async fn stop_call_for_reason(&mut self, reason: StopReason) -> Result<()> {
        self.apply_with_call_stop_voice(
            Event::CallStop,
            CallStopVoice::Cancel(VoiceCancellationCause::AutomaticCallStop(reason)),
        )
        .await?;
        self.say(
            Notice::new(NoticeCode::CallStoppedAutomatically)
                .with_token(NoticeKey::Cause, stop_cause(reason)),
        );
        self.store_actor.snapshot(&self.state).await?;
        Ok(())
    }

    pub(super) async fn finish_call_drain(&mut self) -> Result<()> {
        let Some(call) = self.capture.call_runtime.as_ref() else {
            return Ok(());
        };
        let Some(ended_at_ms) = call.closing_at_ms else {
            return Ok(());
        };
        // A chunk of this call the store has not confirmed yet is the closure
        // waiting, not failing: maintenance comes back each tick (SIM-57).
        let sealing = self
            .capture
            .pending_seals
            .iter()
            .any(|segment| segment.metadata.recording_id == call.authorization.id);
        if self.capture.mic_stream.is_some() || self.audio_is_draining() || sealing {
            return Ok(());
        }
        let recording = call.authorization.id.clone();
        let started_at_ms = call.authorization.started_at_ms;
        let gaps = call.gaps;
        self.queue_capture_finalize(
            &recording,
            started_at_ms,
            ended_at_ms.max(started_at_ms),
            gaps,
        )
        .await?;
        self.capture.call_runtime = None;
        Ok(())
    }

    pub(super) async fn call_start_response(
        &mut self,
        command_id: &str,
        result: &Value,
    ) -> Result<bool> {
        let Some(call) = self.capture.call_runtime.as_ref() else {
            return Ok(false);
        };
        if call.start_request != command_id {
            return Ok(false);
        }
        if call.closing_at_ms.is_some() || self.state.durable.call.is_none() {
            // The start can open after a stop overtook it. Let the retired
            // start path close the stream identified by this late receipt.
            return Ok(false);
        }
        let readback: CallStartReceipt = decode_result_body("audio_start", result)?;
        if Some(readback.stream_id.as_str()) != self.capture.mic_stream.as_deref() {
            bail!("La confirmación de llamada corresponde a otro stream");
        }
        if readback.processing != "voice_processing"
            || !readback.processing_enabled
            || readback.processing_bypassed
            || readback.agc_enabled
            || readback.playback_reference != "shared_engine"
            || readback.pipeline_id.is_empty()
            || readback.pipeline_id.len() > 128
        {
            bail!("El Mac no acreditó la ruta VoiceProcessing y su referencia compartida");
        }
        let pipeline = readback.pipeline_id;
        // What the microphone gives under voice processing: its channels and
        // rate, before the helper mixes them to the call's mono stream.
        self.log(
            "call_audio_opened",
            &[
                ("input_channels", &result["input_channels"].to_string()),
                ("sample_rate", &result["sample_rate"].to_string()),
            ],
        );
        let recording = call.authorization.id.clone();
        let job = Job {
            id: format!("call-processing-{recording}"),
            kind: orbit_store::QueueKind::AudioProcessingReceipt.into(),
            state: JobState::Succeeded,
            created_at: now(),
            payload: json!({"recording_id":recording,
                "observed":result,"aec_hardware_validated":false,"origin":"native_start_readback"}),
        };
        self.store_actor
            .run(move |store| Ok(store.enqueue_job(&job)?))
            .await?;
        let call = self
            .capture
            .call_runtime
            .as_mut()
            .context("Llamada perdida")?;
        call.pipeline_id = Some(pipeline);
        call.last_audio = Instant::now();
        self.capture
            .pending_starts
            .retain(|pending| pending != command_id);
        self.state.set_voice_processing_available(true);
        self.state.set_playback_reference_available(true);
        self.apply(Event::CallReady).await?;
        Ok(true)
    }

    /// Supported modes permit only a user-requested attempt. Availability flags
    /// are kept separate from per-stream processing proof and hardware efficacy.
    pub(super) fn observe_voice_capabilities(&mut self, value: &Value) -> Result<()> {
        let previous = self.state.capabilities().clone();
        let observed: VoiceCapabilities = decode_result_body("voice_capabilities", value)?;
        if let Some(modes) = observed.supported_audio_processing {
            self.state.set_voice_processing_supported(
                modes.iter().any(|mode| mode == "voice_processing"),
            );
        }
        if let Some(available) = observed.voice_processing_available {
            self.state.set_voice_processing_available(available);
        }
        if let Some(available) = observed.playback_reference_available {
            self.state.set_playback_reference_available(available);
        }
        if previous != *self.state.capabilities() {
            self.store_actor.defer_snapshot(&self.state)?;
        }
        Ok(())
    }

    pub(super) fn interrupt_call_turn(&mut self) {
        if let Some(call) = self
            .capture
            .call_runtime
            .as_mut()
            .filter(|call| call.closing_at_ms.is_none())
        {
            // Cutting the voice is all this does. The request, the tool and the
            // turn behind it continue, and the answer still arrives as text.
            call.detector = call.detector.cancelled_turn();
            call.preroll.clear();
            call.preroll_ns = 0;
            call.joined = None;
            self.capture.buffer.voice = None;
            if let Some(state) = self.state.durable.call.as_mut() {
                state.phase = CallPhase::Listening;
            }
        }
    }

    pub(super) async fn prepare_call_frame(
        &mut self,
        frame: &Value,
        received_at_ms: i64,
    ) -> Result<TurnAction> {
        let Some(call) = self.capture.call_runtime.as_ref() else {
            return Ok(TurnAction::None);
        };
        if call.closing_at_ms.is_some() || self.state.durable.call.is_none() {
            return Ok(TurnAction::None);
        }
        let Some(expected_pipeline) = call.pipeline_id.as_deref() else {
            return Ok(TurnAction::None);
        };
        let facts = CallFrameFacts::deserialize(frame);
        if processing_pipeline(frame) != Some(expected_pipeline) || facts.is_err() {
            if let Some(call) = self.capture.call_runtime.as_mut() {
                call.gaps = call.gaps.saturating_add(1);
            }
            bail!(
                "Cambió o se perdió la evidencia de procesamiento/referencia de la llamada; requiere reinicio explícito"
            );
        }
        let facts = facts?;
        let pcm = crate::capture::frame_pcm(frame)?;
        let frames = u64::from(facts.frames);
        let rate = u64::from(facts.sample_rate);
        let duration_ns = frames * 1_000_000_000 / rate;
        let policy = VoicePolicy::default();
        let responding = facts.playback_active || self.conversation.speech_task.active().is_some();
        let waiting = self.conversation.voice_task.active().is_some()
            || self.state.durable.active_request.is_some();
        let response_active = responding || waiting;
        let observation = VoiceObservation {
            generation: self.state.generation(),
            authorized: !self.state.privacy_stopped() && self.state.durable.call.is_some(),
            processing_observed: true,
            monotonic_ns: facts.monotonic_ns,
            duration_ns,
            received_at_ms,
            voiced: energy_exceeds(&pcm, policy.rms_threshold),
            response_active,
        };
        let (action, preroll, began) = {
            let call = self
                .capture
                .call_runtime
                .as_mut()
                .context("Llamada ausente")?;
            call.last_audio = Instant::now();
            call.playback_active = facts.playback_active;
            call.frames_heard = call.frames_heard.saturating_add(1);
            call.loudest_rms = call.loudest_rms.max(orbit_core::voice::rms(&pcm));
            if observation.voiced {
                call.voiced_frames = call.voiced_frames.saturating_add(1);
                call.voiced_run_ns = call.voiced_run_ns.saturating_add(duration_ns);
                call.longest_voiced_run_ns = call.longest_voiced_run_ns.max(call.voiced_run_ns);
            } else {
                call.voiced_run_ns = 0;
            }
            let (detector, action) = call.detector.observe(policy, observation);
            call.detector = detector;
            let began = matches!(action, TurnAction::Begin { .. }).then(|| call.detector.clone());
            let preroll: Vec<_> = if matches!(action, TurnAction::Begin { .. }) {
                call.preroll
                    .iter()
                    .map(|(frame, time, _)| (frame.clone(), *time))
                    .collect()
            } else {
                Vec::new()
            };
            call.preroll
                .push_back((frame.clone(), received_at_ms, duration_ns));
            call.preroll_ns = call.preroll_ns.saturating_add(duration_ns);
            while call.preroll_ns > policy.preroll_ms * 1_000_000 {
                let Some((_, _, duration)) = call.preroll.pop_front() else {
                    break;
                };
                call.preroll_ns = call.preroll_ns.saturating_sub(duration);
            }
            (action, preroll, began)
        };
        match action {
            TurnAction::Begin { interrupt } => {
                let joined = if interrupt {
                    self.joinable_turn()
                } else {
                    None
                };
                if interrupt {
                    self.cancel_voice_transcription(if joined.is_some() {
                        VoiceCancellationCause::CallTurnJoined
                    } else {
                        VoiceCancellationCause::CallTurnReplaced
                    })
                    .await?;
                    self.capture.buffer.voice = None;
                    self.apply(Event::Interrupt).await?;
                    // The interrupt retires the turn in course, as Esc does,
                    // and that reset the detector this frame just opened a
                    // turn in: the turn never ended and the next one found
                    // its buffer taken (SIM-57).
                    if let (Some(call), Some(began)) = (self.capture.call_runtime.as_mut(), began) {
                        call.detector = began;
                    }
                }
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
                if let Some(call) = self.capture.call_runtime.as_mut() {
                    call.joined = joined;
                }
                self.apply(Event::CallTurnStarted).await?;
            }
            TurnAction::Stop(reason) => {
                if reason == StopReason::Idle {
                    self.log_call_idle_stop(reason, received_at_ms);
                }
                self.stop_call_for_reason(reason).await?;
            }
            TurnAction::None
                if !self
                    .capture
                    .call_runtime
                    .as_ref()
                    .is_some_and(|call| call.detector.is_capturing()) =>
            {
                let phase = if responding {
                    CallPhase::Responding
                } else if waiting {
                    CallPhase::Waiting
                } else {
                    CallPhase::Listening
                };
                if self
                    .state
                    .durable
                    .call
                    .as_ref()
                    .is_some_and(|call| call.phase != phase)
                {
                    self.apply(Event::CallPhase(phase)).await?;
                }
            }
            TurnAction::None | TurnAction::Finish { .. } => {}
        }
        Ok(action)
    }

    /// Runs after the current frame has reached both original and derived buffers.
    pub(super) async fn finish_call_frame(&mut self, action: TurnAction) -> Result<()> {
        if let TurnAction::Finish { submit } = action {
            self.finish_call_voice(submit).await?;
            if let Some(call) = self.state.durable.call.as_mut() {
                call.phase = if submit {
                    CallPhase::Waiting
                } else {
                    CallPhase::Listening
                };
            }
            self.store_actor.snapshot(&self.state).await?;
        }
        Ok(())
    }

    /// The turn whose transcription is still in course, when Mars speaks
    /// again before it reached Deimos: her new words continue it, so its voice
    /// goes first in the new turn instead of being cancelled and lost
    /// (SIM-57). A turn already sent to Deimos is not carried: he has it.
    fn joinable_turn(&mut self) -> Option<CallTurnAudio> {
        if self.state.durable.active_request.is_some()
            || self.conversation.voice_task.active().is_none()
        {
            return None;
        }
        let operation = self.conversation.voice_operation.as_deref()?;
        self.capture
            .call_runtime
            .as_mut()?
            .submitted
            .take()
            .filter(|turn| turn.operation_id == operation)
    }

    /// Keeps a call turn's voice while it is transcribed (see `joinable_turn`).
    pub(super) fn keep_submitted_call_turn(
        &mut self,
        operation_id: &str,
        voice: crate::capture::VoiceBuffer,
    ) {
        if let Some(call) = self
            .capture
            .call_runtime
            .as_mut()
            .filter(|call| call.closing_at_ms.is_none())
        {
            call.submitted = Some(CallTurnAudio {
                operation_id: operation_id.to_owned(),
                sample_rate: voice.sample_rate,
                channels: voice.channels,
                started_at_ms: voice.started_at_ms,
                monotonic_ns: voice.monotonic_ns,
                last_sequence: voice.last_sequence(),
                pcm: voice.pcm,
            });
        }
    }

    /// Puts the carried turn's voice before the turn about to be sent, when
    /// both are the same format and the whole still fits one voice turn's
    /// bounds (60 s, and the encrypted chunk). Otherwise the carried voice is
    /// dropped with its numbers in the log.
    pub(super) fn prepend_joined_call_turn(&mut self) {
        let Some(joined) = self
            .capture
            .call_runtime
            .as_mut()
            .and_then(|call| call.joined.take())
        else {
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

    /// The main loop's facts at the start of a tick, before maintenance runs.
    pub(super) fn note_loop_tick(&mut self, queued_native_events: usize, last_tick: Duration) {
        if let Some(call) = self.capture.call_runtime.as_mut() {
            call.queued_native_events = queued_native_events;
            call.last_tick = last_tick;
        }
    }

    pub(super) async fn check_call_health(&mut self) -> Result<()> {
        let Some(call) = self
            .capture
            .call_runtime
            .as_ref()
            .filter(|call| call.closing_at_ms.is_none())
        else {
            return Ok(());
        };
        let authorized = Duration::from_millis(
            call.authorization
                .expires_at_ms
                .saturating_sub(call.authorization.started_at_ms) as u64,
        );
        // A frame still in the loop's queue was sent; absence is judged only
        // once nothing is waiting (SIM-55).
        let no_audio =
            call.last_audio.elapsed() > CALL_AUDIO_SILENCE_LIMIT && call.queued_native_events == 0;
        let reason = if no_audio {
            StopReason::NoAudio
        } else if call.started.elapsed() >= authorized {
            StopReason::Expired
        } else {
            return Ok(());
        };
        if no_audio {
            self.log_call_idle_stop(reason, now());
        }
        self.stop_call_for_reason(reason).await
    }

    /// Why a call stopped for lack of audio or of conversation, with the
    /// numbers that tell one from the other and a held loop from both.
    fn log_call_idle_stop(&self, reason: StopReason, now_ms: i64) {
        let Some(call) = self.capture.call_runtime.as_ref() else {
            return;
        };
        let deimos_speaking =
            call.playback_active || self.conversation.speech_task.active().is_some();
        let limit_ms = if reason == StopReason::NoAudio {
            CALL_AUDIO_SILENCE_LIMIT.as_millis().to_string()
        } else {
            VoicePolicy::default().idle_timeout_ms.to_string()
        };
        self.log(
            "call_stopped_idle",
            &[
                ("cause", stop_cause(reason)),
                (
                    "last_frame_age_ms",
                    &call.last_audio.elapsed().as_millis().to_string(),
                ),
                (
                    "mars_silent_ms",
                    &now_ms
                        .saturating_sub(call.detector.last_activity_ms())
                        .to_string(),
                ),
                ("limit_ms", &limit_ms),
                (
                    "queued_native_events",
                    &call.queued_native_events.to_string(),
                ),
                (
                    "deimos_speaking",
                    if deimos_speaking { "true" } else { "false" },
                ),
                ("last_tick_ms", &call.last_tick.as_millis().to_string()),
                ("frames", &call.frames_heard.to_string()),
            ],
        );
    }
}

/// The cause token a stop is said with: one word per cause.
fn stop_cause(reason: StopReason) -> &'static str {
    match reason {
        StopReason::Unauthorized => "unauthorized",
        StopReason::GenerationChanged => "generation_changed",
        StopReason::ProcessingLost => "processing_lost",
        StopReason::Discontinuity => "discontinuity",
        StopReason::Expired => "expired",
        StopReason::Idle => "idle",
        StopReason::NoAudio => "no_audio",
    }
}

fn processing_pipeline(value: &Value) -> Option<&str> {
    let evidence = ProcessingEvidence::deserialize(value).ok()?;
    if evidence.processing != "voice_processing"
        || !evidence.processing_enabled
        || evidence.processing_bypassed
        || evidence.agc_enabled
        || evidence.playback_reference != "shared_engine"
    {
        return None;
    }
    Some(evidence.pipeline_id).filter(|id| !id.is_empty() && id.len() <= 128)
}

#[cfg(test)]
mod result_fixture_tests {
    use super::*;

    #[test]
    fn call_start_and_ready_fixtures_use_the_call_readers() -> Result<()> {
        let start: Value = serde_json::from_str(include_str!(
            "../../../../contracts/fixtures/native/results/audio_start-mic.json"
        ))?;
        let receipt: CallStartReceipt = decode_result_body("audio_start", &start)?;
        assert_eq!(receipt.stream_id, "stream-1");
        assert_eq!(receipt.processing, "voice_processing");
        assert!(receipt.processing_enabled);
        assert!(!receipt.processing_bypassed);
        assert!(!receipt.agc_enabled);
        assert_eq!(receipt.playback_reference, "shared_engine");
        assert_eq!(receipt.pipeline_id, "fixture");
        let ready: Value = serde_json::from_str(include_str!(
            "../../../../contracts/fixtures/native/events/ready.json"
        ))?;
        let facts: VoiceCapabilities = decode_result_body("voice_capabilities", &ready)?;
        assert_eq!(facts.voice_processing_available, Some(false));
        assert_eq!(facts.playback_reference_available, Some(false));
        assert!(
            facts
                .supported_audio_processing
                .context("processing modes")?
                .iter()
                .any(|mode| mode == "voice_processing")
        );
        Ok(())
    }
}
