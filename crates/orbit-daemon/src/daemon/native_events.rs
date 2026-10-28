//! Native helper events, routed to the domain each one belongs to: the
//! helper's own lifecycle (`lifecycle`), the capture's frames (`frames`),
//! the shortcut keys (`hotkey`), the answers to orders (`response`, with the
//! audio's retired orders in `retired` and the refusals in `refused`), the
//! voice, the calendar and the speech.
use super::*;
use orbit_core::wire::native::NativeEventHeader;

mod frames;
mod hotkey;
mod lifecycle;
mod refused;
mod response;
mod retired;

impl Runtime {
    pub(super) fn native_event(
        &mut self,
        mut event: Value,
    ) -> impl std::future::Future<Output = Result<()>> + '_ {
        Box::pin(async move {
            self.audio.observe_audio_helper_liveness(&event);
            let host_update = self.helper.observe_host(&event);
            self.observe_calendar_native(&event, host_update);
            let event_name = NativeEventHeader::from_value(&event)?.event.to_owned();
            let preferences_changed = self.observe_speech_preferences(&event)?;
            match event_name.as_str() {
                "ready" => self.on_native_ready(&event).await,
                "response" => self.on_native_response(&event).await,
                "state" => self.on_native_state(&event).await,
                // The helper stopped: the user's brake at the helper latches
                // as theirs; a latch the helper holds that the daemon did not
                // ask for now is adopted so both say the same, attributed to
                // nobody; any other stop cut the capture, and only that.
                // `daemon_requested` is the echo of the daemon's own order:
                // with the daemon not stopped it is not an order of the
                // user's the daemon can vouch for, so it is never theirs.
                "privacy_stop" | "capture_stopped" => self.on_native_capture_stopped(&event).await,
                "local_rearm" => self.on_native_local_rearm().await,
                "helper_disconnected" => self.on_native_helper_disconnected(&event).await,
                "hotkey" => {
                    let clock = chrono::Local::now();
                    self.on_native_hotkey(
                        &event,
                        clock.timestamp_millis(),
                        Some(i64::from(clock.offset().local_minus_utc()) * 1_000),
                    )
                    .await
                }
                "audio" => self.on_native_audio(&mut event).await,
                "gap" => self.on_native_gap(&event).await,
                "error" => self.on_native_error(&event).await,
                "microphone_activity" => self.on_microphone_activity(&event).await,
                "assistant_question" => self.on_assistant_question(&event).await,
                "voice_trigger" => self.on_voice_trigger(&event),
                "notice" => self.on_native_notice(&event),
                "playback_progress" => self.on_playback_progress(event),
                _ => Ok(()),
            }?;
            if preferences_changed || event_name == "ready" {
                self.sync_wake_phrase().await?;
            }
            Ok(())
        })
    }
}

#[cfg(test)]
mod result_fixture_tests {
    use super::*;
    use orbit_core::wire::native::decode_result_body;
    use retired::{RetiredAudioStartResult, RetiredAudioStopResult};

    #[tokio::test]
    async fn meeting_shortcut_uses_the_local_time_or_an_unknown_zone_title() -> Result<()> {
        let mut titles = Vec::new();
        for offset in [Some(7_200_000), None] {
            let directory = tempfile::tempdir()?;
            let (mut runtime, mut native) = super::super::audio::tests::fixture(directory.path())?;
            runtime
                .on_native_hotkey(
                    &json!({"action":"meeting_toggle","phase":"pressed"}),
                    1_800_000,
                    offset,
                )
                .await?;
            titles.push(
                runtime
                    .state
                    .durable
                    .capture
                    .recording
                    .as_ref()
                    .context("shortcut recording")?
                    .title
                    .clone(),
            );
            assert_eq!(native.try_recv()?["command"], "audio_start");
        }
        assert_eq!(titles, ["Reunión 02:30", "Reunión"]);
        Ok(())
    }

    #[test]
    fn retired_audio_receipts_use_the_same_bodies_as_the_helper_fixtures() -> Result<()> {
        let start: Value = serde_json::from_str(include_str!(
            "../../../../contracts/fixtures/native/results/audio_start.json"
        ))?;
        let receipt: RetiredAudioStartResult = decode_result_body("audio_start", &start)?;
        assert_eq!(receipt.stream_id, "stream-1");
        let mut stop: Value = serde_json::from_str(include_str!(
            "../../../../contracts/fixtures/native/results/audio_stop.json"
        ))?;
        let receipt: RetiredAudioStopResult = decode_result_body("audio_stop", &stop)?;
        assert_eq!(receipt.stream_id, "stream-1");
        assert!(receipt.drain_complete);
        stop.as_object_mut()
            .context("stop fixture body")?
            .remove("drain_complete");
        let error = decode_result_body::<RetiredAudioStopResult>("audio_stop", &stop)
            .err()
            .context("stop without a drain receipt was accepted")?;
        assert_eq!(error.field, "drain_complete");
        assert_eq!(error.observed_type, "absent");
        Ok(())
    }
}
