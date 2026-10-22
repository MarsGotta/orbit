//! Bounded audio assembly outside native realtime callbacks.
//! Pausing a recording never pauses an explicitly active voice turn, or vice versa.
use anyhow::{Context, Result, bail};
use base64::{Engine, engine::general_purpose::STANDARD};
use orbit_core::audio_signal::{PcmSignal, measure_s16le};
use orbit_core::wire::native::NativeAudioFrameRef;
use orbit_store::{ChunkMetadata, MAX_CHUNK_BYTES};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

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

struct Track {
    pcm: Vec<u8>,
    rate: u32,
    channels: u16,
    sequence: u64,
    native_sequence: Option<u64>,
    native_stream: Option<String>,
    last_monotonic_ns: Option<u64>,
    first_sample: u64,
    start_ms: i64,
    monotonic_ns: u64,
}

impl Track {
    fn new(rate: u32, channels: u16) -> Self {
        Self {
            pcm: Vec::new(),
            rate,
            channels,
            sequence: 0,
            native_sequence: None,
            native_stream: None,
            last_monotonic_ns: None,
            first_sample: 0,
            start_ms: 0,
            monotonic_ns: 0,
        }
    }
}

#[derive(Default)]
pub struct CaptureBuffer {
    recording_id: Option<String>,
    capture_epoch_id: Option<String>,
    recording_sources: BTreeSet<String>,
    tracks: BTreeMap<String, Track>,
    pub voice: Option<VoiceBuffer>,
}

pub struct VoiceBuffer {
    pub pcm: Vec<u8>,
    pub sample_rate: u32,
    pub channels: u16,
    /// Estimated wall clock at receipt of the first native frame.
    pub started_at_ms: i64,
    /// The first native timestamp, never reconstructed from wall time.
    pub monotonic_ns: u64,
    /// Optional producer epoch observed in the first frame. Independent of the
    /// recording scope so PTT can share a producer across a meeting boundary.
    pub capture_epoch_id: Option<String>,
    native_stream: Option<String>,
    native_sequence: Option<u64>,
    last_monotonic_ns: Option<u64>,
}

impl VoiceBuffer {
    /// The helper's sequence of the last frame in this turn.
    pub fn last_sequence(&self) -> Option<u64> {
        self.native_sequence
    }
}

impl CaptureBuffer {
    /// Includes the incomplete tail that has not become a sealed segment yet.
    pub fn has_pending_audio(&self) -> bool {
        self.voice.is_some() || self.tracks.values().any(|track| !track.pcm.is_empty())
    }

    /// Recording PCM received and not sealed yet (a voice turn aside).
    pub fn has_unsealed_recording_audio(&self) -> bool {
        self.tracks.values().any(|track| !track.pcm.is_empty())
    }

    /// Ownership lasts through native drain, regardless of the UI capture
    /// phase. A mic used only by PTT must not affect a paused/closed recording.
    pub fn recording_owner(&self, source: &str) -> Option<&str> {
        self.recording_sources
            .contains(source)
            .then_some(self.recording_id.as_deref())
            .flatten()
    }

    /// Legacy callers have unknown epoch; no timestamp or identity is invented.
    #[cfg(test)]
    pub fn begin_recording(&mut self, id: String) -> Result<()> {
        self.begin_recording_epoch(id, None)
    }

    /// Begin an explicitly identified capture scope after prior producers have
    /// drained. Recording tails must be sealed before changing their epoch.
    /// Independent PTT retains its own producer epoch and is not relabeled.
    pub fn begin_recording_with_epoch(&mut self, id: String, epoch: String) -> Result<()> {
        validate_epoch_id(&epoch)?;
        self.begin_recording_epoch(id, Some(epoch))
    }

    pub fn recording_epoch(&self) -> Option<&str> {
        self.recording_id.as_ref()?;
        self.capture_epoch_id.as_deref()
    }

    fn begin_recording_epoch(&mut self, id: String, epoch: Option<String>) -> Result<()> {
        if self
            .recording_id
            .as_ref()
            .is_some_and(|current| current != &id)
        {
            bail!("Finaliza la grabación anterior antes de cambiar su identidad");
        }
        if self.capture_epoch_id != epoch {
            if self.tracks.values().any(|track| !track.pcm.is_empty()) {
                bail!("Sella el audio pendiente antes de cambiar la época de captura");
            }
            for track in self.tracks.values_mut() {
                track.native_sequence = None;
                track.native_stream = None;
                track.last_monotonic_ns = None;
            }
        }
        self.recording_id = Some(id);
        self.capture_epoch_id = epoch;
        self.recording_sources
            .extend(["mic_local".into(), "remote_app".into()]);
        Ok(())
    }

