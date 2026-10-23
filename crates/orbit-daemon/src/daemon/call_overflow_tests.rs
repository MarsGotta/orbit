use super::*;

#[tokio::test]
async fn emergency_queue_overflow_marks_the_continuous_original_incomplete() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _receiver) = crate::daemon::audio::tests::fixture(directory.path())?;
    {
        let fixture_value = serde_json::json!(true);
        crate::daemon::authority_fixture::set(
            &mut runtime.state,
            "capabilities.voice_processing_supported",
            fixture_value,
        );
    }
    runtime
        .apply(Event::CallStart {
            id: "overflow-call".into(),
            duration_ms: 60_000,
            origin: orbit_core::CallOrigin::App,
        })
        .await?;
    assert!(runtime.state.durable.capture.recording.is_none());
    let metadata = orbit_store::ChunkMetadata {
        capture_epoch_id: None,
        id: "overflow-call-mic-0".into(),
        recording_id: "overflow-call".into(),
        track: "mic_local".into(),
        sequence: 0,
        sample_rate: 8000,
        channels: 1,
        frames: 1,
        monotonic_ns: 1,
        first_sample: 0,
        started_at_ms: 1,
    };
    // Reserve queue slots only: this test does not claim to persist these
    // synthetic entries, nor to exercise physical-memory exhaustion.
    for _ in 0..4096 {
        runtime
            .capture
            .pending_seals
            .push_back(crate::capture::Segment {
                metadata: metadata.clone(),
                pcm: vec![1; 2],
            });
    }
    assert!(
        runtime
            .seal(vec![crate::capture::Segment {
                metadata,
                pcm: vec![2; 2]
            }])
            .is_err()
    );
    assert_eq!(runtime.capture.pending_seals.len(), 4096);
    assert_eq!(
        runtime
            .capture
            .call_runtime
            .as_ref()
            .context("call absent")?
            .gaps,
        1
    );
    assert!(crate::daemon::turn::said::has(
        &runtime.state,
        orbit_core::notice::NoticeCode::AudioReserveExhausted
    ));
    Ok(())
}
