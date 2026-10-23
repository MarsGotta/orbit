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

#[path = "call_receipts.rs"]
mod receipts;
use health::stop_cause;
use receipts::processing_pipeline;

#[path = "call_frames.rs"]
mod frames;

#[path = "call_health.rs"]
mod health;

const MICROPHONE_SOURCE: &str = "mic_local";

/// How long a call goes without any microphone frame before the daemon stops
/// it as `no_audio`.
///
/// What it protects: a call whose microphone stopped delivering (the tap
/// gone, the device changed, the helper alive but sending nothing) must not
/// stay open looking as if it listened. Why 3 s: it is the window in which
/// the helper declares the same loss on its side (`no_capture_frames` in
/// `AudioCapture.swift`); the two are independent and need not agree. What it
/// is not: the user's silence, which is still audio and has its own minute
/// (`VoicePolicy::idle_timeout_ms`), and not a budget for the daemon's loop:
/// frames waiting in the loop's queue are audio arriving, so a loop that was
/// held reads them before this is judged.
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
    /// captured now carries.
    submitted: Option<CallTurnAudio>,
    joined: Option<CallTurnAudio>,
    /// A turn in which nothing was heard (the hub found no words in it),
    /// waiting for what the user says next: they may not have finished.
    /// The next turn carries it, within the same bounds as a join.
    carried: Option<CallTurnAudio>,
    /// The turn being captured began while Deimos answered: it interrupts
    /// him only once its words are known (`hear_over_answer`).
    capturing_over_answer: bool,
    /// The turn spoken over Deimos's answer whose words are being asked for.
    over_answer: Option<OverAnswerTurn>,
    /// What the main loop knew when the current tick began: the
    /// helper events it had not read yet, and how long the tick before took.
    queued_native_events: usize,
    last_tick: Duration,
}

/// What one call frame did to the turn detector: its action, the preroll a
/// turn that begins starts with, and the detector as that frame left it.
type ObservedFrame = (TurnAction, Vec<(Value, i64)>, Option<TurnDetector>);

impl CallRuntime {
    /// A call just authorized, whose microphone start is `request`.
    fn new(generation: u64, authorization: CallState, request: String) -> Self {
        Self {
            detector: TurnDetector::authorized(
                generation,
                authorization.started_at_ms,
                authorization.expires_at_ms,
            ),
            authorization,
            start_request: request,
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
            carried: None,
            capturing_over_answer: false,
            over_answer: None,
            queued_native_events: 0,
            last_tick: Duration::ZERO,
        }
    }
}

impl crate::daemon::audio::AudioHandle {
    pub(super) fn note_call_gap(&mut self, recording_id: &str) {
        if let Some(call) = self
            .call
            .as_mut()
            .filter(|call| call.authorization.id == recording_id)
        {
            call.gaps = call.gaps.saturating_add(1);
        }
    }

    pub(super) fn mark_call_stopping(&mut self) {
        if let Some(call) = self.call.as_mut() {
            call.closing_at_ms.get_or_insert_with(now);
            call.preroll.clear();
            call.preroll_ns = 0;
            if self
                .pending_starts
                .iter()
                .any(|id| id == &call.start_request)
                && let Some(stream) = self.mic_stream.as_ref()
            {
                self.retired_call_starts
                    .insert(call.start_request.clone(), stream.clone());
            }
            self.pending_starts.retain(|id| id != &call.start_request);
        }
    }

    pub(super) fn closing_call_owns_stream(&self, stream: &str) -> bool {
        self.call.as_ref().is_some_and(|call| {
            call.closing_at_ms.is_some() && self.mic_stream.as_deref() == Some(stream)
        })
    }

    pub(super) fn closing_call_start_owns_stream(&self, command_id: &str, stream: &str) -> bool {
        self.call_start_request_pending(command_id) && self.closing_call_owns_stream(stream)
    }

    pub(super) fn call_start_request_pending(&self, command_id: &str) -> bool {
        self.call
            .as_ref()
            .is_some_and(|call| call.start_request == command_id)
    }

    /// One call frame through the turn detector: the call's counters, the
    /// detector's action, and for a turn that begins, the preroll it starts
    /// with and the detector as that frame left it.
    fn observe_call_audio(
        &mut self,
        frame: &Value,
        received_at_ms: i64,
        observation: VoiceObservation,
        pcm: &[u8],
        playback_active: bool,
    ) -> Result<ObservedFrame> {
        let policy = VoicePolicy::default();
        let duration_ns = observation.duration_ns;
        let call = self.call.as_mut().context("Llamada ausente")?;
        call.last_audio = Instant::now();
        call.playback_active = playback_active;
        call.frames_heard = call.frames_heard.saturating_add(1);
        call.loudest_rms = call.loudest_rms.max(orbit_core::voice::rms(pcm));
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
        Ok((action, preroll, began))
    }

    /// The main loop's facts at the start of a tick, before maintenance runs.
    pub(super) fn note_loop_tick(&mut self, queued_native_events: usize, last_tick: Duration) {
        if let Some(call) = self.call.as_mut() {
            call.queued_native_events = queued_native_events;
            call.last_tick = last_tick;
        }
    }
}

#[cfg(test)]
#[path = "call_overflow_tests.rs"]
mod overflow_tests;

#[path = "call_turns.rs"]
mod turns;
pub(super) use turns::{CallTurnAudio, OverAnswerTurn, call_turn_audio};

impl Runtime {
    pub(super) async fn start_call_audio(&mut self) -> Result<()> {
        if self.audio.retired_call_starts.len() >= MAX_RETIRED_CALL_STARTS {
            let pending = self.audio.retired_call_starts.len();
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
        let authorization = self.state.durable.voice.call.clone();
        let authorization = authorization.context("Llamada sin autorización local")?;
        if self.audio.call.is_some()
            || self.audio.mic_stream.is_some()
            || self.audio.remote_stream.is_some()
            || self.capture.buffer.voice.is_some()
            || self.assistant.voice_task.active().is_some()
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
        self.audio.call = Some(CallRuntime::new(
            self.state.generation(),
            authorization,
            request.clone(),
        ));
        self.audio.pending_starts.push(request);
        self.audio.mic_stream = Some(stream);
        self.helper.record_pending(&command);
        tokio::time::timeout(Duration::from_millis(500), self.helper.sender.send(command))
            .await??;
        Ok(())
    }

    pub(super) async fn stop_call_audio(&mut self, voice: CallStopVoice) -> Result<()> {
        if let Some(call) = self.audio.call.as_ref() {
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
        self.audio.mark_call_stopping();
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
        let Some(call) = self.audio.call.as_ref() else {
            return Ok(());
        };
        let Some(ended_at_ms) = call.closing_at_ms else {
            return Ok(());
        };
        // A chunk of this call the store has not confirmed yet is the closure
        // waiting, not failing: maintenance comes back each tick.
        let sealing = self
            .capture
            .pending_seals
            .iter()
            .any(|segment| segment.metadata.recording_id == call.authorization.id);
        if self.audio.mic_stream.is_some() || self.audio.audio_is_draining() || sealing {
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
        self.audio.call = None;
        Ok(())
    }
}
