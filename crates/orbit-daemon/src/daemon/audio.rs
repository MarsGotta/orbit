//! Audio lifecycle commands complete only after the native reader drains the
//! preceding frames and delivers the matching response in the same stream.
use super::{Runtime, id, now};
use crate::native;
use orbit_core::notice::{Notice, NoticeCode, NoticeKey};

#[cfg(test)]
#[path = "audio_tests.rs"]
pub(super) mod tests;
use anyhow::{Result, bail};
use orbit_core::wire::native::{
    NativeAudioIdentityRef, NativeCommandHeader, NativeEventHeader, decode_result_body,
};
use orbit_core::{CapturePhase, Event, decide, protocol::MeetingAudio};
use orbit_store::{Job, JobState};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{collections::BTreeMap, time::Duration};
use tokio::time::Instant;

const AUDIO_SILENCE_TIMEOUT: Duration = Duration::from_secs(3);
const AUDIO_DRAIN_LIMIT: Duration = Duration::from_secs(15);
const MICROPHONE_SOURCE: &str = "mic_local";

/// Bounds the receipt ledger for selective stops after retired starts. The
/// helper command channel holds 64 orders, so 64 unmatched stops already fill
/// one channel's worth of debt; a 65th uses a whole-helper cut with its counts
/// instead of losing an order. This is not a deadline or a capture limit.
pub(super) const MAX_RETIRED_STOPS: usize = 64;

#[derive(Clone, Copy)]
pub(super) enum VoiceCancellationCause {
    Authority(orbit_core::authority::AuthorityChange),
    AutomaticCallStop(orbit_core::voice::StopReason),
    CallTurnReplaced,
    /// The user spoke again before the turn reached Deimos; their voice goes
    /// on in the next turn.
    CallTurnJoined,
    HelperCut,
    CaptureCut,
}

#[derive(Clone, Copy)]
pub(super) enum CallStopVoice {
    KeepTranscription,
    Cancel(VoiceCancellationCause),
}

struct PendingStop {
    stream: String,
    source: &'static str,
    answered: bool,
}

pub(super) struct RetiredStreamStop {
    pub(super) stream: String,
    pub(super) sent: bool,
    /// A superseded call stop still has a receipt to read, but only the new
    /// selective stop may finish the late-started stream.
    pub(super) may_finish_call: bool,
}

#[derive(Deserialize)]
struct DrainAcknowledgement {
    drain_complete: bool,
}

/// Which whole-helper stop is waiting for its drain.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum FullStop {
    /// The user's stop: the helper writes its durable latch (`privacy_stop`).
    Privacy,
    /// A cut, a closing or a revocation: the helper stops every capture
    /// source and leaves its latch as it was (`capture_stop`).
    Cut,
}

impl FullStop {
    fn command(self) -> &'static str {
        match self {
            Self::Privacy => "privacy_stop",
            Self::Cut => "capture_stop",
        }
    }
}

/// Identity links survive provider failure; no transcription is a replacement
/// for the original microphone bytes stored under this source.
pub(super) struct VoiceSource {
    pub recording_id: String,
    pub chunk_id: String,
    pub session_id: Option<String>,
    pub message_id: String,
    pub duration_ms: u64,
}

#[derive(Default)]
pub(super) struct AudioLifecycle {
    pub(super) signal: Option<super::track_signal::MeetingSignal>,
    stops: BTreeMap<String, PendingStop>,
    recording_end: Option<bool>,
    voice_end: bool,
    pub(super) submit_voice: bool,
    /// The whole-helper stop still waiting for its drain, by command id.
    pub(super) full_stop: Option<(String, FullStop)>,
    /// A whole-helper stop timed out before entering the bounded channel.
    /// A timeout is backpressure, not absence: while the helper exists,
    /// maintenance resends this exact stop ID and then waits for its drain ACK.
    full_stop_send_owed: bool,
    /// A whole-helper stop (hers or a cut) whose local flush has not
    /// succeeded yet: the maintenance retries it until the audio held in
    /// memory is durable. Cleared with the lifecycle when the flush lands.
    full_stop_owed: bool,
    deadline: Option<Instant>,
    started_at: Option<Instant>,
    last_liveness: Option<Instant>,
}

impl AudioLifecycle {
    fn pending(&self) -> bool {
        !self.stops.is_empty() || self.full_stop.is_some()
    }

    /// Whether a finished recording is still waiting to be closed.
    ///
    /// `complete_audio_drain` puts this back when it finds a seal
    /// still in flight, because a recording cannot be declared finalized
    /// while a chunk of it is unwritten. It is the one flag that says "the Mac
    /// has confirmed, the audio is in, and all that is left is to say so", and
    /// it is what the daemon has to come back to once the seals land.
    fn finish_deferred(&self) -> bool {
        self.recording_end.is_some()
    }

    pub(super) fn response_command(&self, command_id: &str) -> Option<&'static str> {
        if self.stops.contains_key(command_id) {
            return Some("audio_stop");
        }
        self.full_stop
            .as_ref()
            .filter(|(id, _)| id == command_id)
            .map(|(_, kind)| kind.command())
    }
}

