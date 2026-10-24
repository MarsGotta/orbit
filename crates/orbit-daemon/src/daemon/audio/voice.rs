//! The voice note's audio: it consumes the microphone, the one a meeting
//! holds or its own, and never stops, restarts or reroutes a meeting's. When
//! it ends, fails or is cancelled, only the note's own state changes.
use super::super::Runtime;
use super::microphone::Holder;
use anyhow::Result;
use orbit_core::authority::AuthorityChange;
use orbit_core::notice::{Notice, NoticeCode};
use orbit_core::voice::StopReason;

/// The voice note's own state.
#[derive(Default)]
pub(in crate::daemon) struct VoiceAudio {
    /// The note may go to transcription when its audio is in. Esc, a stop or
    /// a cancellation clears it, also while waiting for the helper's drain.
    pub(in crate::daemon) submit: bool,
    /// The note ended and its transcription waits for the drain.
    pub(in crate::daemon) end: bool,
}

/// Identity links survive provider failure; no transcription is a replacement
/// for the original microphone bytes stored under this source.
pub(in crate::daemon) struct VoiceSource {
    pub recording_id: String,
    pub chunk_id: String,
    pub session_id: Option<String>,
    pub message_id: String,
    pub duration_ms: u64,
}

#[derive(Clone, Copy)]
pub(in crate::daemon) enum VoiceCancellationCause {
    Authority(AuthorityChange),
    AutomaticCallStop(StopReason),
    CallTurnReplaced,
    /// The user spoke again before the turn reached Deimos; their voice goes
    /// on in the next turn.
    CallTurnJoined,
    HelperCut,
    CaptureCut,
}

impl VoiceCancellationCause {
    /// The failure code the cancelled note keeps.
    fn code(self) -> &'static str {
        match self {
            Self::Authority(change) => match change {
                AuthorityChange::UserStop => "voice_transcription_privacy_stopped",
                // wire/stored name kept for compatibility with 0.9.0
                AuthorityChange::Interrupt => "voice_transcription_interrupted_by_mars",
                AuthorityChange::Closing => "voice_transcription_orbit_closing",
                AuthorityChange::Revoked
                | AuthorityChange::Rearm
                | AuthorityChange::RearmWithoutStop
                | AuthorityChange::HubLink
                | AuthorityChange::HubTransportReady
                | AuthorityChange::HubLinkOffline
                | AuthorityChange::HubDisconnected
                | AuthorityChange::Conversation
                | AuthorityChange::Device => "voice_transcription_generation_changed",
            },
            Self::AutomaticCallStop(
                StopReason::Unauthorized
                | StopReason::GenerationChanged
                | StopReason::ProcessingLost
                | StopReason::Discontinuity
                | StopReason::Expired
                | StopReason::Idle
                | StopReason::NoAudio,
            ) => "voice_transcription_call_stopped_automatically",
            Self::CallTurnReplaced => "voice_transcription_call_turn_replaced",
            Self::CallTurnJoined => "voice_transcription_joined_next_turn",
            Self::HelperCut => "voice_transcription_helper_cut",
            Self::CaptureCut => "voice_transcription_capture_cut",
        }
    }
}

impl Runtime {
    pub(in crate::daemon) async fn start_voice_audio(&mut self) -> Result<()> {
        self.capture.begin_voice()?;
        self.audio.lifecycle.voice.submit = true;
        self.take_microphone(None).await
    }

    pub(in crate::daemon) async fn finish_voice_audio(&mut self) -> Result<()> {
        self.audio.lifecycle.voice.end = true;
        self.release_microphone(Holder::VoiceNote).await?;
        self.complete_audio_drain().await
    }

    /// Esc: the note finishes as any other and is kept on this Mac like a
    /// cancelled turn, but it is never transcribed or sent.
    pub(in crate::daemon) async fn cancel_voice_audio(&mut self) -> Result<()> {
        self.audio.lifecycle.voice.submit = false;
        self.finish_voice_audio().await
    }

    /// After the drain: the note that ended goes to transcription.
    pub(super) async fn dispatch_drained_voice(&mut self) -> Result<()> {
        if self.audio.lifecycle.voice.end {
            // This drain is over whatever the dispatch says: a note that
            // could not be sent waits on the Mac, and nothing else waits for
            // it (`busy/audio_drain` once held every recording until a restart).
            self.audio.lifecycle.voice.end = false;
            self.dispatch_voice_transcription().await?;
        }
        Ok(())
    }

    pub(in crate::daemon) fn discard_refused_voice(&mut self) {
        self.capture.discard_voice();
        self.audio.lifecycle.voice.submit = false;
        self.audio.lifecycle.voice.end = false;
    }

    pub(in crate::daemon) fn retire_voice_transcription(
        &mut self,
        change: AuthorityChange,
    ) -> bool {
        self.mark_voice_transcription_cancelled(VoiceCancellationCause::Authority(change))
    }

    fn mark_voice_transcription_cancelled(&mut self, cause: VoiceCancellationCause) -> bool {
        let op = self.assistant.voice_turn_op();
        let pending = self.assistant.cancel_voice_turn();
        if let Some(operation_id) = pending.as_ref() {
            // A turn spoken over Deimos's answer reached no one and was never
            // stored: the change that stops its transcription retires it.
            let _ = self.audio.take_over_answer_turn(operation_id);
            if let Some(voice) = self
                .state
                .durable
                .assistant
                .turn
                .as_mut()
                .and_then(|turn| turn.untranscribed_voice_mut(operation_id))
            {
                voice.failure = Some(orbit_core::VoiceTranscriptionFailure::local(cause.code()));
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
        self.audio.lifecycle.voice.submit = false;
        pending.is_some()
    }

    pub(in crate::daemon) async fn cancel_voice_transcription(
        &mut self,
        cause: VoiceCancellationCause,
    ) -> Result<()> {
        let pending = self.mark_voice_transcription_cancelled(cause);
        if pending {
            self.store_actor.snapshot(&self.state).await?;
            self.persisted.durable = crate::daemon::DurableSnapshot::from(&self.state);
        }
        Ok(())
    }
}
