//! Bounded audio assembly outside native realtime callbacks.
//! Pausing a recording never pauses an explicitly active voice turn, or vice versa.
use anyhow::{Context, Result, bail};
use base64::{Engine, engine::general_purpose::STANDARD};
use orbit_core::audio_signal::{PcmSignal, measure_s16le};
use orbit_core::wire::native::NativeAudioFrameRef;
use orbit_store::ChunkMetadata;
use serde_json::Value;

pub struct Segment {
    pub metadata: ChunkMetadata,
    pub pcm: Vec<u8>,
}

/// Signal measured from the same decoded bytes admitted to the original.
/// This is a reception observation, not a durable-store receipt or speech label.
pub struct ObservedPcmSignal {
    pub source: &'static str,
    pub stream_id: String,
    pub sample_rate: u32,
    pub level: PcmSignal,
}

pub struct IngestedAudio {
    pub segments: Vec<Segment>,
    pub signal: Option<ObservedPcmSignal>,
}

mod tracks;
mod voice;
pub use tracks::RecordingTracks;
use tracks::TrackFrame;
pub use voice::VoiceBuffer;

#[derive(Default)]
pub struct CaptureBuffer {
    pub tracks: RecordingTracks,
    pub voice: Option<VoiceBuffer>,
}

impl CaptureBuffer {
    /// Includes the incomplete tail that has not become a sealed segment yet.
    pub fn has_pending_audio(&self) -> bool {
        self.voice.is_some() || self.tracks.has_unsealed()
    }

    /// Recording PCM received and not sealed yet (a voice turn aside).
    pub fn has_unsealed_recording_audio(&self) -> bool {
        self.tracks.has_unsealed()
    }

    pub fn recording_owner(&self, source: &str) -> Option<&str> {
        self.tracks.recording_owner(source)
    }

    /// Legacy callers have unknown epoch; no timestamp or identity is invented.
    #[cfg(test)]
    pub fn begin_recording(&mut self, id: String) -> Result<()> {
        self.tracks.begin_recording_epoch(id, None)
    }

    pub fn begin_recording_with_epoch(&mut self, id: String, epoch: String) -> Result<()> {
        self.tracks.begin_recording_with_epoch(id, epoch)
    }

    pub fn recording_epoch(&self) -> Option<&str> {
        self.tracks.recording_epoch()
    }

    pub fn restore_position(&mut self, metadata: &ChunkMetadata) -> Result<()> {
        self.tracks.restore_position(metadata)
    }

    pub fn begin_voice(&mut self) -> Result<()> {
        begin_voice(&mut self.voice)
    }

    /// Older fixtures need only segments; production consumes the observation too.
    #[cfg(test)]
    pub fn ingest(&mut self, frame: &Value, now_ms: i64) -> Result<Vec<Segment>> {
        self.ingest_observed(frame, now_ms)
            .map(|ingested| ingested.segments)
    }

    /// Observations leave this boundary only after the original was admitted.
    /// Voice-only frames and replayed preroll do not represent new original PCM.
    pub fn ingest_observed(&mut self, frame: &Value, now_ms: i64) -> Result<IngestedAudio> {
        ingest(&mut self.tracks, &mut self.voice, frame, now_ms, true)
    }

    /// Preroll belongs only to the derived utterance. The continuous original
    /// was already recorded when these frames first arrived and is not replayed.
    pub fn ingest_voice_frame(&mut self, frame: &Value, now_ms: i64) -> Result<()> {
        ingest(&mut self.tracks, &mut self.voice, frame, now_ms, false).map(|_| ())
    }

    pub fn track_rate(&self, source: &str) -> Option<u32> {
        self.tracks.track_rate(source)
    }

    /// A producer declared a hole (a new route, or audio it dropped on
    /// purpose). Seal the tail so the missing time never becomes contiguous
    /// PCM; only the producer sequence restarts, not the meeting or chunk
    /// counters, and the stream identity and clock are still checked. A voice
    /// turn on the same microphone skips the hole the same way.
    pub fn restart_source(&mut self, source: &str) -> Result<Vec<Segment>> {
        restart_source(&mut self.tracks, &mut self.voice, source)
    }

    /// End one recording source after native stop acknowledgment. Other sources
    /// can continue draining, and an independent voice turn can still use mic.
    pub fn pause_source(&mut self, source: &str) -> Result<Vec<Segment>> {
        self.tracks.pause_source(source)
    }

    pub fn flush(&mut self, end: bool) -> Vec<Segment> {
        self.tracks.flush(end)
    }
}