impl Runtime {
    fn begin_audio_drain(&mut self, announce: bool) {
        let observed_at = Instant::now();
        let started_at = *self.capture.audio.started_at.get_or_insert(observed_at);
        self.capture.audio.last_liveness.get_or_insert(observed_at);
        self.capture.audio.deadline = Some(
            observed_at
                .checked_add(AUDIO_SILENCE_TIMEOUT)
                .unwrap_or(observed_at)
                .min(
                    started_at
                        .checked_add(AUDIO_DRAIN_LIMIT)
                        .unwrap_or(started_at),
                ),
        );
        self.state.views.draining_since_ms.get_or_insert_with(now);
        if announce {
            self.say(NoticeCode::AudioDrainWaiting);
        }
    }

    pub(super) fn observe_audio_helper_liveness(&mut self, event: &Value) {
        if !self.capture.audio.pending()
            || !matches!(
                NativeEventHeader::from_value(event)
                    .ok()
                    .map(|header| header.event),
                Some("state" | "audio" | "response" | "local_rearm")
            )
        {
            return;
        }
        let observed_at = Instant::now();
        let Some(started_at) = self.capture.audio.started_at else {
            return;
        };
        self.capture.audio.last_liveness = Some(observed_at);
        self.capture.audio.deadline = Some(
            observed_at
                .checked_add(AUDIO_SILENCE_TIMEOUT)
                .unwrap_or(observed_at)
                .min(
                    started_at
                        .checked_add(AUDIO_DRAIN_LIMIT)
                        .unwrap_or(started_at),
                ),
        );
    }

    /// Attribute a safety fact to the actual producer owner, not the UI phase.
    /// Keep it in RAM even if persisting the snapshot fails; pretending no gap
    /// would be less safe than retaining this conservative monotonic counter.
    pub(super) fn note_bound_audio_gap(&mut self, source: &str) -> Result<()> {
        let Some(owner) = self
            .capture
            .buffer
            .recording_owner(source)
            .map(str::to_owned)
        else {
            return Ok(());
        };
        self.note_call_gap(&owner);
        if let Some(recording) = self
            .state
            .durable
            .recording
            .as_mut()
            .filter(|recording| recording.id == owner)
        {
            recording.gaps = recording.gaps.saturating_add(1);
            if let Some(track) = recording
                .tracks
                .iter_mut()
                .find(|track| track.name == source)
            {
                track.gaps = track.gaps.saturating_add(1);
            }
            self.store_actor.defer_snapshot(&self.state)?;
        }
        Ok(())
    }

    pub(super) fn note_bound_track_gap(&mut self, source: &str) -> Result<()> {
        let Some(owner) = self
            .capture
            .buffer
            .recording_owner(source)
            .map(str::to_owned)
        else {
            return Ok(());
        };
        let Some(track) = self
            .state
            .durable
            .recording
            .as_mut()
            .filter(|recording| recording.id == owner)
            .and_then(|recording| {
                recording
                    .tracks
                    .iter_mut()
                    .find(|track| track.name == source)
            })
        else {
            return Ok(());
        };
        track.gaps = track.gaps.saturating_add(1);
        self.store_actor.defer_snapshot(&self.state)?;
        Ok(())
    }

    /// Timeout/EOF is not a successful barrier. Preserve what arrived and
    /// declare uncertain missing tail samples before clearing source identity.
    pub(super) async fn abandon_audio_drain(&mut self) -> Result<()> {
        let mut gap_error = None;
        for source in ["mic_local", "remote_app"] {
            if let Err(error) = self.note_bound_audio_gap(source) {
                gap_error = Some(error);
            }
        }
        self.flush_stopped_audio().await?;
        if let Some(error) = gap_error {
            return Err(error);
        }
        Ok(())
    }

    pub(super) fn retire_voice_transcription(
        &mut self,
        change: orbit_core::authority::AuthorityChange,
    ) -> bool {
        self.mark_voice_transcription_cancelled(VoiceCancellationCause::Authority(change))
    }

    fn mark_voice_transcription_cancelled(&mut self, cause: VoiceCancellationCause) -> bool {
        let op = self
            .conversation
            .voice_task
            .ticket()
            .map(|ticket| ticket.op().clone());
        self.conversation.voice_task.cancel();
        let pending = self.conversation.voice_operation.take();
        if let Some(operation_id) = pending.as_ref() {
            // A turn spoken over Deimos's answer reached no one and was never
            // stored: the change that stops its transcription retires it.
            let _ = self.take_over_answer_turn(operation_id);
            let code = match cause {
                VoiceCancellationCause::Authority(change) => match change {
                    orbit_core::authority::AuthorityChange::UserStop => {
                        "voice_transcription_privacy_stopped"
                    }
                    orbit_core::authority::AuthorityChange::Interrupt => {
                        // wire/stored name kept for compatibility with 0.9.0
                        "voice_transcription_interrupted_by_mars"
                    }
                    orbit_core::authority::AuthorityChange::Closing => {
                        "voice_transcription_orbit_closing"
                    }
                    orbit_core::authority::AuthorityChange::Revoked
                    | orbit_core::authority::AuthorityChange::Rearm
                    | orbit_core::authority::AuthorityChange::RearmWithoutStop
                    | orbit_core::authority::AuthorityChange::HubLink
                    | orbit_core::authority::AuthorityChange::HubTransportReady
                    | orbit_core::authority::AuthorityChange::HubLinkOffline
                    | orbit_core::authority::AuthorityChange::HubDisconnected
                    | orbit_core::authority::AuthorityChange::Conversation
                    | orbit_core::authority::AuthorityChange::Device => {
                        "voice_transcription_generation_changed"
                    }
                },
                VoiceCancellationCause::AutomaticCallStop(reason) => match reason {
                    orbit_core::voice::StopReason::Unauthorized
                    | orbit_core::voice::StopReason::GenerationChanged
                    | orbit_core::voice::StopReason::ProcessingLost
                    | orbit_core::voice::StopReason::Discontinuity
                    | orbit_core::voice::StopReason::Expired
                    | orbit_core::voice::StopReason::Idle
                    | orbit_core::voice::StopReason::NoAudio => {
                        "voice_transcription_call_stopped_automatically"
                    }
                },
                VoiceCancellationCause::CallTurnReplaced => {
                    "voice_transcription_call_turn_replaced"
                }
                VoiceCancellationCause::CallTurnJoined => "voice_transcription_joined_next_turn",
                VoiceCancellationCause::HelperCut => "voice_transcription_helper_cut",
                VoiceCancellationCause::CaptureCut => "voice_transcription_capture_cut",
            };
            if let Some(voice) = self
                .state
                .durable
                .turn
                .as_mut()
                .and_then(|turn| turn.untranscribed_voice_mut(operation_id))
            {
                voice.failure = Some(orbit_core::VoiceTranscriptionFailure::local(code));
            }
        }
        if pending.is_some()
            && let Some(op) = op
        {
            let mut notice = Notice::new(NoticeCode::VoiceTurnCancelled);
            notice.op = Some(op);
            // Close the old operation without replacing the stop's own cause.
            self.state.durable.notice_log.record(notice);
            self.report_notices();
        }
        // A voice turn still held by the user can finish locally, but must not
        // be submitted after Esc, including while waiting for native stop ACK.
        self.capture.audio.submit_voice = false;
        pending.is_some()
    }

