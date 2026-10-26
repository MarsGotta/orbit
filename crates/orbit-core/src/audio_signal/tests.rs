use super::*;

fn pcm(values: &[i16]) -> Vec<u8> {
    values
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect()
}

fn signal(value: i16, frames: usize) -> PcmSignal {
    let bytes: Vec<_> = std::iter::repeat_n(value.to_le_bytes(), frames)
        .flatten()
        .collect();
    measure_s16le(&bytes).unwrap_or_else(|| panic!("valid bounded fixture"))
}

#[test]
fn measurement_rejects_empty_odd_and_oversized_pcm() {
    for bytes in [
        vec![],
        vec![0],
        vec![0; MAX_PCM_BYTES + 1],
        vec![0; MAX_PCM_BYTES + 2],
    ] {
        assert_eq!(measure_s16le(&bytes), None);
    }
    assert_eq!(signal(0, MAX_PCM_BYTES / 2).frames(), 192_000);
}

#[test]
fn extrema_and_maximum_buffer_do_not_overflow() {
    let measured =
        measure_s16le(&pcm(&[i16::MIN, i16::MAX])).unwrap_or_else(|| panic!("valid extrema"));
    assert_eq!(measured.frames(), 2);
    assert_eq!(measured.sum_squared_amplitudes(), 2_147_418_113);
    assert!(!measured.is_low_signal());
    assert_eq!(
        signal(i16::MIN, 192_000).sum_squared_amplitudes(),
        206_158_430_208_000
    );
}

#[test]
fn rms_threshold_is_inclusive_exact_and_not_mean_absolute_amplitude() {
    for values in [vec![0], vec![32, -32], vec![0, 45]] {
        assert!(measure_s16le(&pcm(&values)).is_some_and(PcmSignal::is_low_signal));
    }
    for values in [vec![33], vec![-33], vec![32, 33], vec![0, 46]] {
        assert!(measure_s16le(&pcm(&values)).is_some_and(|level| !level.is_low_signal()));
    }
}

#[test]
fn no_samples_wait_then_delay_without_claiming_low_signal() {
    let monitor = TrackSignalMonitor::new(1_000);
    for now in [1_000, 3_999] {
        assert_eq!(
            monitor.view(now),
            TrackSignalView {
                condition: SignalCondition::AwaitingSamples,
                received_frames: 0,
                since_last_samples_ms: None,
                low_signal_ms: 0,
                speech_quiet_ms: None,
                voiced_frames: None,
            }
        );
    }
    assert_eq!(
        monitor.view(4_000).condition,
        SignalCondition::DeliveryDelayed
    );
    assert_eq!(monitor.view(4_000).low_signal_ms, 0);
    assert_eq!(monitor.view(4_000).since_last_samples_ms, None);
}

#[test]
fn ten_seconds_counts_samples_not_receiver_wait_time() {
    let mut monitor = TrackSignalMonitor::new(0);
    let low = signal(32, 8_000);
    for index in 0..9 {
        assert!(monitor.observe(low, 8_000, index * 1_000));
    }
    assert_eq!(monitor.view(8_000).condition, SignalCondition::Receiving);
    assert_eq!(monitor.view(8_000).low_signal_ms, 9_000);
    // Waiting cannot turn nine seconds of samples into ten.
    assert_eq!(monitor.view(10_999).low_signal_ms, 9_000);
    assert!(monitor.observe(low, 8_000, 9_000));
    assert_eq!(monitor.view(9_000).condition, SignalCondition::LowSignal);
    assert_eq!(monitor.view(9_000).low_signal_ms, LOW_SIGNAL_THRESHOLD_MS);
    assert_eq!(monitor.view(9_000).received_frames, 80_000);
}

