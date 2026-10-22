//! What a gap the helper declares does to its one track, beyond being
//! counted. The helper stops or restarts one track without cutting the
//! other, and says so with a `gap`; the recording goes on.
use super::Runtime;
use anyhow::Result;
use orbit_core::Event;
use orbit_core::notice::{Notice, NoticeCode, NoticeKey};

/// One track stopped and is not coming back; the other goes on.
pub(super) const TRACK_STOPPED: &str = "track_stopped";

impl Runtime {
    pub(super) async fn track_gap(&mut self, source: &str, reason: Option<&str>) -> Result<()> {
        match (source, reason) {
            // `system_audio_restarted` (the system's audio stream stopped and
            // the helper started it again under the same stream id): the
            // producer's sequence starts over.
            // Every declared gap already sealed the tail and restarted the
            // source's sequence (`on_native_gap`); nothing more is owed.
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
