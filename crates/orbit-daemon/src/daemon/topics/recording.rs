use super::{ProjectTopic, TopicSource};
use orbit_core::{
    Connection,
    refusal::Refusal,
    topics::{
        Knowledge, Pending, TopicData,
        recording::{Application, Value},
    },
};

pub(super) struct Projector;

impl ProjectTopic for Projector {
    fn project(&self, source: &TopicSource<'_>) -> TopicData {
        let state = source.state;
        let applications = if state.privacy_stopped() {
            Knowledge::Absent {
                refusal: Refusal::PrivacyStopped { since_ms: None },
            }
        } else if !state.helper_connected() {
            Knowledge::Absent {
                refusal: Refusal::HelperNotRunning,
            }
        } else if let Some(cause) = &state.views.audio_sources_unavailable {
            Knowledge::Absent {
                refusal: cause.clone(),
            }
        } else if source
            .runtime
            .is_some_and(|runtime| runtime.features.meeting.audio_source_observed)
        {
            Knowledge::Present {
                value: state
                    .views
                    .audio_sources
                    .iter()
                    .map(|source| Application {
                        name: source.application_name.clone(),
                        pid: source.application_pid,
                    })
                    .collect(),
                as_of_ms: None,
            }
        } else {
            Knowledge::Unknown {
                why: Pending::NotObservedYet,
            }
        };
        let recording_signal = source.runtime.map_or_else(
            || state.views.recording_signal.clone(),
            |runtime| runtime.recording_signal_view(),
        );
        TopicData::Recording(Knowledge::Present {
            value: Value {
                recording: state.durable.recording.clone().map(Box::new),
                recording_signal: recording_signal.map(Box::new),
                pending_jobs: state.durable.pending_jobs,
                offline: !matches!(state.connection(), Connection::Online),
                call: source.runtime.map(|runtime| Box::new(runtime.call_topic())),
                applications,
                question: source
                    .runtime
                    .and_then(|runtime| runtime.features.question.current.clone())
                    .map(Box::new),
                voice_trigger: source.runtime.and_then(|runtime| runtime.voice_status()),
            },
            as_of_ms: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::topics::TopicBoard;
    use orbit_core::refusal::Resource;
    use orbit_core::{
        CapturePhase, Recording, State,
        audio_signal::{RecordingSignalView, SignalCondition, TrackSignalView},
        topics::TopicId,
    };
    use tokio::sync::watch;

    fn starting_state() -> State {
        let mut state = State::default();
        state.durable.recording = Some(Recording {
            id: "recording-1".into(),
            title: "Prueba".into(),
            started_at_ms: 10,
            ended_at_ms: None,
            expires_at_ms: 60_010,
            application_pid: None,
            audio: orbit_core::protocol::MeetingAudio::System,
            phase: CapturePhase::Starting,
            sealed_chunks: 0,
            gaps: 0,
            tracks: Vec::new(),
            finalizing_since_ms: None,
            ceiling_notice_given: false,
        });
        state
    }

    #[test]
    fn starting_is_published_by_the_daemon_topic() -> Result<(), serde_json::Error> {
        let state = starting_state();
        let projected = Projector.project(&TopicSource {
            state: &state,
            runtime: None,
            now_ms: 0,
        });
        let json = serde_json::to_value(projected)?;
        assert_eq!(json["knowledge"]["state"], "present");
        assert_eq!(json["knowledge"]["value"]["recording"]["phase"], "starting");
        Ok(())
    }

    #[test]
    fn unverifiable_inventory_is_absent_with_its_numbers() -> Result<(), serde_json::Error> {
        let mut state = starting_state();
        {
            let fixture_value = serde_json::json!(true);
            crate::daemon::authority_fixture::set(
                &mut state,
                "capabilities.native_helper",
                fixture_value,
            );
        }
        state.views.audio_sources_unavailable = Some(Refusal::TooMany {
            resource: Resource::Source,
            count: 33,
            max: 32,
        });
        let projected = Projector.project(&TopicSource {
            state: &state,
            runtime: None,
            now_ms: 0,
        });
        let json = serde_json::to_value(projected)?;
        assert_eq!(json["knowledge"]["state"], "present");
        let refusal = &json["knowledge"]["value"]["applications"]["refusal"];
        assert_eq!(refusal["code"], "too_many");
        assert_eq!(refusal["count"], 33);
        assert_eq!(refusal["max"], 32);
        Ok(())
    }

    #[test]
    fn failed_inventory_keeps_a_live_recording_and_its_signal() {
        let mut state = starting_state();
        {
            let fixture_value = serde_json::json!(true);
            crate::daemon::authority_fixture::set(
                &mut state,
                "capabilities.native_helper",
                fixture_value,
            );
        }
        state.views.audio_sources_unavailable = Some(Refusal::ReadInterrupted {
            cause: orbit_core::refusal::ReadInterruption::InventoryDispatch {
                sent_reads: 2,
                total_reads: 2,
            },
        });
        state.views.recording_signal = Some(RecordingSignalView {
            recording_id: "recording-1".into(),
            capture_epoch_id: "epoch-1".into(),
            mic_local: TrackSignalView {
                condition: SignalCondition::Receiving,
                received_frames: 42,
                since_last_samples_ms: Some(0),
                low_signal_ms: 0,
            },
            remote_app: None,
        });
        let projected = Projector.project(&TopicSource {
            state: &state,
            runtime: None,
            now_ms: 0,
        });
        let TopicData::Recording(Knowledge::Present { value, .. }) = projected else {
            panic!("the inventory failure hid the recording");
        };
        assert_eq!(
            value.recording.as_deref().map(|item| &item.phase),
            Some(&CapturePhase::Starting)
        );
        assert_eq!(
            value
                .recording_signal
                .as_deref()
                .map(|signal| signal.mic_local.received_frames),
            Some(42)
        );
        assert!(matches!(value.applications, Knowledge::Absent { .. }));
    }

    #[test]
    fn missing_helper_privacy_and_unobserved_inventory_have_distinct_causes() {
        let mut state = starting_state();
        let project = |state: &State| {
            Projector.project(&TopicSource {
                state,
                runtime: None,
                now_ms: 0,
            })
        };
        let TopicData::Recording(Knowledge::Present { value, .. }) = project(&state) else {
            unreachable!("the recording exists even without the helper");
        };
        assert!(matches!(
            value.applications,
            Knowledge::Absent {
                refusal: Refusal::HelperNotRunning
            }
        ));

        {
            let fixture_value = serde_json::json!(true);
            crate::daemon::authority_fixture::set(&mut state, "privacy_stopped", fixture_value);
        }
        let TopicData::Recording(Knowledge::Present { value, .. }) = project(&state) else {
            unreachable!("the recording exists even when the user stops privacy");
        };
        assert!(matches!(
            value.applications,
            Knowledge::Absent {
                refusal: Refusal::PrivacyStopped { .. }
            }
        ));

        {
            let fixture_value = serde_json::json!(false);
            crate::daemon::authority_fixture::set(&mut state, "privacy_stopped", fixture_value);
        }
        {
            let fixture_value = serde_json::json!(true);
            crate::daemon::authority_fixture::set(
                &mut state,
                "capabilities.native_helper",
                fixture_value,
            );
        }
        let TopicData::Recording(Knowledge::Present { value, .. }) = project(&state) else {
            unreachable!("the recording exists while it waits for the inventory");
        };
        assert!(matches!(
            value.applications,
            Knowledge::Unknown {
                why: Pending::NotObservedYet
            }
        ));
    }

    #[test]
    fn a_thousand_changing_audio_observations_publish_at_most_four_frames() {
        let mut state = starting_state();
        let track = TrackSignalView {
            condition: SignalCondition::Receiving,
            received_frames: 0,
            since_last_samples_ms: Some(0),
            low_signal_ms: 0,
        };
        state.views.recording_signal = Some(RecordingSignalView {
            recording_id: "recording-1".into(),
            capture_epoch_id: "epoch-1".into(),
            mic_local: track.clone(),
            remote_app: Some(track),
        });
        let mut board = TopicBoard::new("synthetic-epoch".into());
        let channels = board.channels();
        let receiver = watch::Sender::subscribe(&channels[&TopicId::Recording]);
        board.publish(&TopicSource {
            state: &state,
            runtime: None,
            now_ms: 0,
        });
        let first = receiver.borrow().clone();
        assert_eq!(first.revision, 1);
        assert!(matches!(
            first.data,
            TopicData::Recording(orbit_core::topics::Knowledge::Present { .. })
        ));

        for frame in 0..1_000 {
            if let Some(signal) = state.views.recording_signal.as_mut() {
                signal.mic_local.received_frames = frame + 1;
                if let Some(remote) = signal.remote_app.as_mut() {
                    remote.received_frames = frame + 1;
                    remote.condition = if frame % 2 == 0 {
                        SignalCondition::LowSignal
                    } else {
                        SignalCondition::Receiving
                    };
                }
            }
            board.state_changed();
            board.publish(&TopicSource {
                state: &state,
                runtime: None,
                now_ms: frame,
            });
        }
        let last = receiver.borrow().clone();
        assert!(
            last.revision > first.revision,
            "changed values must be published"
        );
        assert!(last.revision <= 4, "{} frames in one second", last.revision);
        eprintln!("published_frames_in_1000_ms={}", last.revision);
        let TopicData::Recording(Knowledge::Present { value, .. }) = last.data else {
            unreachable!("the topic lost the recording levels");
        };
        let Some(signal) = value.recording_signal.as_deref() else {
            unreachable!("the received signal vanished from the topic");
        };
        let Some(remote) = signal.remote_app.as_ref() else {
            unreachable!("the second track is missing");
        };
        assert!(signal.mic_local.received_frames > 0);
        assert_eq!(remote.received_frames, signal.mic_local.received_frames);
        assert_eq!(signal.mic_local.condition, SignalCondition::Receiving);
        assert_eq!(remote.condition, SignalCondition::LowSignal);
    }
}