    pub(super) async fn cancel_voice_transcription(
        &mut self,
        cause: VoiceCancellationCause,
    ) -> Result<()> {
        let pending = self.mark_voice_transcription_cancelled(cause);
        if pending {
            self.store_actor.snapshot(&self.state).await?;
            self.work.durable_snapshot = crate::daemon::DurableSnapshot::from(&self.state);
        }
        Ok(())
    }

    pub(super) fn discard_refused_voice(&mut self) {
        self.capture.buffer.voice = None;
        self.capture.audio.submit_voice = false;
        self.capture.audio.voice_end = false;
    }

    pub(super) fn validate_audio_transition(&self, event: &Event) -> Result<()> {
        if matches!(
            event,
            Event::StartMeeting { .. }
                | Event::CallStart { .. }
                | Event::ResumeMeeting
                | Event::VoiceStart
                | Event::LocalRearm
        ) && (self.capture.audio.pending()
            || self.capture.audio.recording_end.is_some()
            || self.capture.audio.voice_end)
        {
            return Err(orbit_core::refusal::Refusal::Busy {
                operation: orbit_core::refusal::Operation::AudioDrain,
                retry_after_ms: None,
            }
            .into());
        }
        if matches!(event, Event::VoiceStart)
            && (self.conversation.voice_task.active().is_some()
                || self.capture.buffer.voice.is_some())
        {
            return Err(orbit_core::refusal::Refusal::Busy {
                operation: orbit_core::refusal::Operation::Interaction,
                retry_after_ms: None,
            }
            .into());
        }
        if matches!(event, Event::CallStart { .. })
            && (self.capture.call_runtime.is_some()
                || self.capture.mic_stream.is_some()
                || self.capture.remote_stream.is_some()
                || self.capture.buffer.voice.is_some()
                || self.conversation.voice_task.active().is_some())
        {
            return Err(orbit_core::refusal::Refusal::Busy {
                operation: orbit_core::refusal::Operation::AudioSession,
                retry_after_ms: None,
            }
            .into());
        }
        Ok(())
    }

    pub(super) async fn start_recording_audio(
        &mut self,
        recording_id: String,
        audio: MeetingAudio,
    ) -> Result<()> {
        let epoch = id();
        self.capture
            .buffer
            .begin_recording_with_epoch(recording_id.clone(), epoch.clone())?;
        // Reserve exhaustion belongs to the recording whose unsealed PCM
        // overflowed. A later capture starts with a fresh 64 MiB reserve.
        if self.capture.pending_seals.is_empty() && !self.capture.buffer.has_pending_audio() {
            self.capture.reserve_exhausted = false;
        }
        for track in ["mic_local", "remote_app"] {
            if let Some(receipt) = self.reader.latest_chunk_receipt(&recording_id, track)? {
                self.capture.buffer.restore_position(&receipt.metadata)?;
            }
        }
        let (expected_tracks, capture_scope, application_pid) = match audio {
            MeetingAudio::System => (vec!["mic_local", "remote_app"], Some("system"), None),
            MeetingAudio::Application { application_pid } => (
                vec!["mic_local", "remote_app"],
                Some("application"),
                Some(application_pid),
            ),
            MeetingAudio::MicrophoneOnly => (vec![MICROPHONE_SOURCE], None, None),
        };
        let mut payload = json!({"recording_id":recording_id,
            "capture_epoch_id":epoch,"expected_tracks":expected_tracks,"clock_domain":"mach_absolute_time",
            "origin":self.capture_origin(&recording_id)});
        if let Some(fields) = payload.as_object_mut() {
            if let Some(scope) = capture_scope {
                fields.insert("capture_scope".into(), json!(scope));
            }
            if let Some(pid) = application_pid {
                fields.insert("application_pid".into(), json!(pid));
            }
        }
        let job = Job {
            id: format!("capture-epoch-{epoch}"),
            kind: orbit_store::QueueKind::CaptureEpoch.into(),
            state: JobState::Succeeded,
            created_at: now(),
            payload,
        };
        self.store_actor
            .run(move |store| Ok(store.enqueue_job(&job)?))
            .await?;
        if self.capture.mic_stream.is_none() {
            let stream = id();
            self.start_track("mic_local", &stream, Some(audio)).await?;
            self.capture.mic_stream = Some(stream);
        }
        if !matches!(audio, MeetingAudio::MicrophoneOnly) {
            let stream = id();
            self.start_track("remote_app", &stream, Some(audio)).await?;
            self.capture.remote_stream = Some(stream);
        }
        self.begin_recording_signal(recording_id, epoch);
        self.capture_started_if_ready().await?;
        // «Oye Deimos» goes with the meeting, on its microphone
        // stream, whoever opened it.
        self.sync_wake_phrase().await?;
        Ok(())
    }

