use super::*;
use serde_json::json;

fn frame(source: &str, stream: &str, sequence: u64, pcm: &[u8]) -> Value {
    json!({"source":source,"stream_id":stream,"sample_rate":8000,
        "channels":1,"frames":pcm.len()/2,"sequence":sequence,
        "monotonic_ns":1_000_000+sequence*1_000_000,
        "pcm_s16le_base64":STANDARD.encode(pcm)})
}

#[test]
fn observed_zero_pcm_is_preserved_byte_exactly_through_tail_flush() -> Result<()> {
    let mut buffer = CaptureBuffer::default();
    buffer.begin_recording_with_epoch("meeting".into(), "epoch-one".into())?;
    let first = vec![0; 8_000];
    let tail = vec![0; 4_000];
    for (sequence, pcm) in [(0, &first), (1, &tail)] {
        let observed = buffer.ingest_observed(&frame("mic_local", "mic", sequence, pcm), 42)?;
        assert!(observed.segments.is_empty());
        assert_eq!(
            Some(observed.signal.context("signal absent")?.level),
            measure_s16le(pcm)
        );
    }
    let mut segments = buffer.flush(true);
    assert_eq!(segments.len(), 1);
    let segment = segments.remove(0);
    assert_eq!(segment.pcm, [first, tail].concat());
    assert_eq!(segment.metadata.frames, 6_000);
    assert_eq!(segment.metadata.first_sample, 0);
    assert_eq!(segment.metadata.sequence, 0);
    assert_eq!(segment.metadata.monotonic_ns, 1_000_000);
    assert_eq!(segment.metadata.started_at_ms, 42);
    assert_eq!(
        segment.metadata.capture_epoch_id.as_deref(),
        Some("epoch-one")
    );
    Ok(())
}

#[test]
fn observation_identifies_the_exact_source_stream_rate_and_decoded_pcm() -> Result<()> {
    let mut buffer = CaptureBuffer::default();
    buffer.begin_recording("meeting".into())?;
    let pcm = [i16::MIN, 0, 1, -1, i16::MAX]
        .into_iter()
        .flat_map(i16::to_le_bytes)
        .collect::<Vec<_>>();
    let long_stream = "é".repeat(64); // Exactly 128 UTF-8 bytes, not 128 characters.
    for (source, stream) in [
        ("mic_local", long_stream.as_str()),
        ("remote_app", "Remote-ID"),
    ] {
        let mut input = frame(source, stream, 0, &pcm);
        input["levels"] = json!({"rms_dbfs":-120,"peak_dbfs":-120,"clipped_frames":0});
        let observed = buffer.ingest_observed(&input, 1)?;
        assert!(observed.segments.is_empty());
        let signal = observed.signal.context("signal absent")?;
        assert_eq!(signal.source, source);
        assert_eq!(signal.stream_id.as_bytes(), stream.as_bytes());
        assert_eq!(signal.sample_rate, 8_000);
        assert_eq!(Some(signal.level), measure_s16le(&pcm));
    }
    let segments = buffer.flush(true);
    assert_eq!(segments.len(), 2);
    assert!(segments.iter().all(|segment| segment.pcm == pcm));
    Ok(())
}

#[test]
fn invalid_original_never_returns_an_observation_or_changes_accepted_bytes() -> Result<()> {
    for (field, invalid) in [
        ("pcm_s16le_base64", json!("%%%")),
        ("sample_rate", json!(48_000)),
        ("channels", json!(2)),
        ("sequence", json!(3)),
        ("stream_id", json!("other-stream")),
        ("stream_id", json!("x".repeat(129))),
        ("monotonic_ns", json!(0)),
        ("source", json!("unknown")),
        ("capture_epoch_id", json!("../invalid")),
    ] {
        let mut buffer = CaptureBuffer::default();
        buffer.begin_recording("meeting".into())?;
        let pcm = vec![0; 200];
        buffer.ingest_observed(&frame("mic_local", "mic", 0, &pcm), 1)?;
        let mut invalid_frame = frame("mic_local", "mic", 1, &pcm);
        invalid_frame[field] = invalid;
        assert!(
            buffer.ingest_observed(&invalid_frame, 2).is_err(),
            "{field}"
        );
        let segments = buffer.flush(true);
        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0].pcm, pcm, "{field}");
        assert_eq!(segments[0].metadata.frames, 100);
    }
    Ok(())
}

#[test]
fn absent_original_owner_and_voice_only_audio_have_no_signal_observation() -> Result<()> {
    let mut buffer = CaptureBuffer::default();
    let pcm = vec![0; 200];
    let input = frame("mic_local", "mic", 0, &pcm);
    let unowned = buffer.ingest_observed(&input, 1)?;
    assert!(unowned.signal.is_none());
    assert!(unowned.segments.is_empty());
    assert!(!buffer.has_pending_audio());
    buffer.begin_voice()?;
    let voice_only = buffer.ingest_observed(&input, 2)?;
    assert!(voice_only.signal.is_none());
    assert!(voice_only.segments.is_empty());
    assert_eq!(buffer.voice.as_ref().context("voice absent")?.pcm, pcm);
    assert!(buffer.flush(true).is_empty());
    Ok(())
}

