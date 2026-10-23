//! The app's orders to the recording: a meeting started by hand or from
//! «Grabar» on an event.
use super::*;

impl Runtime {
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
