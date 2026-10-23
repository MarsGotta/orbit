//! Who holds the microphone: the one fact the lifecycles share.
//!
//! One stream serves whoever listens. The meeting owns it while it records;
//! a voice note dictated over the meeting consumes the same stream and never
//! changes its holder; a meeting that starts while a note runs takes it over;
//! when the meeting pauses or ends under a note, the note keeps it. A call
//! opens its own stream with voice processing and shares it with nobody.
//!
//! The helper is told the holder (`audio_purpose`) because a change of audio
//! device depends on it: a meeting's microphone moves to the new device and
//! only a note dictated over it ends; a lone note's stops. A note begun before
//! a microphone-only meeting once left the helper believing the note alone
//! held it, and AirPods connecting stopped the whole meeting.
use super::super::{Runtime, id};
use anyhow::Result;
use orbit_core::protocol::MeetingAudio;
use orbit_core::wire::native::NativeMicrophonePurpose;
use serde_json::json;

/// What the helper was last told about the open microphone stream.
#[derive(Default)]
pub(super) struct MicrophoneHolder {
    told: Option<NativeMicrophonePurpose>,
}

/// A lifecycle that may hold the microphone stream with another.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Holder {
    Meeting,
    VoiceNote,
}

impl Runtime {
    /// Whether `holder` consumes the microphone now. A call's microphone is
    /// the call's alone.
    fn holds_microphone(&self, holder: Holder) -> bool {
        self.capture.call_runtime.is_none()
            && match holder {
                Holder::Meeting => self.recording_active(),
                Holder::VoiceNote => self.state.durable.voice.voice_active,
            }
    }

    /// The open microphone's holder as the helper must know it.
    fn microphone_purpose(&self) -> Option<NativeMicrophonePurpose> {
        self.capture.mic_stream.as_ref()?;
        if self.holds_microphone(Holder::Meeting) {
            Some(NativeMicrophonePurpose::Meeting)
        } else if self.holds_microphone(Holder::VoiceNote) {
            Some(NativeMicrophonePurpose::VoiceNote)
        } else {
            None
        }
    }

    /// The microphone for a meeting (`audio`) or a voice note (`None`):
    /// opened when nobody holds it, otherwise shared, and the helper told who
    /// holds it now.
    pub(super) async fn take_microphone(&mut self, audio: Option<MeetingAudio>) -> Result<()> {
        if self.capture.mic_stream.is_none() {
            let stream = id();
            self.start_track("mic_local", &stream, audio).await?;
            self.capture.mic_stream = Some(stream);
            self.capture.audio.microphone.told = Some(if audio.is_some() {
                NativeMicrophonePurpose::Meeting
            } else {
                NativeMicrophonePurpose::VoiceNote
            });
        }
        self.tell_microphone_holder().await;
        Ok(())
    }

    /// `leaving` is done with the microphone. Returns whether the other
    /// lifecycle still holds it (the helper is then told so); otherwise the
    /// stream, if open, stops.
    pub(super) async fn release_microphone(&mut self, leaving: Holder) -> Result<bool> {
        let other = match leaving {
            Holder::Meeting => Holder::VoiceNote,
            Holder::VoiceNote => Holder::Meeting,
        };
        if self.holds_microphone(other) {
            self.tell_microphone_holder().await;
            return Ok(true);
        }
        if let Some(stream) = self.capture.mic_stream.clone() {
            self.request_stream_stop("mic_local", stream).await?;
        }
        Ok(false)
    }

    /// Tells the helper who holds the microphone when that changed. An order
    /// that does not leave never stops what asked for it (a start, a pause):
    /// it is logged, and the next change of holder tells it again.
    async fn tell_microphone_holder(&mut self) {
        let Some(purpose) = self.microphone_purpose() else {
            return;
        };
        if self.capture.audio.microphone.told == Some(purpose) {
            return;
        }
        let Some(stream) = self.capture.mic_stream.clone() else {
            return;
        };
        let told = self
            .native_send(
                "audio_purpose",
                json!({"stream_id": stream, "purpose": purpose}),
            )
            .await;
        match told {
            Ok(()) => self.capture.audio.microphone.told = Some(purpose),
            Err(error) => self.log(
                "audio_purpose_not_sent",
                &[
                    ("purpose", purpose_label(purpose)),
                    ("error", &error.to_string()),
                ],
            ),
        }
    }
}

fn purpose_label(purpose: NativeMicrophonePurpose) -> &'static str {
    match purpose {
        NativeMicrophonePurpose::Meeting => "meeting",
        NativeMicrophonePurpose::VoiceNote => "voice_note",
    }
}
