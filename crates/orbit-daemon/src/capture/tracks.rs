//! A recording's tracks: the PCM each one has received and not sealed yet,
//! its producer's sequence and clock, and the chunk counters it continues.
use super::{Segment, validate_epoch_id};
use anyhow::{Context, Result, bail};
use orbit_store::ChunkMetadata;
use std::collections::{BTreeMap, BTreeSet};

/// The wall clock of a stream's first frame paired with its producer clock.
/// Every chunk of that stream starts at this wall time plus the producer's
/// elapsed time, so audio that arrives late (a held daemon, a backlog the
/// helper kept) keeps the time it was captured instead of the time it was
/// read.
#[derive(Clone, Copy)]
struct ClockAnchor {
    wall_ms: i64,
    monotonic_ns: u64,
}

impl ClockAnchor {
    /// The wall time of a frame captured at `monotonic_ns`, never later than
    /// it was received: capture always precedes receipt.
    fn wall_at(self, monotonic_ns: u64, received_ms: i64) -> i64 {
        let elapsed_ms = monotonic_ns.saturating_sub(self.monotonic_ns) / 1_000_000;
        let elapsed_ms = i64::try_from(elapsed_ms).unwrap_or(i64::MAX);
        self.wall_ms.saturating_add(elapsed_ms).min(received_ms)
    }
}

pub(super) struct Track {
    pub(super) pcm: Vec<u8>,
    pub(super) rate: u32,
    pub(super) channels: u16,
    sequence: u64,
    native_sequence: Option<u64>,
    native_stream: Option<String>,
    last_monotonic_ns: Option<u64>,
    first_sample: u64,
    start_ms: i64,
    monotonic_ns: u64,
    clock: Option<ClockAnchor>,
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
            clock: None,
        }
    }

    /// The producer's identity, sequence and clock are forgotten: the next
    /// frame starts a new stream, with its own clock anchor.
    fn forget_producer(&mut self) {
        self.native_sequence = None;
        self.native_stream = None;
        self.last_monotonic_ns = None;
        self.clock = None;
    }
}

/// One frame already decoded and checked against its bounds.
pub(super) struct TrackFrame<'a> {
    pub(super) source: &'static str,
    pub(super) stream: &'a str,
    pub(super) rate: u32,
    pub(super) channels: u16,
    pub(super) native_sequence: u64,
    pub(super) monotonic_ns: u64,
    pub(super) pcm: &'a [u8],
}

#[derive(Default)]
pub struct RecordingTracks {
    recording_id: Option<String>,
    capture_epoch_id: Option<String>,
    recording_sources: BTreeSet<String>,
    tracks: BTreeMap<String, Track>,
}

