//! The capture's frames and the holes the helper declares in them.
use super::super::*;
use orbit_core::notice::{Notice, NoticeCode, NoticeKey};
use orbit_core::wire::native::NativeAudioFrameRef;
use serde::Deserialize;

#[derive(Deserialize)]
struct NativeGapSource<'a> {
    source: &'a str,
    reason: Option<&'a str>,
    /// Samples the producer dropped (the helper's count). The daemon's own
    /// bridge counts dropped events here and states `duration_ns` instead.
    #[serde(default)]
    frames: Option<u64>,
    duration_ns: Option<u64>,
}

impl Runtime {
    pub(super) async fn on_native_audio(&mut self, event: &mut Value) -> Result<()> {
        self.on_native_audio_at(event, now()).await
    }

    /// A frame of audio as it arrived at `received_at_ms`.
    pub(in crate::daemon) async fn on_native_audio_at(
        &mut self,
        event: &mut Value,
        received_at_ms: i64,
    ) -> Result<()> {
        if !self.accepts_audio(event) {
            return Ok(());
        }
        // A producer's lifetime is stable for a shared PTT turn. The
        // recorder separately assigns its common start/resume epoch.
        let stream_id = NativeAudioFrameRef::from_value(event)?.stream_id.to_owned();
        event
            .as_object_mut()
            .context("Audio event has no object")?
            .insert("capture_epoch_id".into(), Value::String(stream_id));
        let turn_action = self.prepare_call_frame(event, received_at_ms).await?;
        let ingested = self.capture.buffer.ingest_observed(event, received_at_ms)?;
        if let Some(source) = ingested.gap {
            self.note_audio_hole(source, "audio_discontinuity")?;
        }
        self.observe_recording_signal(ingested.signal, std::time::Instant::now());
        self.seal(ingested.segments)?;
        self.finish_audio_failures();
        self.finish_call_frame(turn_action).await?;
        Ok(())
    }

    pub(super) async fn on_native_gap(&mut self, event: &Value) -> Result<()> {
        if self.accepts_audio(event) {
            // A hole in a live or interrupted recording is declared:
            // the gaps counter moves and an additive notice lands,
            // the recording keeps going and no privacy stop is
            // requested. A gap that arrives for a paused meeting or
            // for a stream this Mac no longer owns cannot be
            // attributed to a recording the helper is still feeding.
            // Malformed or unattributable gaps are dropped by the same
            // containment as damaged frames; neither can stop capture.
            match orbit_core::failure::classify_gap(&self.state) {
                Outcome::Degraded { kind, .. } => {
                    let recording_attributable = self
                        .state
                        .durable
                        .capture
                        .recording
                        .as_ref()
                        .is_some_and(|r| {
                            matches!(
                                r.phase,
                                CapturePhase::Recording
                                    | CapturePhase::Starting
                                    | CapturePhase::Interrupted
                            )
                        });
                    if recording_attributable {
                        self.apply(Event::Degraded(kind)).await?;
                        let gap = NativeGapSource::deserialize(event)?;
                        // Whoever dropped the audio (a new route, the helper's
                        // ring, either side of the bridge under backpressure)
                        // declared it: the tail before the hole is sealed and
                        // the producer's next sequence is accepted, so the
                        // hole never becomes contiguous PCM and never cuts the
                        // meeting. The next frame re-anchors its stream and clock.
                        let rate = self.capture.buffer.track_rate(gap.source);
                        let segments = self.capture.buffer.restart_source(gap.source)?;
                        self.seal(segments)?;
                        if gap.reason != Some("audio_device_changed") {
                            self.note_declared_gap(&gap, rate);
                        }
                        if gap.source == "mic_local" && gap.reason == Some("audio_device_changed") {
                            let mut notice = Notice::new(NoticeCode::AudioRouteRecovered);
                            if let Some(duration_ns) = gap.duration_ns {
                                self.log(
                                    "audio_route_gap",
                                    &[("duration_ns", &duration_ns.to_string())],
                                );
                                notice = notice.with(NoticeKey::ElapsedMs, duration_ns / 1_000_000);
                            } else {
                                self.log("audio_route_gap", &[("duration_unknown", "true")]);
                            }
                            if !self.state.privacy_stopped()
                                && !self.audio.audio_is_draining()
                                && self.state.durable.capture.recording.as_ref().is_some_and(
                                    |recording| recording.phase == CapturePhase::Recording,
                                )
                            {
                                self.say(notice);
                            }
                        }
                        self.track_gap(gap.source, gap.reason).await?;
                        self.note_bound_track_gap(gap.source)?;
                    } else if !self.state.privacy_stopped() {
                        bail!(
                            "The audio source reported a discontinuity without an active recording"
                        );
                    }
                }
                Outcome::Refused { .. } | Outcome::IntegrityFailure { .. } => {
                    // classify_gap always returns Degraded(AudioGap).
                }
            }
        }
        Ok(())
    }

    /// A declared hole is recorded with its duration: the producer's own
    /// `duration_ns` when it states one, otherwise its dropped samples at the
    /// track's rate. Unknown stays unknown, never zero.
    fn note_declared_gap(&mut self, gap: &NativeGapSource<'_>, rate: Option<u32>) {
        let duration_ms = gap
            .duration_ns
            .map(|nanoseconds| nanoseconds / 1_000_000)
            .or_else(|| match (gap.frames, rate) {
                (Some(frames), Some(rate)) if rate > 0 && frames > 0 => {
                    Some(frames.saturating_mul(1_000) / u64::from(rate))
                }
                _ => None,
            });
        let reason = gap.reason.unwrap_or("unspecified");
        let mut notice = Notice::new(NoticeCode::AudioGap).with_token(NoticeKey::Cause, reason);
        match duration_ms {
            Some(duration_ms) => {
                self.log(
                    "audio_gap",
                    &[
                        ("source", gap.source),
                        ("reason", reason),
                        ("duration_ms", &duration_ms.to_string()),
                    ],
                );
                notice = notice.with(NoticeKey::DurationMs, duration_ms);
            }
            None => self.log(
                "audio_gap",
                &[
                    ("source", gap.source),
                    ("reason", reason),
                    ("duration_unknown", "true"),
                ],
            ),
        }
        self.say(notice);
    }
}