    /// Rebuild counters from authenticated durable manifests after a daemon restart.
    /// Calling this for every page uses constant memory (one cursor per track).
    pub fn restore_position(&mut self, metadata: &ChunkMetadata) -> Result<()> {
        if self.recording_id.as_deref() != Some(&metadata.recording_id) {
            bail!("El fragmento pertenece a otra grabación");
        }
        let track = self
            .tracks
            .entry(metadata.track.clone())
            .or_insert_with(|| Track::new(metadata.sample_rate, metadata.channels));
        if !track.pcm.is_empty() {
            bail!("No se puede restaurar una pista mientras recibe audio");
        }
        if track.rate != metadata.sample_rate || track.channels != metadata.channels {
            bail!("La pista guardada contiene formatos incompatibles");
        }
        track.sequence = track.sequence.max(
            metadata
                .sequence
                .checked_add(1)
                .context("Secuencia agotada")?,
        );
        track.first_sample = track.first_sample.max(
            metadata
                .first_sample
                .checked_add(metadata.frames)
                .context("Contador de muestras agotado")?,
        );
        Ok(())
    }

    pub fn begin_voice(&mut self) -> Result<()> {
        if self.voice.is_some() {
            bail!("El turno de voz anterior aún está finalizando");
        }
        self.voice = Some(VoiceBuffer {
            pcm: Vec::new(),
            sample_rate: 0,
            channels: 1,
            started_at_ms: 0,
            monotonic_ns: 0,
            capture_epoch_id: None,
            native_stream: None,
            native_sequence: None,
            last_monotonic_ns: None,
        });
        Ok(())
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
        self.ingest_selected(frame, now_ms, true)
    }

    /// Preroll belongs only to the derived utterance. The continuous original
    /// was already recorded when these frames first arrived and is not replayed.
    pub fn ingest_voice_frame(&mut self, frame: &Value, now_ms: i64) -> Result<()> {
        self.ingest_selected(frame, now_ms, false).map(|_| ())
    }