impl RecordingTracks {
    /// Recording PCM received and not sealed yet.
    pub fn has_unsealed(&self) -> bool {
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

    pub fn recording_id(&self) -> Option<&str> {
        self.recording_id.as_deref()
    }

    pub fn recording_epoch(&self) -> Option<&str> {
        self.recording_id.as_ref()?;
        self.capture_epoch_id.as_deref()
    }

    /// Whether `source` feeds the recording now.
    pub fn records(&self, source: &str) -> bool {
        self.recording_id.is_some() && self.recording_sources.contains(source)
    }

    /// Begin an explicitly identified capture scope after prior producers have
    /// drained. Recording tails must be sealed before changing their epoch.
    pub fn begin_recording_with_epoch(&mut self, id: String, epoch: String) -> Result<()> {
        validate_epoch_id(&epoch)?;
        self.begin_recording_epoch(id, Some(epoch))
    }

    pub(super) fn begin_recording_epoch(
        &mut self,
        id: String,
        epoch: Option<String>,
    ) -> Result<()> {
        if self
            .recording_id
            .as_ref()
            .is_some_and(|current| current != &id)
        {
            bail!("Finish the previous recording before changing its identity");
        }
        if self.capture_epoch_id != epoch {
            if self.has_unsealed() {
                bail!("Seal pending audio before changing the capture epoch");
            }
            self.tracks.values_mut().for_each(Track::forget_producer);
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
            bail!("The chunk belongs to another recording");
        }
        let track = self
            .tracks
            .entry(metadata.track.clone())
            .or_insert_with(|| Track::new(metadata.sample_rate, metadata.channels));
        if !track.pcm.is_empty() {
            bail!("Cannot restore a track while it receives audio");
        }
        if track.rate != metadata.sample_rate || track.channels != metadata.channels {
            bail!("The stored track contains incompatible formats");
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

    /// A track keeps the format it started with: the hub joins its chunks
    /// into one stream. The helper converts every route to it, so a frame in
    /// another format is refused (and counted as a hole), never a new format.
    pub(super) fn check_format(&self, frame: &TrackFrame<'_>) -> Result<()> {
        if self
            .tracks
            .get(frame.source)
            .is_some_and(|track| track.rate != frame.rate || track.channels != frame.channels)
        {
            bail!("Track format changed; the chunk is discarded");
        }
        Ok(())
    }

    /// A discontinuity starts another stretch of the same track. It is a
    /// hole, never a reason to stop the recorder.
    pub(super) fn discontinuous(&self, frame: &TrackFrame<'_>) -> bool {
        self.tracks.get(frame.source).is_some_and(|track| {
            // A declared gap or resume already sealed the old stretch. Its
            // next frame owns a fresh clock without counting it twice.
            if track.native_sequence.is_none() {
                return false;
            }
            track
                .native_sequence
                .is_some_and(|previous| previous.checked_add(1) != Some(frame.native_sequence))
                || track
                    .native_stream
                    .as_deref()
                    .is_some_and(|previous| previous != frame.stream)
                || track
                    .last_monotonic_ns
                    .is_some_and(|previous| frame.monotonic_ns < previous)
        })
    }

    /// Appends a checked frame and seals the track once it holds a second.
    pub(super) fn append(&mut self, frame: &TrackFrame<'_>, now_ms: i64) -> Result<Vec<Segment>> {
        let recording_id = self
            .recording_id
            .clone()
            .context("Recording identity missing")?;
        let epoch = self.capture_epoch_id.clone();
        let track = self
            .tracks
            .entry(frame.source.into())
            .or_insert_with(|| Track::new(frame.rate, frame.channels));
        if track.native_stream.as_deref() != Some(frame.stream) || track.clock.is_none() {
            track.clock = Some(ClockAnchor {
                wall_ms: now_ms,
                monotonic_ns: frame.monotonic_ns,
            });
        }
        track.native_sequence = Some(frame.native_sequence);
        track.native_stream = Some(frame.stream.into());
        track.last_monotonic_ns = Some(frame.monotonic_ns);
        if track.pcm.is_empty() {
            track.start_ms = track
                .clock
                .map_or(now_ms, |clock| clock.wall_at(frame.monotonic_ns, now_ms));
            track.monotonic_ns = frame.monotonic_ns;
        }
        track.pcm.extend_from_slice(frame.pcm);
        if track.pcm.len() >= track.rate as usize * usize::from(track.channels) * 2 {
            return Ok(vec![seal(
                &recording_id,
                epoch.as_deref(),
                frame.source,
                track,
            )?]);
        }
        Ok(Vec::new())
    }

    /// A track's chunk counter, set by a test to reach its exhaustion.
    #[cfg(test)]
    pub(crate) fn set_sequence(&mut self, source: &str, sequence: u64) -> Result<()> {
        self.tracks
            .get_mut(source)
            .context("track absent")?
            .sequence = sequence;
        Ok(())
    }

    /// The PCM a track holds and has not sealed yet.
    #[cfg(test)]
    pub(crate) fn pending_pcm(&self, source: &str) -> Option<&[u8]> {
        self.tracks.get(source).map(|track| track.pcm.as_slice())
    }

    /// The sample rate a recording track was opened with, if any.
    pub fn track_rate(&self, source: &str) -> Option<u32> {
        self.tracks.get(source).map(|track| track.rate)
    }

    /// A producer declared a hole: the tail is sealed so the missing time
    /// never becomes contiguous PCM; the producer clock and sequence re-anchor.
    pub(super) fn restart_source(&mut self, source: &str) -> Result<Vec<Segment>> {
        let mut segments = Vec::new();
        if let (Some(id), Some(track)) = (self.recording_id.as_deref(), self.tracks.get_mut(source))
        {
            if !track.pcm.is_empty() {
                segments.push(seal(id, self.capture_epoch_id.as_deref(), source, track)?);
            }
            track.forget_producer();
        }
        Ok(segments)
    }

    /// End one recording source after native stop acknowledgment. Other
    /// sources can continue draining.
    pub fn pause_source(&mut self, source: &str) -> Result<Vec<Segment>> {
        self.recording_sources.remove(source);
        let mut result = Vec::new();
        if let (Some(id), Some(track)) = (self.recording_id.as_deref(), self.tracks.get_mut(source))
        {
            if !track.pcm.is_empty() {
                result.push(seal(id, self.capture_epoch_id.as_deref(), source, track)?);
            }
            track.forget_producer();
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
        if end && !self.has_unsealed() {
            self.recording_id = None;
            self.capture_epoch_id = None;
            self.tracks.clear();
        }
        segments
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
