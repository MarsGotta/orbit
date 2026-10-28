//! The microphone and the system's audio, as the three lifecycles that listen
//! share them: the meeting recording (`meeting`), the voice note (`voice`)
//! and the call with the assistant (`call`). Each keeps its own state and
//! makes its own decisions; what they share is who holds the microphone
//! (`microphone`) and how a stream is let go (`drain`): a lifecycle command
//! completes only after the native reader drains the preceding frames and
//! delivers the matching response in the same stream.
//!
//! The meeting owns the microphone while it records. A voice note dictated
//! over it consumes the same stream and never stops it, restarts it or
//! changes its route; when the note ends, fails or is cancelled, the meeting's
//! capture carries on untouched. A call opens its own stream with voice
//! processing and admits no meeting or note beside it.
use super::Runtime;
use orbit_core::Event;
use orbit_core::wire::native::NativeAudioIdentityRef;
use serde_json::Value;
use std::collections::BTreeMap;
use tokio::time::Instant;

mod call;
mod drain;
mod meeting;
mod microphone;
mod mute;
mod timeout;
mod voice;

pub(in crate::daemon) use call::{CallStopVoice, RetiredStreamStop};
pub(in crate::daemon) use drain::FullStop;
pub(in crate::daemon) use voice::{VoiceCancellationCause, VoiceSource};

#[cfg(test)]
#[path = "audio/microphone_tests.rs"]
mod microphone_tests;
#[cfg(test)]
#[path = "audio/mute_tests.rs"]
mod mute_tests;
#[cfg(test)]
#[path = "audio/sharing_tests.rs"]
mod sharing_tests;
#[cfg(test)]
#[path = "audio_tests.rs"]
pub(super) mod tests;

/// The capture's audio as the lifecycles share it: each one's own state, who
/// the helper was told holds the microphone, and the stops still waiting for
/// the helper's drain.
#[derive(Default)]
pub(in crate::daemon) struct AudioLifecycle {
    /// The meeting recording's.
    pub(in crate::daemon) meeting: meeting::RecordingAudio,
    /// The voice note's.
    pub(in crate::daemon) voice: voice::VoiceAudio,
    /// Who the helper was last told holds the microphone.
    microphone: microphone::MicrophoneHolder,
    /// «Silenciar» beyond the recording's spans: the call's, and the orders
    /// the helper has not answered.
    mute: mute::MicrophoneMute,
    /// Selective stops waiting for their drain, by command id.
    pub(in crate::daemon) stops: BTreeMap<String, drain::PendingStop>,
    /// The whole-helper stop still waiting for its drain, by command id.
    pub(in crate::daemon) full_stop: Option<(String, FullStop)>,
    /// A whole-helper stop timed out before entering the bounded channel.
    /// A timeout is backpressure, not absence: while the helper exists,
    /// maintenance resends this exact stop ID and then waits for its drain ACK.
    full_stop_send_owed: bool,
    /// A whole-helper stop (hers or a cut) whose local flush has not
    /// succeeded yet: the maintenance retries it until the audio held in
    /// memory is durable. Cleared with the lifecycle when the flush lands.
    full_stop_owed: bool,
    pub(in crate::daemon) deadline: Option<Instant>,
    pub(in crate::daemon) started_at: Option<Instant>,
    last_liveness: Option<Instant>,
}

/// The audio's handle: the three lifecycles, the streams this capture
/// opened, and the starts and stops the helper has yet to answer.
#[derive(Default)]
pub(in crate::daemon) struct AudioHandle {
    /// Unread helper facts at the last loop tick: silence cannot be decided
    /// before they are consumed (they may include fresh voice).
    pub(in crate::daemon) queued_native_events: usize,
    pub(in crate::daemon) lifecycle: AudioLifecycle,
    /// The call with the assistant while it runs.
    pub(in crate::daemon) call: Option<super::call::CallRuntime>,
    pub(in crate::daemon) mic_stream: Option<String>,
    pub(in crate::daemon) remote_stream: Option<String>,
    /// `audio_start` orders the helper has not answered yet.
    pub(in crate::daemon) pending_starts: Vec<String>,
    /// A retired call's start, by command id, with the stream it asked for.
    pub(in crate::daemon) retired_call_starts: BTreeMap<String, String>,
    /// Stops of retired streams, by command id.
    pub(in crate::daemon) retired_stops: BTreeMap<String, RetiredStreamStop>,
    /// The meeting's audio sources the helper was asked to list.
    pub(in crate::daemon) inventory: super::meeting_runtime::MeetingRuntime,
}

impl AudioHandle {
    /// The microphone stream open now, if one is.
    pub(in crate::daemon) fn microphone_stream(&self) -> Option<&str> {
        self.mic_stream.as_deref()
    }
}

impl AudioLifecycle {
    fn pending(&self) -> bool {
        !self.stops.is_empty() || self.full_stop.is_some()
    }

    pub(in crate::daemon) fn response_command(&self, command_id: &str) -> Option<&'static str> {
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
    /// What may start while the audio is busy. A drain in progress or a
    /// finish still owed holds every start; a note admits no second note; a
    /// call admits nothing else on the microphone.
    pub(in crate::daemon) fn validate_audio_transition(&self, event: &Event) -> anyhow::Result<()> {
        let busy = |operation| -> anyhow::Result<()> {
            Err(orbit_core::refusal::Refusal::Busy {
                operation,
                retry_after_ms: None,
            }
            .into())
        };
        if matches!(
            event,
            Event::StartMeeting { .. }
                | Event::CallStart { .. }
                | Event::ResumeMeeting
                | Event::VoiceStart
                | Event::LocalRearm
        ) && (self.audio.lifecycle.pending()
            || self.audio.lifecycle.meeting.finish_owed()
            || self.audio.lifecycle.voice.end)
        {
            return busy(orbit_core::refusal::Operation::AudioDrain);
        }
        if matches!(event, Event::VoiceStart) && self.meeting_microphone_muted() {
            anyhow::bail!("The microphone is muted: unmute it to dictate a voice note");
        }
        if matches!(event, Event::VoiceStart)
            && (self.assistant.voice_turn_running() || self.capture.voice_capturing())
        {
            return busy(orbit_core::refusal::Operation::Interaction);
        }
        if matches!(event, Event::CallStart { .. })
            && (self.audio.call.is_some()
                || self.audio.mic_stream.is_some()
                || self.audio.remote_stream.is_some()
                || self.capture.voice_capturing()
                || self.assistant.voice_turn_running())
        {
            return busy(orbit_core::refusal::Operation::AudioSession);
        }
        Ok(())
    }

    /// A frame belongs to a stream this capture opened.
    pub(in crate::daemon) fn accepts_audio(&self, frame: &Value) -> bool {
        if self.state.privacy_stopped() && !self.audio.audio_is_draining() {
            return false;
        }
        let Ok(audio) = NativeAudioIdentityRef::from_value(frame) else {
            return false;
        };
        match audio.source {
            "mic_local" => self.audio.mic_stream.as_deref() == Some(audio.stream_id),
            "remote_app" => self.audio.remote_stream.as_deref() == Some(audio.stream_id),
            _ => false,
        }
    }
}
