//! The shortcut keys: push-to-talk, the user's stop, the call and the
//! meeting.
use super::super::*;
use orbit_core::notice::NoticeCode;
use serde::Deserialize;

#[derive(Deserialize)]
struct NativeHotkeyFields<'a> {
    action: &'a str,
    phase: &'a str,
}

impl Runtime {
    pub(super) async fn on_native_hotkey(
        &mut self,
        event: &Value,
        now_ms: i64,
        utc_offset_ms: Option<i64>,
    ) -> Result<()> {
        let hotkey = NativeHotkeyFields::deserialize(event)?;
        let pressed = hotkey.phase == "pressed";
        let result: Result<()> = async {
            match hotkey.action {
                "ptt" => {
                    self.apply(if pressed {
                        Event::VoiceStart
                    } else {
                        Event::VoiceEnd
                    })
                    .await?
                }
                "privacy_stop" if pressed => {
                    self.privacy_stop(orbit_core::privacy_stop::UserStop::Hotkey)
                        .await
                }
                "call_toggle" if pressed => {
                    self.apply(if self.state.durable.voice.call.is_some() {
                        Event::CallStop
                    } else {
                        Event::CallStart {
                            id: id(),
                            duration_ms: i64::from(orbit_core::voice::HOTKEY_CALL_MINUTES) * 60_000,
                            origin: orbit_core::CallOrigin::Hotkey,
                        }
                    })
                    .await?
                }
                "meeting_toggle" if pressed => {
                    self.toggle_meeting_from_keyboard(event, now_ms, utc_offset_ms)
                        .await?
                }
                _ => {}
            }
            Ok(())
        }
        .await;
        if let Err(error) = result {
            if error
                .downcast_ref::<orbit_core::refusal::Refusal>()
                .is_none()
            {
                return Err(error);
            }
            // Expected interaction refusal is not audio corruption.
            // Persistence/native failures still propagate fail-closed.
            let notice = super::refusal_notice(&error, NoticeCode::ShortcutRefused);
            self.say_failure(notice, &error);
        }
        Ok(())
    }

    /// The meeting shortcut ends the recording in progress, or starts one
    /// with the application the Mac had running.
    async fn toggle_meeting_from_keyboard(
        &mut self,
        event: &Value,
        now_ms: i64,
        utc_offset_ms: Option<i64>,
    ) -> Result<()> {
        if self.recording_active() {
            self.command(Command::MeetingFinish).await
        } else {
            // Started from the keyboard this used to pass no
            // application at all, so the recording held one
            // voice and nobody learned that until reading the
            // transcript. The Mac now sends what is running
            // and the choice is made here.
            let choice = orbit_core::meeting_audio::choose(&meeting_applications(event));
            let audio = match &choice {
                orbit_core::meeting_audio::AudioChoice::Application {
                    application_pid, ..
                } => orbit_core::protocol::MeetingAudio::Application {
                    application_pid: *application_pid,
                },
                orbit_core::meeting_audio::AudioChoice::NoCandidate
                | orbit_core::meeting_audio::AudioChoice::Ambiguous { .. } => {
                    orbit_core::protocol::MeetingAudio::MicrophoneOnly
                }
            };
            // Said before the meeting rather than after it:
            // a recording with a single voice is only worth
            // knowing about while it can still be fixed.
            self.say(orbit_core::meeting_audio::notice(&choice));
            self.command(Command::MeetingStart {
                title: orbit_core::protocol::recording_title(String::new(), now_ms, utc_offset_ms),
                audio,
                duration_minutes: None,
                event: None,
            })
            .await
        }
    }
}