    pub(super) async fn stop_recording_audio(&mut self) -> Result<()> {
        // Pausing or finishing turns «Oye Deimos» off first, also
        // when a voice note keeps the stream open.
        self.sync_wake_phrase().await?;
        if self.state.privacy_stopped() {
            // The user's stop latches the helper; a revocation stops it
            // without writing a latch nobody pressed.
            let kind = if orbit_core::privacy_stop::latches_helper(&self.state) {
                FullStop::Privacy
            } else {
                FullStop::Cut
            };
            return self.request_full_audio_stop(kind).await;
        }
        let end = self
            .state
            .durable
            .recording
            .as_ref()
            .is_none_or(|recording| recording.phase != CapturePhase::Paused);
        self.capture.audio.recording_end = Some(end);
        if let Some(stream) = self.capture.remote_stream.clone() {
            self.request_stream_stop("remote_app", stream).await?;
        } else {
            let segments = self.capture.buffer.pause_source("remote_app")?;
            self.seal(segments)?;
        }
        if !self.state.durable.voice_active {
            if let Some(stream) = self.capture.mic_stream.clone() {
                self.request_stream_stop("mic_local", stream).await?;
            } else {
                let segments = self.capture.buffer.pause_source("mic_local")?;
                self.seal(segments)?;
            }
        } else {
            // The native microphone remains live for voice. Recording stops at
            // this explicit local boundary and must not resume from later PCM.
            let segments = self.capture.buffer.pause_source("mic_local")?;
            self.seal(segments)?;
        }
        self.complete_audio_drain().await
    }

    async fn set_microphone_purpose(
        &mut self,
        purpose: orbit_core::wire::native::NativeMicrophonePurpose,
    ) -> Result<()> {
        if let Some(stream) = self.capture.mic_stream.clone() {
            self.native_send(
                "audio_purpose",
                json!({"stream_id": stream, "purpose": purpose}),
            )
            .await?;
        }
        Ok(())
    }

    pub(super) async fn start_voice_audio(&mut self) -> Result<()> {
        self.capture.buffer.begin_voice()?;
        self.capture.audio.submit_voice = true;
        if self.capture.mic_stream.is_none() {
            let stream = id();
            self.start_track("mic_local", &stream, None).await?;
            self.capture.mic_stream = Some(stream);
        } else {
            self.set_microphone_purpose(
                orbit_core::wire::native::NativeMicrophonePurpose::VoiceNote,
            )
            .await?;
        }
        Ok(())
    }

    pub(super) async fn finish_voice_audio(&mut self) -> Result<()> {
        self.capture.audio.voice_end = true;
        if self.recording_active() {
            self.set_microphone_purpose(orbit_core::wire::native::NativeMicrophonePurpose::Meeting)
                .await?;
        }
        if !self.recording_active()
            && let Some(stream) = self.capture.mic_stream.clone()
        {
            self.request_stream_stop("mic_local", stream).await?;
        }
        self.complete_audio_drain().await
    }

    /// Esc: the note finishes as any other and is kept on this Mac like a
    /// cancelled turn, but it is never transcribed or sent.
    pub(super) async fn cancel_voice_audio(&mut self) -> Result<()> {
        self.capture.audio.submit_voice = false;
        self.finish_voice_audio().await
    }

    async fn request_stream_stop(&mut self, source: &'static str, stream: String) -> Result<()> {
        if self
            .capture
            .audio
            .stops
            .values()
            .any(|pending| pending.stream == stream)
        {
            return Ok(());
        }
        let command = native::command("audio_stop", json!({"stream_id":stream}))?;
        let command_id = NativeCommandHeader::from_value(&command)?.id.to_owned();
        self.capture.audio.stops.insert(
            command_id,
            PendingStop {
                stream,
                source,
                answered: false,
            },
        );
        self.begin_audio_drain(true);
        self.record_pending_native(&command);
        match tokio::time::timeout(Duration::from_millis(500), self.native.send(command)).await {
            Ok(Ok(())) => Ok(()),
            Ok(Err(_closed)) => {
                bail!("El Mac no recibió la orden audio_stop: el canal con Orbit.app está cerrado")
            }
            Err(_elapsed) => bail!(
                "El Mac no recibió la orden audio_stop en 500 ms; el canal sigue presente pero no aceptó la orden dentro de su plazo"
            ),
        }
    }

