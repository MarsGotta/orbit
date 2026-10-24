//! The app's orders to the recording: the voice note, the call with the
//! assistant, and a meeting started by hand or from «Grabar» on an event.
use super::*;

impl Runtime {
    /// Routes an order to the recording: the voice note, the call with the
    /// assistant, the meeting and the microphone.
    pub(super) async fn recording_order(&mut self, command: Command) -> Result<()> {
        match command {
            Command::VoiceStart => self.apply(Event::VoiceStart).await?,
            Command::VoiceEnd => self.apply(Event::VoiceEnd).await?,
            Command::VoiceCancel => self.apply(Event::VoiceCancel).await?,
            Command::VoiceRetry { message_id } => {
                self.retry_voice_transcription(message_id).await?
            }
            Command::VoiceDiscard { message_id } => self.discard_voice_note(message_id).await?,
            Command::CallStart { duration_minutes } => {
                self.call_start_command(duration_minutes).await?
            }
            Command::CallStop => self.apply(Event::CallStop).await?,
            Command::MeetingStart {
                title,
                audio,
                duration_minutes: _,
                event,
            } => self.meeting_start_command(title, audio, event).await?,
            Command::MeetingPause => self.apply(Event::PauseMeeting).await?,
            Command::MeetingResume => self.apply(Event::ResumeMeeting).await?,
            Command::MeetingFinish => self.apply(Event::FinishMeeting).await?,
            Command::MeetingDiscard { recording_id } => self.discard_meeting(recording_id).await?,
            Command::MicrophoneMute { muted } => self.mute_microphone(muted).await?,
            Command::AudioSourcesRefresh => self.refresh_audio_sources()?,
            off_loop_orders!()
            | device_orders!()
            | assistant_orders!()
            | note_orders!()
            | agenda_orders!() => return Err(super::misrouted()),
        }
        Ok(())
    }

    /// «Grabar» on an event during a call records the call's application
    /// when the helper says which one holds the microphone; otherwise the
    /// system, as before.
    pub(super) async fn meeting_start_command(
        &mut self,
        title: String,
        audio: orbit_core::protocol::MeetingAudio,
        event: Option<orbit_core::call_detection::LinkedEvent>,
    ) -> Result<()> {
        // «Grabar» on an event during a call records the
        // call's application when the helper says which one holds the
        // microphone; otherwise the system, as before.
        let audio = match (&event, audio) {
            (Some(_), orbit_core::protocol::MeetingAudio::System) => {
                self.calls.detector.holding_call().map_or(audio, |app| {
                    orbit_core::protocol::MeetingAudio::Application {
                        application_pid: app.application_pid,
                    }
                })
            }
            _ => audio,
        };
        self.start_meeting(title, audio, event, "explicit").await?;
        Ok(())
    }

    /// A call the app asked for, for the minutes it chose.
    pub(super) async fn call_start_command(&mut self, duration_minutes: u16) -> Result<()> {
        self.apply(Event::CallStart {
            id: id(),
            duration_ms: i64::from(duration_minutes) * 60_000,
            origin: orbit_core::CallOrigin::App,
        })
        .await
    }
}