/// A new voice turn, refused while the previous one is still finishing.
pub fn begin_voice(voice: &mut Option<VoiceBuffer>) -> Result<()> {
    if voice.is_some() {
        bail!("El turno de voz anterior aún está finalizando");
    }
    *voice = Some(VoiceBuffer::new());
    Ok(())
}

/// A declared hole on `source`: see [`CaptureBuffer::restart_source`].
pub fn restart_source(
    tracks: &mut RecordingTracks,
    voice: &mut Option<VoiceBuffer>,
    source: &str,
) -> Result<Vec<Segment>> {
    if source == "mic_local"
        && let Some(voice) = voice.as_mut()
    {
        voice.skip_hole();
    }
    tracks.restart_source(source)
}

/// One frame into the recording (when `record` and its source feeds it) and
/// into the voice turn on the microphone. Every check runs before anything is
/// appended, the recording's first and the voice turn's next, so a refused
/// frame changes neither.
pub fn ingest(
    tracks: &mut RecordingTracks,
    voice: &mut Option<VoiceBuffer>,
    frame: &Value,
    now_ms: i64,
    record: bool,
) -> Result<IngestedAudio> {
    let decoded =
        NativeAudioFrameRef::from_value(frame).context("Evento audio con campo o tipo inválido")?;
    let source = match decoded.source {
        "mic_local" => "mic_local",
        "remote_app" => "remote_app",
        _ => bail!("Pista de audio no válida"),
    };
    let stream = Some(decoded.stream_id)
        .filter(|s| !s.is_empty() && s.len() <= 128)
        .context("Identidad de audio ausente")?;
    let rate = decoded.sample_rate;
    let channels = u16::from(decoded.channels);
    let frames = u64::from(decoded.frames);
    if !(8_000..=192_000).contains(&rate)
        || channels != 1
        || frames == 0
        || frames > u64::from(rate)
    {
        bail!("Formato de audio fuera de límites");
    }
    let pcm = frame_pcm_decoded(&decoded)?;
    let epoch = frame_epoch(decoded.capture_epoch_id)?;
    let checked = TrackFrame {
        source,
        stream,
        rate,
        channels,
        native_sequence: decoded.sequence,
        monotonic_ns: decoded.monotonic_ns,
        pcm: &pcm,
    };
    let recording = record && tracks.records(source);
    if recording {
        tracks.check(&checked)?;
    }
    if source == "mic_local"
        && let Some(voice) = voice.as_mut()
    {
        voice.check(&checked, epoch)?;
        voice.append(&checked, epoch, now_ms);
    }
    if !recording {
        return Ok(IngestedAudio {
            segments: Vec::new(),
            signal: None,
        });
    }
    // Never decode base64 again or trust separately declared native levels.
    // Measurement is optional; a missing level must not discard valid audio.
    let signal = measure_s16le(&pcm).map(|level| ObservedPcmSignal {
        source,
        stream_id: stream.to_owned(),
        sample_rate: rate,
        level,
    });
    let segments = tracks.append(&checked, now_ms)?;
    Ok(IngestedAudio { segments, signal })
}

/// Decode a single observed mono frame with hard bounds before base64 allocation.
pub fn frame_pcm(frame: &Value) -> Result<Vec<u8>> {
    let decoded =
        NativeAudioFrameRef::from_value(frame).context("Evento audio con campo o tipo inválido")?;
    frame_pcm_decoded(&decoded)
}

fn frame_pcm_decoded(frame: &NativeAudioFrameRef<'_>) -> Result<Vec<u8>> {
    let rate = frame.sample_rate;
    let frames = frame.frames;
    if !(8_000..=192_000).contains(&rate) || frame.channels != 1 || frames == 0 || frames > rate {
        bail!("Formato de audio fuera de límites");
    }
    let expected_bytes = usize::try_from(frames)? * 2;
    let encoded = frame.pcm_s16le_base64;
    if encoded.len() > expected_bytes.div_ceil(3) * 4 {
        bail!("PCM codificado fuera de límites");
    }
    let pcm = STANDARD.decode(encoded)?;
    if pcm.len() != expected_bytes {
        bail!("Longitud PCM inconsistente");
    }
    Ok(pcm)
}

pub(crate) fn validate_epoch_id(epoch: &str) -> Result<()> {
    if epoch.is_empty()
        || epoch.len() > 128
        || matches!(epoch, "." | "..")
        || !epoch
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_.:".contains(&byte))
    {
        bail!("Identidad de época de captura no válida");
    }
    Ok(())
}