    /// A late start can open a stream after its capture was drained. This
    /// stop owns only that stream and its receipt; it must not pause the source
    /// buffer or the microphone of the capture that followed it.
    pub(super) async fn request_retired_stream_stop(&mut self, stream: &str) -> Result<()> {
        if self.capture.retired_stops.len() >= MAX_RETIRED_STOPS {
            self.retired_stop_capacity_exhausted().await?;
            return Ok(());
        }
        let command = native::command("audio_stop", json!({"stream_id":stream}))?;
        let command_id = NativeCommandHeader::from_value(&command)?.id.to_owned();
        self.capture.retired_stops.insert(
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

    async fn retired_stop_capacity_exhausted(&mut self) -> Result<()> {
        let pending = self.capture.retired_stops.len();
        self.log(
            "retired_stop_capacity_exhausted",
            &[
                ("pending", &pending.to_string()),
                ("max", &MAX_RETIRED_STOPS.to_string()),
            ],
        );
        let cause = format!(
            "retired_stop_capacity_exhausted: {pending} paradas selectivas pendientes; máximo {MAX_RETIRED_STOPS}. Se pide una parada completa al Mac."
        );
        self.contain_coded(
            orbit_core::privacy_stop::StopCause::HelperIntegrityFailure,
            "retired_stop_capacity_exhausted",
            &cause,
        )
        .await;
        if !self.state.privacy_stopped() && self.capture.audio.full_stop.is_none() {
            self.request_full_audio_stop(FullStop::Cut).await?;
        }
        Ok(())
    }

    async fn send_retired_stream_stop(&mut self, command_id: &str) {
        let Some(stream) = self
            .capture
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
        self.record_pending_native(&command);
        // The established 500 ms helper-channel budget protects the actor
        // from backpressure. It does not prove the retired stream closed: only
        // its matching drain receipt does, and an unsent stop stays owed.
        match tokio::time::timeout(Duration::from_millis(500), self.native.send(command)).await {
            Ok(Ok(())) => {
                if let Some(stop) = self.capture.retired_stops.get_mut(command_id) {
                    stop.sent = true;
                }
                self.log(
                    "retired_audio_stop_sent",
                    &[("id", command_id), ("stream_id", &stream)],
                );
            }
            Ok(Err(_closed)) => {
                self.capture.retired_stops.remove(command_id);
                self.log("retired_audio_stop_helper_gone", &[("id", command_id)]);
            }
            Err(_elapsed) => {
                self.log("retired_audio_stop_send_owed", &[("id", command_id)]);
            }
        }
    }

    pub(super) async fn retry_retired_stream_stops(&mut self) {
        if !self.state.capabilities().native_helper() {
            let retired_call_starts = self.capture.retired_call_starts.len();
            let retired_stops = self.capture.retired_stops.len();
            if retired_call_starts > 0 || retired_stops > 0 {
                self.log(
                    "retired_capture_debts_helper_unavailable",
                    &[
                        ("retired_call_starts", &retired_call_starts.to_string()),
                        ("retired_stops", &retired_stops.to_string()),
                    ],
                );
                self.capture.retired_call_starts.clear();
                self.capture.retired_stops.clear();
            }
            return;
        }
        let owed = self
            .capture
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
    pub(super) async fn retire_pending_stream_stop(
        &mut self,
        stream: &str,
        start_failed: bool,
    ) -> Result<bool> {
        let ids: Vec<String> = self
            .capture
            .audio
            .stops
            .iter()
            .filter(|(_, stop)| stop.stream == stream)
            .map(|(id, _)| id.clone())
            .collect();
        for id in ids {
            let Some(stop) = self.capture.audio.stops.remove(&id) else {
                continue;
            };
            if !stop.answered {
                if self.capture.retired_stops.len() >= MAX_RETIRED_STOPS {
                    self.retired_stop_capacity_exhausted().await?;
                    return Ok(false);
                }
                self.capture.retired_stops.insert(
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

    pub(super) fn mark_overtaken_stop_answered(&mut self, command_id: &str) {
        if let Some(stop) = self.capture.audio.stops.get_mut(command_id) {
            stop.answered = true;
        }
    }

    /// The wait for the Mac's drain ends with this notice instead.
    pub(super) fn replace_drain_wait(&mut self, notice: Notice) {
        self.say(notice);
    }

    pub(super) fn call_stop_overtook_unanswered_start(&self, command_id: &str) -> bool {
        self.capture
            .audio
            .stops
            .get(command_id)
            .is_some_and(|stop| {
                stop.source == "mic_local"
                    && self.closing_call_owns_stream(&stop.stream)
                    && self
                        .capture
                        .retired_call_starts
                        .values()
                        .any(|stream| stream == &stop.stream)
            })
    }

    pub(super) async fn capture_started_if_ready(&mut self) -> Result<()> {
        if self.capture.pending_starts.is_empty()
            && self
                .state
                .durable
                .recording
                .as_ref()
                .is_some_and(|r| r.phase == CapturePhase::Starting)
        {
            let decision = decide(&self.state, Event::CaptureStarted, now())?;
            self.store_actor.snapshot(&decision.state).await?;
            self.state = decision.state;
        }
        Ok(())
    }

    pub(super) async fn audio_response(
        &mut self,
        command_id: &str,
        result: &Value,
    ) -> Result<bool> {
        let full_stop = self
            .capture
            .audio
            .full_stop
            .as_ref()
            .filter(|(id, _)| id == command_id)
            .map(|(_, kind)| *kind);
        if (full_stop.is_some() || self.capture.audio.stops.contains_key(command_id))
            && !decode_result_body::<DrainAcknowledgement>("audio_stop", result)?.drain_complete
        {
            bail!(
                "El helper no confirmó el drenaje completo; se conserva el audio y la parada pendiente"
            );
        }
        if let Some(kind) = full_stop {
            self.flush_stopped_audio().await?;
            // This receipt drained every stream already held by the helper,
            // including the selective stops in the bounded retired ledger.
            self.capture.retired_stops.clear();
            match kind {
                FullStop::Privacy => {
                    if self
                        .state
                        .durable
                        .recording
                        .as_ref()
                        .is_some_and(|recording| {
                            recording.phase == CapturePhase::Interrupted
                                && recording.ended_at_ms.is_some()
                                && recording.sealed_chunks == 0
                        })
                    {
                        self.say(NoticeCode::PrivacyStopConfirmed);
                    } else {
                        self.say(
                            Notice::new(NoticeCode::PrivacyStopConfirmed)
                                .with(NoticeKey::Fragments, true),
                        );
                    }
                }
                // The cut's cause stays the line the user reads; the confirmation
                // is added to it.
                FullStop::Cut => {
                    if self
                        .state
                        .durable
                        .recording
                        .as_ref()
                        .is_some_and(|recording| {
                            recording.phase == CapturePhase::Interrupted
                                && recording.ended_at_ms.is_some()
                                && recording.sealed_chunks == 0
                        })
                    {
                        self.say(NoticeCode::CaptureCutConfirmed);
                    } else {
                        self.say(
                            Notice::new(NoticeCode::CaptureCutConfirmed)
                                .with(NoticeKey::Fragments, true),
                        );
                    }
                }
            }
            self.store_actor.snapshot(&self.state).await?;
            return Ok(true);
        }
        let Some(pending) = self.capture.audio.stops.remove(command_id) else {
            return Ok(false);
        };
        if pending.source == "mic_local"
            && self.capture.mic_stream.as_deref() == Some(&pending.stream)
        {
            self.capture.mic_stream = None;
        }
        if pending.source == "remote_app"
            && self.capture.remote_stream.as_deref() == Some(&pending.stream)
        {
            self.capture.remote_stream = None;
        }
        let segments = self.capture.buffer.pause_source(pending.source)?;
        self.seal(segments)?;
        self.complete_audio_drain().await?;
        Ok(true)
    }

    /// Close a recording whose finish was deferred because a seal was still
    /// in flight, once that seal has landed.
    ///
    /// The last thing `complete_audio_drain` does for a finished
    /// meeting is ask `decide` for `CaptureFinalized`, which is what takes the
    /// recording out of `Finalizing`. It refuses to do that while a chunk is
    /// still being written — rightly, because a recording is not finalized
    /// while part of it is unwritten — and stores the intent to come back.
    ///
    /// Coming back is what went missing. Two paths observe a seal landing: the
    /// store actor's completion queue, drained by `collect_ready_seals` from
    /// `seal` and from the tick, and the work event `StoreWork::Sealed`. Only
    /// the second re-drove the drain, and only inside `if let Some(id)` — that
    /// is, only when the segment was *still* pending by the time the work event
    /// was handled. Under load the collector usually gets there first, the work
    /// event then finds nothing to apply, and the deferred finish is dropped on
    /// the floor. The recording sits in `Finalizing` until
    /// `FINALIZING_TIMEOUT_MS` interrupts it fifteen seconds later and tells
    /// the person the Mac never confirmed the close — when the Mac had
    /// confirmed it, and the daemon had recorded that it did.
    ///
    /// Measured at four runs in twenty under load before this existed.
    ///
    /// So both observers end here instead, and the tick asks the same question
    /// once a second, because the condition — a finish is waiting and no seal
    /// is left — is cheap to read and does not depend on which path noticed.
    pub(super) async fn resume_deferred_drain(&mut self) -> Result<()> {
        if self.capture.audio.finish_deferred()
            && !self.capture.audio.pending()
            && self.capture.pending_seals.is_empty()
        {
            self.complete_audio_drain().await?;
        }
        Ok(())
    }

    pub(super) async fn complete_audio_drain(&mut self) -> Result<()> {
        if self.capture.audio.pending() {
            return Ok(());
        }
        self.capture.audio.deadline = None;
        self.capture.audio.started_at = None;
        self.capture.audio.last_liveness = None;
        self.state.views.draining_since_ms = None;
        if let Some(end) = self.capture.audio.recording_end {
            let segments = self.capture.buffer.flush(end);
            if let Err(error) = self.seal(segments) {
                self.say_failure(NoticeCode::RecordingSealFailed, &error);
                return Err(error);
            }
            if !self.capture.pending_seals.is_empty() {
                return Ok(());
            }
            self.capture.audio.recording_end = None;
            if end
                && self
                    .state
                    .durable
                    .recording
                    .as_ref()
                    .is_some_and(|r| r.phase == CapturePhase::Finalizing)
            {
                let decision = decide(&self.state, Event::CaptureFinalized, now())?;
                self.store_actor.snapshot(&decision.state).await?;
                self.state = decision.state;
                self.queue_finalize().await?;
            }
        }
        if self.capture.audio.voice_end {
            // This drain is over whatever the dispatch says: a note that
            // could not be sent waits on the Mac, and nothing else waits for
            // it (`busy/audio_drain` once held every recording until a restart).
            self.capture.audio.voice_end = false;
            self.dispatch_voice_transcription().await?;
        }
        // Expiry can interrupt a recording while its stream still owes an
        // audio_stop receipt. Only now, after that receipt and the final seal,
        // can an empty capture release its recording identity.
        if let Some(interruption) = self.capture.interruption.clone() {
            Box::pin(self.close_empty_interrupted(interruption)).await?;
        }
        self.finish_call_drain().await?;
        if self.state.notice_code() == Some(NoticeCode::AudioDrainWaiting) {
            self.say(NoticeCode::AudioDrainConfirmed);
        }
        self.store_actor.snapshot(&self.state).await?;
        Ok(())
    }

    /// Stops every capture source in the helper and waits for its drain: the
    /// user's latching stop, or a cut that leaves the latch as it was.
    pub(super) async fn request_full_audio_stop(&mut self, kind: FullStop) -> Result<()> {
        self.capture.audio.full_stop_owed = true;
        self.mark_call_stopping();
        self.capture.audio.submit_voice = false;
        self.capture.audio.voice_end = false;
        self.capture.audio.recording_end = Some(true);
        self.capture.pending_starts.clear();
        if self.capture.audio.full_stop.is_some() {
            return Ok(());
        }
        if !self.state.capabilities().native_helper() {
            self.flush_stopped_audio().await?;
            return Ok(());
        }
        let name = kind.command();
        let command = native::command(name, json!({}))?;
        self.capture.audio.full_stop = Some((
            NativeCommandHeader::from_value(&command)?.id.to_owned(),
            kind,
        ));
        self.capture.audio.full_stop_send_owed = false;
        self.record_pending_native(&command);
        self.begin_audio_drain(false);
        // The caller already wrote why the stop happened, and that cause is the
        // only line telling a person what to do about it. Overwriting it made
        // every stop read identically — a changed audio device looked exactly
        // like a deliberate brake — so the wait is appended, never substituted.
        if self.state.durable.notice.is_none() {
            self.say(Notice::new(NoticeCode::FullStopRequested).with_token(NoticeKey::Code, name));
        }
        // Every helper send carries the 500 ms deadline. This bound
        // protects the actor from a full helper channel; it is the measured
        // native command deadline, not evidence that Orbit.app died and not a
        // deadline after which the microphone is known to be closed.
        match tokio::time::timeout(Duration::from_millis(500), self.native.send(command)).await {
            Ok(Ok(())) => Ok(()),
            Ok(Err(_closed)) => {
                self.state = decide(&self.state, Event::HelperStopUndelivered, now())?.state;
                bail!(
                    "El Mac no recibió la orden {name}: el canal con Orbit.app está cerrado. Orbit ya no guarda el audio de esa captura."
                )
            }
            Err(_elapsed) => {
                self.capture.audio.full_stop_send_owed = true;
                bail!(
                    "El Mac no recibió la orden {name} en 500 ms. Orbit ya no guarda el audio de esa captura; la orden se reintenta desde el mantenimiento hasta que el canal la acepte."
                )
            }
        }
    }

    /// Resend a whole-helper stop which timed out before it entered the helper's
    /// channel. Success means delivered to that channel, not microphone
    /// closed; `full_stop` remains until the matching drain response arrives.
    pub(super) async fn retry_full_stop_send(&mut self) -> Result<()> {
        if !self.capture.audio.full_stop_send_owed || !self.state.capabilities().native_helper() {
            return Ok(());
        }
        let Some((command_id, kind)) = self.capture.audio.full_stop.as_ref() else {
            self.capture.audio.full_stop_send_owed = false;
            return Ok(());
        };
        let command_id = command_id.clone();
        let name = kind.command();
        let command = native::command(name, json!({"id":command_id}))?;
        self.record_pending_native(&command);
        match tokio::time::timeout(Duration::from_millis(500), self.native.send(command)).await {
            Ok(Ok(())) => {
                self.capture.audio.full_stop_send_owed = false;
                // The drain window starts when the helper actually receives
                // this order, not when its full channel rejected it.
                self.capture.audio.started_at = None;
                self.begin_audio_drain(false);
                Ok(())
            }
            Ok(Err(_closed)) => {
                self.state = decide(&self.state, Event::HelperStopUndelivered, now())?.state;
                bail!("El Mac no recibió la orden {name}: el canal con Orbit.app está cerrado")
            }
            Err(_elapsed) => bail!(
                "El Mac no recibió la orden {name} en 500 ms; el canal sigue presente y la parada continúa pendiente"
            ),
        }
    }

    pub(super) async fn flush_stopped_audio(&mut self) -> Result<()> {
        // All native tail frames must arrive before this local durability
        // boundary. On failure voice stays owned here or in pending_seals.
        self.capture.audio.submit_voice = false;
        self.dispatch_voice_transcription().await?;
        self.capture.mic_stream = None;
        self.capture.remote_stream = None;
        self.capture.pending_starts.clear();
        self.capture.audio = AudioLifecycle::default();
        self.state.views.draining_since_ms = None;
        let segments = self.capture.buffer.flush(true);
        self.seal(segments)?;
        if let Some(interruption) = self.capture.interruption.clone() {
            Box::pin(self.close_empty_interrupted(interruption)).await?;
        }
        self.finish_call_drain().await?;
        Ok(())
    }

    pub(super) fn begin_call_voice(&mut self, preroll: &[(Value, i64)]) -> Result<()> {
        self.capture.buffer.begin_voice()?;
        self.capture.audio.submit_voice = true;
        for (frame, time) in preroll {
            self.capture.buffer.ingest_voice_frame(frame, *time)?;
        }
        Ok(())
    }

    pub(super) async fn finish_call_voice(&mut self, submit: bool) -> Result<()> {
        if submit {
            self.dispatch_voice_transcription().await
        } else {
            self.capture.buffer.voice = None;
            Ok(())
        }
    }

    pub(super) fn audio_is_draining(&self) -> bool {
        self.capture.audio.pending()
    }

    /// Whether a whole-helper stop still owes the local flush of the audio it
    /// stopped: always under the user's stop, and after a cut or a closing until
    /// the flush succeeds.
    pub(super) fn stopped_audio_owed(&self) -> bool {
        self.state.privacy_stopped() || self.capture.audio.full_stop_owed
    }

    pub(super) fn full_stop_send_owed(&self) -> bool {
        self.capture.audio.full_stop_send_owed
    }

    pub(super) fn audio_timed_out(&self) -> bool {
        self.capture
            .audio
            .deadline
            .is_some_and(|deadline| Instant::now() >= deadline)
    }

    fn live_call_reached_drain_limit(&self) -> bool {
        let now = Instant::now();
        let hard_limit_reached = self
            .capture
            .audio
            .started_at
            .and_then(|started| started.checked_add(AUDIO_DRAIN_LIMIT))
            .is_some_and(|limit| now >= limit);
        let helper_is_live = self
            .capture
            .audio
            .last_liveness
            .and_then(|seen| seen.checked_add(AUDIO_SILENCE_TIMEOUT))
            .is_some_and(|silent_at| now < silent_at);
        let meeting_is_draining = self
            .state
            .durable
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
        hard_limit_reached
            && helper_is_live
            && self.capture.call_runtime.is_some()
            && !meeting_is_draining
    }

    /// The Mac did not confirm an audio stop in its window, or its helper was
    /// lost while an unsent stop had no deadline left. What is live is cut
    /// with its cause; the audio that arrived is kept and the missing tail is
    /// declared as a gap. The user's stop stays in force.
    pub(super) async fn handle_audio_timeout(&mut self) {
        let helper_lost_with_unsent_stop =
            self.full_stop_send_owed() && !self.state.capabilities().native_helper();
        if !self.audio_timed_out() && !helper_lost_with_unsent_stop {
            return;
        }
        if helper_lost_with_unsent_stop {
            // The earlier timeout deliberately removed the drain deadline
            // while the channel existed. Losing that channel later makes the
            // retry impossible, even though no deadline can fire again.
            let name = match self.capture.audio.full_stop.as_ref() {
                Some((_, kind)) => kind.command(),
                None => "orden de parada",
            };
            let cause = format!(
                "El helper de Orbit.app se desconectó con {name} sin entregar tras 500 ms. Orbit deja de esperar al helper y envía el audio recibido a sellado local; el final puede faltar."
            );
            self.contain(
                orbit_core::privacy_stop::StopCause::AudioStopUnconfirmed,
                &cause,
            )
            .await;
            if self.abandon_audio_drain().await.is_ok() {
                // `halt` saw the still-owned audio before it could be sealed
                // and said a storage warning. Once the local handoff
                // succeeds, that warning and the old retry promise are false.
                self.say_failure(NoticeCode::AudioStopUnconfirmed, &cause);
            }
            let _ = self.store_actor.snapshot(&self.state).await;
            return;
        }
        if self.full_stop_send_owed() && self.state.capabilities().native_helper() {
            // The order never entered the helper. A drain deadline cannot
            // prove its microphone closed or discard the retry debt while the
            // channel exists. Once the helper is gone, retry is impossible:
            // abandon the drain below and seal the audio already received.
            // The existing 500 ms send budget governs each maintenance
            // attempt; the drain window restarts only after delivery.
            self.capture.audio.deadline = None;
            self.say(NoticeCode::StopRetrying);
            let _ = self.store_actor.snapshot(&self.state).await;
            return;
        }
        if self.live_call_reached_drain_limit() && self.abandon_audio_drain().await.is_ok() {
            self.say(NoticeCode::CallDrainUnconfirmed);
            let _ = self.store_actor.snapshot(&self.state).await;
            return;
        }
        let waited_ms = self
            .capture
            .audio
            .started_at
            .map_or(0, |started| started.elapsed().as_millis());
        let cause = format!(
            "El Mac no confirmó la parada de audio en {waited_ms} ms (plazo: {} ms sin respuesta del Mac, {} ms en total). Se conserva el audio recibido; el final puede faltar.",
            AUDIO_SILENCE_TIMEOUT.as_millis(),
            AUDIO_DRAIN_LIMIT.as_millis()
        );
        self.contain(
            orbit_core::privacy_stop::StopCause::AudioStopUnconfirmed,
            &cause,
        )
        .await;
        let _ = self.abandon_audio_drain().await;
        let _ = self.store_actor.snapshot(&self.state).await;
    }

    pub(super) fn accepts_audio(&self, frame: &Value) -> bool {
        if self.state.privacy_stopped() && !self.audio_is_draining() {
            return false;
        }
        let Ok(audio) = NativeAudioIdentityRef::from_value(frame) else {
            return false;
        };
        match audio.source {
            "mic_local" => self.capture.mic_stream.as_deref() == Some(audio.stream_id),
            "remote_app" => self.capture.remote_stream.as_deref() == Some(audio.stream_id),
            _ => false,
        }
    }
}