    fn ingest_selected(
        &mut self,
        frame: &Value,
        now_ms: i64,
        record: bool,
    ) -> Result<IngestedAudio> {
        let decoded = NativeAudioFrameRef::from_value(frame)
            .context("Evento audio con campo o tipo inválido")?;
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
        let native_sequence = decoded.sequence;
        let monotonic_ns = decoded.monotonic_ns;
        if !(8_000..=192_000).contains(&rate)
            || channels != 1
            || frames == 0
            || frames > u64::from(rate)
        {
            bail!("Formato de audio fuera de límites");
        }
        let pcm = frame_pcm_decoded(&decoded)?;
        let capture_epoch_id = frame_epoch(decoded.capture_epoch_id)?;

        let recording =
            record && self.recording_id.is_some() && self.recording_sources.contains(source);
        if recording && let Some(track) = self.tracks.get(source) {
            if track.rate != rate || track.channels != channels {
                bail!("Formato de pista cambiado; inicia una grabación nueva");
            }
            if track
                .native_sequence
                .is_some_and(|previous| previous.checked_add(1) != Some(native_sequence))
                || track
                    .native_stream
                    .as_deref()
                    .is_some_and(|previous| previous != stream)
                || track
                    .last_monotonic_ns
                    .is_some_and(|previous| monotonic_ns < previous)
            {
                bail!("Discontinuidad en la secuencia o reloj de audio");
            }
        }
        if source == "mic_local"
            && let Some(voice) = self.voice.as_mut()
        {
            if !voice.pcm.is_empty() && voice.capture_epoch_id.as_deref() != capture_epoch_id {
                bail!("La época del productor cambió durante el turno de voz");
            }
            if voice.sample_rate != 0 && (voice.sample_rate != rate || voice.channels != channels) {
                bail!("El micrófono cambió de formato durante el turno");
            }
            if voice
                .native_sequence
                .is_some_and(|previous| previous.checked_add(1) != Some(native_sequence))
                || voice
                    .native_stream
                    .as_deref()
                    .is_some_and(|previous| previous != stream)
                || voice
                    .last_monotonic_ns
                    .is_some_and(|previous| monotonic_ns < previous)
            {
                bail!("El turno de voz contiene una discontinuidad");
            }
            if voice.pcm.len() + pcm.len() > rate as usize * 2 * 60 {
                bail!("El turno de voz alcanzó el límite de 60 segundos");
            }
            if voice.pcm.len() + pcm.len() > MAX_CHUNK_BYTES - 44 {
                bail!("El turno de voz alcanzó el límite de almacenamiento cifrado de 8 MiB");
            }
            if voice.pcm.is_empty() {
                voice.started_at_ms = now_ms;
                voice.monotonic_ns = monotonic_ns;
                voice.capture_epoch_id = capture_epoch_id.map(str::to_owned);
            }
            voice.sample_rate = rate;
            voice.channels = channels;
            voice.native_stream = Some(stream.into());
            voice.native_sequence = Some(native_sequence);
            voice.last_monotonic_ns = Some(monotonic_ns);
            voice.pcm.extend_from_slice(&pcm);
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
        let recording_id = self
            .recording_id
            .as_deref()
            .context("Identidad de grabación ausente")?;
        let track = self
            .tracks
            .entry(source.into())
            .or_insert_with(|| Track::new(rate, channels));
        track.native_sequence = Some(native_sequence);
        track.native_stream = Some(stream.into());
        track.last_monotonic_ns = Some(monotonic_ns);
        if track.pcm.is_empty() {
            track.start_ms = now_ms;
            track.monotonic_ns = monotonic_ns;
        }
        track.pcm.extend_from_slice(&pcm);
        let segments = if track.pcm.len() >= rate as usize * usize::from(channels) * 2 {
            vec![seal(
                recording_id,
                self.capture_epoch_id.as_deref(),
                source,
                track,
            )?]
        } else {
            Vec::new()
        };
        Ok(IngestedAudio { segments, signal })
    }

    /// The sample rate a recording track was opened with, if any.
    pub fn track_rate(&self, source: &str) -> Option<u32> {
        self.tracks.get(source).map(|track| track.rate)
    }

    /// A producer declared a hole (a new route, or audio it dropped on
    /// purpose). Seal the tail so the missing time never becomes contiguous
    /// PCM; only the producer sequence restarts, not the meeting or chunk
    /// counters, and the stream identity and clock are still checked. A voice
    /// turn on the same microphone skips the hole the same way.
    pub fn restart_source(&mut self, source: &str) -> Result<Vec<Segment>> {
        if source == "mic_local"
            && let Some(voice) = self.voice.as_mut()
        {
            voice.native_sequence = None;
        }
        let mut segments = Vec::new();
        if let (Some(id), Some(track)) = (self.recording_id.as_deref(), self.tracks.get_mut(source))
        {
            if !track.pcm.is_empty() {
                segments.push(seal(id, self.capture_epoch_id.as_deref(), source, track)?);
            }
            track.native_sequence = None;
        }
        Ok(segments)
    }

    /// End one recording source after native stop acknowledgment. Other sources
    /// can continue draining, and an independent voice turn can still use mic.
    pub fn pause_source(&mut self, source: &str) -> Result<Vec<Segment>> {
        self.recording_sources.remove(source);
        let mut result = Vec::new();
        if let (Some(id), Some(track)) = (self.recording_id.as_deref(), self.tracks.get_mut(source))
        {
            if !track.pcm.is_empty() {
                result.push(seal(id, self.capture_epoch_id.as_deref(), source, track)?);
            }
            track.native_sequence = None;
            track.native_stream = None;
            track.last_monotonic_ns = None;
        }
        Ok(result)
    }

    pub fn flush(&mut self, end: bool) -> Vec<Segment> {
        let mut segments = Vec::new();
        for source in ["mic_local", "remote_app"] {
            // Counters are checked before ingest can advance them. Exhaustion
            // cannot occur with bounded real capture; preserve buffers on error.
            if let Ok(mut sealed) = self.pause_source(source) {
                segments.append(&mut sealed);
            }
        }
        if end && self.tracks.values().all(|track| track.pcm.is_empty()) {
            self.recording_id = None;
            self.capture_epoch_id = None;
            self.tracks.clear();
        }
        segments
    }
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

fn validate_epoch_id(epoch: &str) -> Result<()> {
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

fn seal(
    recording_id: &str,
    capture_epoch_id: Option<&str>,
    source: &str,
    track: &mut Track,
) -> Result<Segment> {
    let frames = track.pcm.len() as u64 / (u64::from(track.channels) * 2);
    let next_sequence = track.sequence.checked_add(1).context("Secuencia agotada")?;
    let next_sample = track
        .first_sample
        .checked_add(frames)
        .context("Contador de muestras agotado")?;
    let metadata = ChunkMetadata {
        id: format!("{recording_id}-{source}-{}", track.sequence),
        recording_id: recording_id.into(),
        track: source.into(),
        capture_epoch_id: capture_epoch_id.map(str::to_owned),
        sequence: track.sequence,
        sample_rate: track.rate,
        channels: track.channels,
        frames,
        started_at_ms: track.start_ms,
        monotonic_ns: track.monotonic_ns,
        first_sample: track.first_sample,
    };
    track.sequence = next_sequence;
    track.first_sample = next_sample;
    Ok(Segment {
        metadata,
        pcm: std::mem::take(&mut track.pcm),
    })
}

#[cfg(test)]
#[path = "capture_observation_tests.rs"]
mod observation_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn frame(sequence: u64, frames: usize) -> Value {
        json!({"source":"mic_local", "stream_id":"mic", "sample_rate":8000,
            "channels":1, "frames":frames, "sequence":sequence,
            "monotonic_ns":sequence.saturating_mul(1000),
            "pcm_s16le_base64":STANDARD.encode(vec![1; frames * 2])})
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
            assert_eq!(voice.native_sequence, Some(0));
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