#[test]
fn delayed_delivery_has_priority_and_breaks_the_low_run() {
    let mut monitor = TrackSignalMonitor::new(0);
    let low = signal(0, 8_000);
    for index in 0..10 {
        assert!(monitor.observe(low, 8_000, index * 1_000));
    }
    assert_eq!(monitor.view(11_999).condition, SignalCondition::LowSignal);
    let delayed = monitor.view(12_000);
    assert_eq!(delayed.condition, SignalCondition::DeliveryDelayed);
    assert_eq!(delayed.since_last_samples_ms, Some(3_000));
    assert_eq!(delayed.low_signal_ms, 0);
    assert!(monitor.observe(low, 8_000, 12_000));
    assert_eq!(monitor.view(12_000).condition, SignalCondition::Receiving);
    assert_eq!(monitor.view(12_000).low_signal_ms, 1_000);
    assert_eq!(monitor.view(12_000).received_frames, 88_000);
}

#[test]
fn a_gap_below_the_deadline_keeps_the_low_run() {
    let mut monitor = TrackSignalMonitor::new(0);
    assert!(monitor.observe(signal(0, 8_000), 8_000, 0));
    assert!(monitor.observe(signal(0, 8_000), 8_000, 2_999));
    assert_eq!(monitor.view(2_999).low_signal_ms, 2_000);
}

#[test]
fn recovered_energy_resets_low_signal_immediately() {
    let mut monitor = TrackSignalMonitor::new(0);
    for index in 0..10 {
        assert!(monitor.observe(signal(0, 8_000), 8_000, index * 1_000));
    }
    assert!(monitor.observe(signal(33, 8_000), 8_000, 10_000));
    assert_eq!(monitor.view(10_000).condition, SignalCondition::Receiving);
    assert_eq!(monitor.view(10_000).low_signal_ms, 0);
    assert!(monitor.observe(signal(0, 8_000), 8_000, 11_000));
    assert_eq!(monitor.view(11_000).low_signal_ms, 1_000);
}

#[test]
fn format_change_resets_low_duration_but_not_accepted_frames() {
    let mut monitor = TrackSignalMonitor::new(0);
    assert!(monitor.observe(signal(0, 8_000), 8_000, 0));
    assert!(monitor.observe(signal(0, 16_000), 16_000, 1_000));
    assert_eq!(monitor.view(1_000).low_signal_ms, 1_000);
    assert_eq!(monitor.view(1_000).received_frames, 24_000);
}

#[test]
fn fractional_packet_durations_accumulate_without_per_packet_rounding() {
    let mut monitor = TrackSignalMonitor::new(0);
    let sample = signal(0, 1);
    for _ in 0..8_000 {
        assert!(monitor.observe(sample, 8_000, 0));
    }
    assert_eq!(monitor.view(0).low_signal_ms, 1_000);
}

#[test]
fn invalid_rate_duration_and_regressive_time_leave_history_unchanged() {
    let mut monitor = TrackSignalMonitor::new(1_000);
    let before = monitor.clone();
    assert!(!monitor.observe(signal(0, 8_000), 8_000, 999));
    assert_eq!(monitor, before);
    assert!(monitor.observe(signal(0, 8_000), 8_000, 2_000));
    let before = monitor.clone();
    for (level, rate, received) in [
        (signal(0, 1), 7_999, 2_001),
        (signal(0, 1), 192_001, 2_001),
        (signal(0, 8_001), 8_000, 2_001),
        (signal(0, 16_000), 16_000, 1_999),
    ] {
        assert!(!monitor.observe(level, rate, received));
        assert_eq!(monitor, before);
    }
    assert!(monitor.observe(signal(0, 192_000), 192_000, 2_000));
}

#[test]
fn regressive_views_or_a_future_observation_return_unknown_without_mutation() {
    let mut monitor = TrackSignalMonitor::new(1_000);
    assert_eq!(monitor.view(999).condition, SignalCondition::Unknown);
    assert!(monitor.observe(signal(0, 8_000), 8_000, 5_000));
    let before = monitor.clone();
    let unknown = monitor.view(4_999);
    assert_eq!(unknown.condition, SignalCondition::Unknown);
    assert_eq!(unknown.since_last_samples_ms, None);
    assert_eq!(unknown.low_signal_ms, 0);
    assert_eq!(unknown.received_frames, 8_000);
    assert_eq!(monitor, before);
    assert_eq!(monitor.view(5_000).condition, SignalCondition::Receiving);
}