fn frame_epoch(epoch: Option<&str>) -> Result<Option<&str>> {
    match epoch {
        None => Ok(None),
        Some(epoch) => {
            validate_epoch_id(epoch)?;
            Ok(Some(epoch))
        }
    }
}

#[cfg(test)]
#[path = "capture_observation_tests.rs"]
mod observation_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use orbit_store::MAX_CHUNK_BYTES;
    use serde_json::json;

    fn frame(sequence: u64, frames: usize) -> Value {
        json!({"source":"mic_local", "stream_id":"mic", "sample_rate":8000,
            "channels":1, "frames":frames, "sequence":sequence,
            "monotonic_ns":sequence.saturating_mul(1000),
            "pcm_s16le_base64":STANDARD.encode(vec![1; frames * 2])})
    }

    /// Audio read late, all at once after a held daemon, keeps the time it
    /// was captured: each chunk starts at the stream's first wall time plus
    /// the producer clock, not at the moment the backlog was read.
    #[test]
    fn a_backlog_read_at_once_keeps_its_capture_times() -> Result<()> {
        let mut buffer = CaptureBuffer::default();
        buffer.begin_recording("meeting".into())?;
        let second = |sequence: u64| {
            let mut frame = frame(sequence, 8000);
            frame["monotonic_ns"] = json!(sequence * 1_000_000_000);
            frame
        };
        let first = buffer.ingest(&second(0), 1_000)?;
        let mut late = Vec::new();
        for sequence in 1..4 {
            late.extend(buffer.ingest(&second(sequence), 20_000)?);
        }
        let starts: Vec<i64> = first
            .iter()
            .chain(&late)
            .map(|segment| segment.metadata.started_at_ms)
            .collect();
        assert_eq!(starts, [1_000, 2_000, 3_000, 4_000]);
        // A new stream anchors its own clock again.
        buffer.pause_source("mic_local")?;
        buffer.begin_recording("meeting".into())?;
        let mut resumed = frame(4, 8000);
        resumed["stream_id"] = json!("mic-again");
        resumed["monotonic_ns"] = json!(90_000_000_000_u64);
        let resumed = buffer.ingest(&resumed, 30_000)?;
        assert_eq!(resumed[0].metadata.started_at_ms, 30_000);
        Ok(())
    }

    #[test]
    fn voice_preserves_first_clocks_and_rejects_clock_regression() -> Result<()> {
        let mut buffer = CaptureBuffer::default();
        buffer.begin_voice()?;
        buffer.ingest(&frame(5, 100), 42)?;
        buffer.ingest(&frame(6, 100), 99)?;
        let mut backwards = frame(7, 100);
        backwards["monotonic_ns"] = json!(1);
        assert!(buffer.ingest(&backwards, 100).is_err());
        let voice = buffer.voice.as_ref().context("voice absent")?;
        assert_eq!(voice.started_at_ms, 42);
        assert_eq!(voice.monotonic_ns, 5000);
        assert_eq!(voice.pcm.len(), 400);
        Ok(())
    }

    #[test]
    fn voice_leaves_room_for_the_wav_header_before_appending() -> Result<()> {
        let mut buffer = CaptureBuffer::default();
        buffer.begin_voice()?;
        let mut first = frame(0, 100);
        first["sample_rate"] = json!(192000);
        buffer.ingest(&first, 42)?;
        let accepted = MAX_CHUNK_BYTES - 44 - 100;
        buffer
            .voice
            .as_mut()
            .context("voice absent")?
            .pcm
            .resize(accepted, 0);
        first["sequence"] = json!(1);
        assert!(buffer.ingest(&first, 43).is_err());
        assert_eq!(
            buffer.voice.as_ref().context("voice absent")?.pcm.len(),
            accepted
        );
        Ok(())
    }

    #[test]
    fn paused_recording_does_not_capture_a_live_voice_turn() -> Result<()> {
        let mut buffer = CaptureBuffer::default();
        buffer.begin_recording("meeting".into())?;
        buffer.begin_voice()?;
        assert!(buffer.ingest(&frame(0, 100), 1)?.is_empty());
        assert_eq!(buffer.flush(false)[0].metadata.frames, 100);
        assert!(buffer.ingest(&frame(1, 8000), 2)?.is_empty());
        assert!(buffer.flush(false).is_empty());
        assert_eq!(
            buffer.voice.as_ref().context("voice absent")?.pcm.len(),
            16_200
        );
        buffer.begin_recording("meeting".into())?;
        let segments = buffer.ingest(&frame(2, 8000), 3)?;
        assert_eq!(segments[0].metadata.sequence, 1);
        assert_eq!(segments[0].metadata.first_sample, 100);
        Ok(())
    }

    #[test]
    fn track_pause_allows_other_track_to_drain() -> Result<()> {
        let mut buffer = CaptureBuffer::default();
        buffer.begin_recording("meeting".into())?;
        buffer.ingest(&frame(0, 100), 1)?;
        assert_eq!(buffer.pause_source("mic_local")?[0].metadata.frames, 100);
        assert!(buffer.ingest(&frame(1, 100), 2)?.is_empty());
        let mut remote = frame(0, 8000);
        remote["source"] = json!("remote_app");
        remote["stream_id"] = json!("remote");
        assert_eq!(buffer.ingest(&remote, 3)?[0].metadata.track, "remote_app");
        Ok(())
    }

    #[test]
    fn gaps_and_counter_wrap_are_rejected_before_contaminating_voice() -> Result<()> {
        let mut buffer = CaptureBuffer::default();
        buffer.begin_recording("meeting".into())?;
        buffer.begin_voice()?;
        buffer.ingest(&frame(u64::MAX, 100), 1)?;
        assert!(buffer.ingest(&frame(0, 100), 2).is_err());
        assert_eq!(
            buffer.voice.as_ref().context("voice absent")?.pcm.len(),
            200
        );
        assert_eq!(buffer.flush(true)[0].metadata.frames, 100);
        Ok(())
    }

    #[test]
    fn resume_after_restart_preserves_chunk_and_sample_identity() -> Result<()> {
        let mut original = CaptureBuffer::default();
        original.begin_recording("meeting".into())?;
        let old = original.ingest(&frame(0, 8000), 1)?.remove(0);
        let mut restored = CaptureBuffer::default();
        restored.begin_recording("meeting".into())?;
        restored.restore_position(&old.metadata)?;
        let new = restored.ingest(&frame(0, 8000), 3)?.remove(0);
        assert_eq!(new.metadata.sequence, 1);
        assert_eq!(new.metadata.first_sample, 8000);
        assert_ne!(new.metadata.id, old.metadata.id);
        Ok(())
    }

    #[test]
    fn malformed_large_frames_are_refused_without_recording_data() -> Result<()> {
        let mut buffer = CaptureBuffer::default();
        buffer.begin_recording("meeting".into())?;
        let mut malformed = frame(0, 1);
        malformed["pcm_s16le_base64"] = json!("A".repeat(100_000));
        assert!(buffer.ingest(&malformed, 1).is_err());
        assert!(buffer.flush(true).is_empty());
        Ok(())
    }

    #[test]
    fn legacy_scope_does_not_invent_epoch_from_frames_or_clock() -> Result<()> {
        let mut buffer = CaptureBuffer::default();
        buffer.begin_recording("meeting".into())?;
        assert_eq!(buffer.recording_epoch(), None);
        let mut observed = frame(0, 8000);
        observed["capture_epoch_id"] = json!("producer-only");
        let segment = buffer.ingest(&observed, 1)?.remove(0);
        assert!(segment.metadata.capture_epoch_id.is_none());
        buffer.flush(true);
        assert_eq!(buffer.recording_epoch(), None);
        Ok(())
    }

    #[test]
    fn a_recording_epoch_cannot_relabel_buffered_tails() -> Result<()> {
        let mut buffer = CaptureBuffer::default();
        buffer.begin_recording_with_epoch("meeting".into(), "epoch-one".into())?;
        buffer.ingest(&frame(0, 100), 1)?;
        assert!(
            buffer
                .begin_recording_with_epoch("meeting".into(), "epoch-two".into())
                .is_err()
        );
        assert!(buffer.begin_recording("meeting".into()).is_err());
        assert_eq!(buffer.recording_epoch(), Some("epoch-one"));
        let previous = buffer.flush(false).remove(0);
        assert_eq!(
            previous.metadata.capture_epoch_id.as_deref(),
            Some("epoch-one")
        );
        assert_eq!(previous.pcm, vec![1; 200]);
        buffer.begin_recording_with_epoch("meeting".into(), "epoch-two".into())?;
        let mut next = frame(0, 8000);
        next["stream_id"] = json!("new-producer");
        let segment = buffer.ingest(&next, 2)?.remove(0);
        assert_eq!(
            segment.metadata.capture_epoch_id.as_deref(),
            Some("epoch-two")
        );
        assert_eq!(segment.metadata.sequence, 1);
        assert_eq!(segment.metadata.first_sample, 100);
        assert_eq!(
            previous.metadata.capture_epoch_id.as_deref(),
            Some("epoch-one")
        );
        buffer.flush(true);
        assert_eq!(buffer.recording_epoch(), None);
        Ok(())
    }

    #[test]
    fn both_tracks_share_scope_epoch_without_relabeling_live_ptt() -> Result<()> {
        let mut buffer = CaptureBuffer::default();
        buffer.begin_recording_with_epoch("meeting".into(), "scope-one".into())?;
        buffer.begin_voice()?;
        let mut mic = frame(0, 100);
        mic["capture_epoch_id"] = json!("mic-producer");
        buffer.ingest(&mic, 1)?;
        let mut remote = frame(0, 100);
        remote["source"] = json!("remote_app");
        remote["stream_id"] = json!("remote");
        remote["capture_epoch_id"] = json!("remote-producer");
        buffer.ingest(&remote, 1)?;
        let segments = buffer.flush(false);
        assert_eq!(segments.len(), 2);
        assert!(
            segments
                .iter()
                .all(|segment| segment.metadata.capture_epoch_id.as_deref() == Some("scope-one"))
        );
        buffer.begin_recording_with_epoch("meeting".into(), "scope-two".into())?;
        mic["sequence"] = json!(1);
        mic["monotonic_ns"] = json!(1000);
        buffer.ingest(&mic, 2)?;
        assert_eq!(
            buffer
                .voice
                .as_ref()
                .context("voice absent")?
                .capture_epoch_id
                .as_deref(),
            Some("mic-producer")
        );
        assert_eq!(
            buffer.voice.as_ref().context("voice absent")?.pcm.len(),
            400
        );
        assert_eq!(
            buffer.flush(false)[0].metadata.capture_epoch_id.as_deref(),
            Some("scope-two")
        );
        Ok(())
    }

    #[test]
    fn voice_epoch_changes_including_known_unknown_transitions_are_rejected() -> Result<()> {
        for (first, changed) in [
            (json!("producer-one"), json!("producer-two")),
            (json!("producer-one"), Value::Null),
            (Value::Null, json!("producer-two")),
        ] {
            let mut buffer = CaptureBuffer::default();
            buffer.begin_voice()?;
            let mut initial = frame(0, 100);
            initial["capture_epoch_id"] = first.clone();
            buffer.ingest(&initial, 1)?;
            let mut next = frame(1, 100);
            next["capture_epoch_id"] = changed;
            assert!(buffer.ingest(&next, 2).is_err());
            let voice = buffer.voice.as_ref().context("voice absent")?;
            assert_eq!(voice.capture_epoch_id.as_deref(), first.as_str());
            assert_eq!(voice.pcm, vec![1; 200]);
            assert_eq!(voice.last_sequence(), Some(0));
        }
        Ok(())
    }

    #[test]
    fn restoration_keeps_new_explicit_epoch_and_prior_sample_identity() -> Result<()> {
        let mut original = CaptureBuffer::default();
        original.begin_recording_with_epoch("meeting".into(), "old-scope".into())?;
        let mut first = frame(0, 8000);
        first["monotonic_ns"] = json!(900_000_000_000_u64);
        let old = original.ingest(&first, 1)?.remove(0);
        let mut restored = CaptureBuffer::default();
        restored.begin_recording_with_epoch("meeting".into(), "new-scope".into())?;
        restored.restore_position(&old.metadata)?;
        let new = restored.ingest(&frame(0, 8000), 3)?.remove(0);
        assert_eq!(new.metadata.capture_epoch_id.as_deref(), Some("new-scope"));
        assert_eq!(new.metadata.sequence, 1);
        assert_eq!(new.metadata.first_sample, 8000);
        assert!(new.metadata.monotonic_ns < old.metadata.monotonic_ns);
        Ok(())
    }

    #[test]
    fn malformed_epochs_fail_before_modifying_capture() -> Result<()> {
        let mut buffer = CaptureBuffer::default();
        for epoch in ["", ".", "..", "../outside", "has space", "é"] {
            assert!(
                buffer
                    .begin_recording_with_epoch("meeting".into(), epoch.into())
                    .is_err()
            );
            assert!(buffer.recording_owner("mic_local").is_none());
        }
        buffer.begin_voice()?;
        for epoch in [
            json!(""),
            json!("../outside"),
            json!("x".repeat(129)),
            json!(true),
            json!({}),
        ] {
            let mut observed = frame(0, 100);
            observed["capture_epoch_id"] = epoch;
            assert!(buffer.ingest(&observed, 1).is_err());
            assert!(
                buffer
                    .voice
                    .as_ref()
                    .context("voice absent")?
                    .pcm
                    .is_empty()
            );
        }
        Ok(())
    }
}
