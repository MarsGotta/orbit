//! What a gap the helper declares does to its one track, beyond being
//! counted. The helper stops or restarts one track without cutting the
//! other, and says so with a `gap`; the recording goes on.
use super::Runtime;
use anyhow::Result;
use orbit_core::Event;
use orbit_core::notice::{Notice, NoticeCode, NoticeKey};

/// The system's audio stream stopped and the helper started it again under
/// the same stream id: the new producer's sequence starts over.
pub(super) const SYSTEM_AUDIO_RESTARTED: &str = "system_audio_restarted";
/// One track stopped and is not coming back; the other goes on.
pub(super) const TRACK_STOPPED: &str = "track_stopped";

impl Runtime {
    pub(super) async fn track_gap(&mut self, source: &str, reason: Option<&str>) -> Result<()> {
        match (source, reason) {
            ("remote_app", Some(SYSTEM_AUDIO_RESTARTED)) => {
                let segments = self.capture.buffer.restart_source(source)?;
                self.seal(segments)?;
            }
            // The helper moved a meeting's microphone to the new device under
            // a voice note: the note, which would join two devices, is
            // cancelled and kept unsent; the meeting goes on.
            ("mic_local", Some("audio_device_changed")) if self.state.durable.voice_active => {
                self.apply(Event::VoiceCancel).await?;
                self.say(NoticeCode::VoiceNoteCancelledByAudioRoute);
            }
            (_, Some(TRACK_STOPPED)) => {
                // A call's application that quits ends the call's recording,
                // as the whole-capture cut it used to be did; that end is
                // what is said then.
                if source == "remote_app" {
                    self.call_capture_cut("system_audio_stream_stopped").await;
                }
                if self.recording_active() {
                    self.say(
                        Notice::new(NoticeCode::TrackStopped).with_token(NoticeKey::Origin, source),
                    );
                }
            }
            _ => {}
        }
        Ok(())
    }
}