#[test]
fn derived_voice_validation_must_succeed_before_an_original_observation_returns() -> Result<()> {
    let mut buffer = CaptureBuffer::default();
    buffer.begin_recording_with_epoch("meeting".into(), "recording-epoch".into())?;
    buffer.begin_voice()?;
    let pcm = vec![0; 200];
    let mut first = frame("mic_local", "mic", 0, &pcm);
    first["capture_epoch_id"] = json!("producer-one");
    assert!(buffer.ingest_observed(&first, 1)?.signal.is_some());
    let mut changed = frame("mic_local", "mic", 1, &pcm);
    changed["capture_epoch_id"] = json!("producer-two");
    assert!(buffer.ingest_observed(&changed, 2).is_err());
    assert_eq!(buffer.voice.as_ref().context("voice absent")?.pcm, pcm);
    let segments = buffer.flush(true);
    assert_eq!(segments.len(), 1);
    assert_eq!(segments[0].pcm, pcm);
    assert_eq!(segments[0].metadata.frames, 100);
    Ok(())
}

#[test]
fn failed_seal_returns_no_observation_and_retains_the_existing_pending_pcm() -> Result<()> {
    let mut buffer = CaptureBuffer::default();
    buffer.begin_recording("meeting".into())?;
    let first = vec![0; 200];
    buffer.ingest_observed(&frame("mic_local", "mic", 0, &first), 1)?;
    // Exhaustion is synthetic. Preserve the existing seal failure semantics:
    // valid bytes already appended stay pending, but there is no success result.
    buffer
        .tracks
        .get_mut("mic_local")
        .context("track absent")?
        .sequence = u64::MAX;
    let tail = vec![1; 15_800];
    assert!(
        buffer
            .ingest_observed(&frame("mic_local", "mic", 1, &tail), 2)
            .is_err()
    );
    assert!(buffer.has_pending_audio());
    assert_eq!(
        buffer.tracks.get("mic_local").context("track absent")?.pcm,
        [first, tail].concat()
    );
    Ok(())
}

#[test]
fn paused_mic_keeps_voice_bytes_without_observation_while_remote_can_continue() -> Result<()> {
    let mut buffer = CaptureBuffer::default();
    buffer.begin_recording("meeting".into())?;
    buffer.begin_voice()?;
    let pcm = vec![0; 200];
    buffer.ingest_observed(&frame("mic_local", "mic", 0, &pcm), 1)?;
    let paused = buffer.pause_source("mic_local")?;
    assert_eq!(paused[0].pcm, pcm);
    let voice_only = buffer.ingest_observed(&frame("mic_local", "mic", 1, &pcm), 2)?;
    assert!(voice_only.signal.is_none());
    assert!(voice_only.segments.is_empty());
    let remote = buffer.ingest_observed(&frame("remote_app", "remote", 0, &pcm), 3)?;
    assert_eq!(
        remote.signal.context("remote signal absent")?.source,
        "remote_app"
    );
    assert_eq!(
        buffer.voice.as_ref().context("voice absent")?.pcm.len(),
        400
    );
    let remaining = buffer.flush(true);
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].metadata.track, "remote_app");
    assert_eq!(remaining[0].pcm, pcm);
    Ok(())
}

#[test]
fn preroll_updates_only_derived_voice_not_original_or_signal() -> Result<()> {
    let mut buffer = CaptureBuffer::default();
    buffer.begin_recording("call-original".into())?;
    let pcm = vec![0; 200];
    let first = frame("mic_local", "mic", 0, &pcm);
    assert!(buffer.ingest_observed(&first, 1)?.signal.is_some());
    buffer.begin_voice()?;
    buffer.ingest_voice_frame(&first, 2)?;
    let second = frame("mic_local", "mic", 1, &pcm);
    assert!(buffer.ingest_observed(&second, 3)?.signal.is_some());
    assert_eq!(
        buffer.voice.as_ref().context("voice absent")?.pcm.len(),
        400
    );
    let segments = buffer.flush(true);
    assert_eq!(segments.len(), 1);
    assert_eq!(segments[0].metadata.frames, 200);
    assert_eq!(segments[0].pcm, [pcm.clone(), pcm.clone()].concat());

    // Exercise the exact internal branch used by the public preroll wrapper.
    buffer.voice = None;
    buffer.begin_voice()?;
    let replay = buffer.ingest_selected(&first, 4, false)?;
    assert!(replay.signal.is_none());
    assert!(replay.segments.is_empty());
    assert_eq!(buffer.voice.as_ref().context("voice absent")?.pcm, pcm);
    Ok(())
}
