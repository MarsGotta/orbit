//! A voice turn on the microphone: its PCM and the producer facts that tell
//! a continuous turn from one with a hole.
use super::tracks::TrackFrame;
use anyhow::{Result, bail};
use orbit_store::MAX_CHUNK_BYTES;

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
    pub(super) fn new() -> Self {
        Self {
            pcm: Vec::new(),
            sample_rate: 0,
            channels: 1,
            started_at_ms: 0,
            monotonic_ns: 0,
            capture_epoch_id: None,
            native_stream: None,
            native_sequence: None,
            last_monotonic_ns: None,
        }
    }

    /// The helper's sequence of the last frame in this turn.
    pub fn last_sequence(&self) -> Option<u64> {
        self.native_sequence
    }

    /// A declared hole on the microphone: the next frame need not follow.
    pub(super) fn skip_hole(&mut self) {
        self.native_sequence = None;
    }

    /// Refuses a frame that would not continue this turn, or would make it
    /// longer than a turn may be.
    pub(super) fn check(&self, frame: &TrackFrame<'_>, epoch: Option<&str>) -> Result<()> {
        if !self.pcm.is_empty() && self.capture_epoch_id.as_deref() != epoch {
            bail!("La época del productor cambió durante el turno de voz");
        }
        if self.sample_rate != 0
            && (self.sample_rate != frame.rate || self.channels != frame.channels)
        {
            bail!("El micrófono cambió de formato durante el turno");
        }
        if self
            .native_sequence
            .is_some_and(|previous| previous.checked_add(1) != Some(frame.native_sequence))
            || self
                .native_stream
                .as_deref()
                .is_some_and(|previous| previous != frame.stream)
            || self
                .last_monotonic_ns
                .is_some_and(|previous| frame.monotonic_ns < previous)
        {
            bail!("El turno de voz contiene una discontinuidad");
        }
        if self.pcm.len() + frame.pcm.len() > frame.rate as usize * 2 * 60 {
            bail!("El turno de voz alcanzó el límite de 60 segundos");
        }
        if self.pcm.len() + frame.pcm.len() > MAX_CHUNK_BYTES - 44 {
            bail!("El turno de voz alcanzó el límite de almacenamiento cifrado de 8 MiB");
        }
        Ok(())
    }

    pub(super) fn append(&mut self, frame: &TrackFrame<'_>, epoch: Option<&str>, now_ms: i64) {
        if self.pcm.is_empty() {
            self.started_at_ms = now_ms;
            self.monotonic_ns = frame.monotonic_ns;
            self.capture_epoch_id = epoch.map(str::to_owned);
        }
        self.sample_rate = frame.rate;
        self.channels = frame.channels;
        self.native_stream = Some(frame.stream.into());
        self.native_sequence = Some(frame.native_sequence);
        self.last_monotonic_ns = Some(frame.monotonic_ns);
        self.pcm.extend_from_slice(frame.pcm);
    }
}