#[test]
fn counters_and_clock_edges_saturate_without_overflow() {
    let mut monitor = TrackSignalMonitor::new(u64::MAX - 1);
    assert!(monitor.observe(signal(0, 8_000), 8_000, u64::MAX - 1));
    monitor.received_frames = u64::MAX - 1;
    monitor.low_frames = u64::MAX - 1;
    assert!(monitor.observe(signal(0, 8_000), 8_000, u64::MAX));
    let view = monitor.view(u64::MAX);
    assert_eq!(view.received_frames, u64::MAX);
    assert_eq!(view.low_signal_ms, u64::MAX / 8);
    assert_eq!(view.since_last_samples_ms, Some(0));
    assert_eq!(view.condition, SignalCondition::LowSignal);
}

#[test]
fn measurements_do_not_modify_or_retain_original_pcm() {
    let bytes = pcm(&[0, -32, 32, i16::MIN, i16::MAX]);
    let original = bytes.clone();
    let level = measure_s16le(&bytes).unwrap_or_else(|| panic!("valid original"));
    assert_eq!(bytes, original);
    drop(bytes);
    let mut monitor = TrackSignalMonitor::new(0);
    assert!(monitor.observe(level, 8_000, 0));
    assert_eq!(monitor.view(0).received_frames, 5);
    assert!(!std::mem::needs_drop::<PcmSignal>());
    assert!(!std::mem::needs_drop::<TrackSignalMonitor>());
    assert!(std::mem::size_of::<PcmSignal>() <= 16);
    assert!(std::mem::size_of::<TrackSignalMonitor>() <= 64);
}

#[test]
fn views_round_trip_with_snake_case_conditions_and_an_absent_unrequested_track() {
    let view = RecordingSignalView {
        recording_id: "recording-owned-by-adapter".into(),
        capture_epoch_id: "epoch-owned-by-adapter".into(),
        mic_local: TrackSignalMonitor::new(0).view(3_000),
        remote_app: None,
    };
    let encoded = serde_json::to_value(&view).unwrap_or_else(|error| panic!("{error}"));
    assert_eq!(encoded["mic_local"]["condition"], "delivery_delayed");
    assert_eq!(encoded["remote_app"], serde_json::Value::Null);
    let decoded: RecordingSignalView =
        serde_json::from_value(encoded).unwrap_or_else(|error| panic!("{error}"));
    assert_eq!(decoded, view);
}

#[test]
fn room_noise_is_quiet_for_speech_but_not_a_low_signal_warning() {
    let mut monitor = TrackSignalMonitor::new(0);
    for second in 1..=120 {
        assert!(monitor.observe(signal(250, 8_000), 8_000, second * 1_000));
    }
    let room = monitor.view(120_000);
    assert_eq!(room.condition, SignalCondition::Receiving);
    assert_eq!(room.low_signal_ms, 0);
    assert_eq!(room.speech_quiet_ms, Some(120_000));
    for (index, voice) in [500, 900].into_iter().enumerate() {
        let at = 121_000 + index as u64 * 2_000;
        assert!(monitor.observe(signal(voice, 8_000), 8_000, at));
        assert_eq!(monitor.view(at).speech_quiet_ms, Some(0));
        assert!(monitor.observe(signal(499, 8_000), 8_000, at + 1_000));
        assert_eq!(monitor.view(at + 1_000).speech_quiet_ms, Some(1_000));
    }
    assert_eq!(monitor.view(127_000).speech_quiet_ms, None);
    assert!(monitor.observe(signal(250, 8_000), 8_000, 127_000));
    assert_eq!(monitor.view(127_000).speech_quiet_ms, Some(1_000));
}
