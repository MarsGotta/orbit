use super::*;

pub(crate) fn project_meeting_topic(
    value: &orbit_core::topics::recording::Value,
) -> MeetingSnapshot {
    use orbit_core::{refusal::Refusal, topics::Knowledge};

    let mut meeting = project_meeting_parts(
        value.recording.as_deref(),
        value.recording_signal.as_deref(),
        value.pending_jobs,
        value.offline,
    );
    if let Some(call) = value.call.as_deref() {
        meeting.call_preference = call.preference;
        meeting.call_preference_unreadable = call.preference_unreadable.clone();
        meeting.call_panel = call.panel.clone().map(call_panel);
    }
    // «Preguntar a Deimos» while a recording is in progress, the
    // question the panel follows and the «Oye Deimos» detector.
    if matches!(
        meeting.phase,
        MeetingPhase::Recording | MeetingPhase::Paused
    ) {
        meeting.ask = Some(meeting_ask());
    }
    meeting.question = value.question.as_deref().map(meeting_question);
    meeting.voice_trigger = value.voice_trigger.clone().map(voice_trigger);
    meeting.microphone_muted = value.microphone_muted;
    match &value.applications {
        Knowledge::Present { value, .. } => {
            meeting.applications = value
                .iter()
                .map(|application| MeetingApplicationSnapshot {
                    name: application.name.clone(),
                    pid: application.pid,
                })
                .collect();
            meeting.applications_state = if meeting.applications.is_empty() {
                MeetingApplicationsState::Empty
            } else {
                MeetingApplicationsState::Ready
            };
            meeting.applications_detail = None;
        }
        Knowledge::Absent { refusal } => {
            meeting.applications.clear();
            meeting.applications_state = if matches!(refusal, Refusal::PrivacyStopped { .. }) {
                MeetingApplicationsState::PrivacyStopped
            } else if matches!(refusal, Refusal::HelperNotRunning) {
                MeetingApplicationsState::HelperMissing
            } else {
                MeetingApplicationsState::Unavailable
            };
            meeting.applications_detail = Some(refusal.to_string());
        }
        Knowledge::Unknown { why } => {
            meeting.applications.clear();
            meeting.applications_state = MeetingApplicationsState::Unavailable;
            meeting.applications_detail = Some(pending_inventory_detail(why));
        }
    }
    meeting
}

/// What «Preguntar a Deimos» offers, from orbit-core's constants.
pub(crate) fn meeting_ask() -> MeetingAskSnapshot {
    use orbit_core::meeting_question as core;
    MeetingAskSnapshot {
        windows: core::WINDOW_MINUTES
            .iter()
            .map(|minutes| MeetingWindowSnapshot { minutes: *minutes })
            .collect(),
        default_minutes: core::DEFAULT_WINDOW_MINUTES,
        max_question_bytes: u32::try_from(core::MAX_QUESTION_BYTES).unwrap_or(u32::MAX),
    }
}

/// The question as the panel shows it; the answer's text is joined from
/// the conversation by the recording feature.
pub(crate) fn meeting_question(
    question: &orbit_core::meeting_question::MeetingQuestion,
) -> MeetingQuestionSnapshot {
    use orbit_core::meeting_question::{QuestionOrigin, QuestionStage};
    MeetingQuestionSnapshot {
        id: question.id.clone(),
        question: question.question.clone(),
        minutes: question.minutes,
        origin: match question.origin {
            QuestionOrigin::Voice => MeetingQuestionOrigin::Voice,
            QuestionOrigin::Typed => MeetingQuestionOrigin::Typed,
            QuestionOrigin::Hotkey => MeetingQuestionOrigin::Hotkey,
        },
        stage: match question.stage {
            QuestionStage::Thinking => MeetingQuestionStage::Thinking,
            QuestionStage::Answered => MeetingQuestionStage::Answered,
            QuestionStage::Failed => MeetingQuestionStage::Failed,
        },
        answer: None,
        cause: question.cause.clone().map(question_cause),
        shown_until_ms: question.shown_until_ms(),
        sent: question.sent,
    }
}

fn question_cause(cause: orbit_core::meeting_question::QuestionCause) -> MeetingQuestionCause {
    use orbit_core::meeting_question::{BusyTurn, QuestionCause as Core};
    match cause {
        Core::PrivacyStopped => MeetingQuestionCause::PrivacyStopped,
        Core::NoMeeting => MeetingQuestionCause::NoMeeting,
        Core::HubUnreachable { connection } => MeetingQuestionCause::HubUnreachable {
            connection: match connection {
                orbit_core::Connection::Offline => MeetingQuestionLink::Offline,
                orbit_core::Connection::Connecting => MeetingQuestionLink::Connecting,
                orbit_core::Connection::Online => MeetingQuestionLink::Online,
                orbit_core::Connection::Revoked => MeetingQuestionLink::Revoked,
            },
        },
        Core::DeimosUnavailable { code } => MeetingQuestionCause::DeimosUnavailable { code },
        Core::TurnInFlight { turn } => MeetingQuestionCause::TurnInFlight {
            turn: match turn {
                BusyTurn::AssistantTurn => MeetingQuestionBusy::AssistantTurn,
                BusyTurn::PendingApproval => MeetingQuestionBusy::PendingApproval,
                BusyTurn::VoiceNote => MeetingQuestionBusy::VoiceNote,
                BusyTurn::VoiceTranscription => MeetingQuestionBusy::VoiceTranscription,
                BusyTurn::Call => MeetingQuestionBusy::Call,
            },
        },
        Core::QuestionEmpty => MeetingQuestionCause::QuestionEmpty,
        Core::QuestionTooLong { bytes, max_bytes } => {
            MeetingQuestionCause::QuestionTooLong { bytes, max_bytes }
        }
        Core::WindowNotOffered { minutes, offered } => {
            MeetingQuestionCause::WindowNotOffered { minutes, offered }
        }
        Core::TurnFailed => MeetingQuestionCause::TurnFailed,
        Core::Interrupted => MeetingQuestionCause::Interrupted,
        Core::HubRefused { code, message } => MeetingQuestionCause::HubRefused { code, message },
        Core::NoAnswer => MeetingQuestionCause::NoAnswer,
        Core::Revoked => MeetingQuestionCause::Revoked,
    }
}

fn voice_trigger(status: orbit_core::meeting_question::VoiceStatus) -> VoiceStatusSnapshot {
    use orbit_core::meeting_question::{NotListening as Off, VoiceStatus as Core};
    use orbit_core::wire::native::WakePhraseAbandonmentCause as Cause;
    match status {
        Core::Listening => VoiceStatusSnapshot::Listening,
        Core::Starting => VoiceStatusSnapshot::Starting,
        Core::Downloading { percent } => VoiceStatusSnapshot::Downloading { percent },
        Core::Abandoned { cause } => VoiceStatusSnapshot::Abandoned {
            cause: match cause {
                Cause::PhraseRevised => PhraseAbandonedCause::PhraseRevised,
                Cause::QuestionWaitExpired => PhraseAbandonedCause::QuestionWaitExpired,
                Cause::QuestionTooLong => PhraseAbandonedCause::QuestionTooLong,
            },
        },
        Core::NotListening { why } => VoiceStatusSnapshot::NotListening {
            why: match why {
                Off::PrivacyStopped => VoiceOffCause::PrivacyStopped,
                Off::Paused => VoiceOffCause::Paused,
                Off::MacosVersion { os_version } => VoiceOffCause::MacosVersion { os_version },
                Off::LocaleUnsupported => VoiceOffCause::LocaleUnsupported,
                Off::Unavailable { code } => VoiceOffCause::Unavailable { code },
                Off::Failed { code, detail } => VoiceOffCause::Failed { code, detail },
                Off::HelperDisconnected => VoiceOffCause::HelperDisconnected,
                Off::HelperWithoutPhrase => VoiceOffCause::HelperWithoutPhrase,
                Off::NoMicrophone => VoiceOffCause::NoMicrophone,
            },
        },
    }
}

fn call_panel(panel: orbit_core::topics::recording::CallPanel) -> CallPanelSnapshot {
    use orbit_core::topics::recording::CallPanel as Core;
    match panel {
        Core::Offer {
            call_id,
            application,
            event_title,
        } => CallPanelSnapshot::Offer {
            call_id,
            application,
            event_title,
        },
        Core::Recording {
            recording_id,
            application,
            event_title,
        } => CallPanelSnapshot::Recording {
            recording_id,
            application,
            event_title,
        },
        Core::Ending {
            recording_id,
            application,
            ends_at_ms,
        } => CallPanelSnapshot::Ending {
            recording_id,
            application,
            ends_at_ms,
        },
    }
}

pub(crate) fn pending_inventory_detail(why: &orbit_core::topics::Pending) -> String {
    use orbit_core::topics::Pending;
    match why {
        Pending::NotObservedYet => "El inventario de audio aún no se ha observado.".into(),
        Pending::HelperStarting => {
            "El helper del Mac aún está arrancando; el inventario de audio no se ha observado."
                .into()
        }
        Pending::Reading { since_ms } => {
            format!("El inventario de audio se está leyendo desde {since_ms} ms.")
        }
        Pending::Transcription {
            completed,
            pending,
            failed,
            queued,
        } => {
            format!(
                "ASR en curso: {completed} grupos transcritos, {pending} pendientes, {failed} fallidos, en cola: {queued}."
            )
        }
        Pending::DaemonStarting => {
            "El daemon aún está arrancando; el inventario de audio no se ha observado.".into()
        }
    }
}

fn project_meeting_parts(
    recording: Option<&orbit_core::Recording>,
    signal: Option<&orbit_core::audio_signal::RecordingSignalView>,
    pending_jobs: u64,
    offline: bool,
) -> MeetingSnapshot {
    let phase = recording.map_or(MeetingPhase::Idle, |recording| match recording.phase {
        CapturePhase::Starting => MeetingPhase::Starting,
        CapturePhase::Recording => MeetingPhase::Recording,
        CapturePhase::Paused => MeetingPhase::Paused,
        CapturePhase::Finalizing => MeetingPhase::Finishing,
        CapturePhase::Idle | CapturePhase::Interrupted => MeetingPhase::Idle,
    });
    let audio = recording.map_or(MeetingAudioKind::System, |recording| {
        meeting_audio_kind(recording.audio)
    });
    MeetingSnapshot {
        phase,
        recording_id: recording.map(|recording| recording.id.clone()),
        title: recording.map(|recording| recording.title.clone()),
        started_at_ms: recording.map(|recording| recording.started_at_ms),
        ends_at_ms: recording.map(|recording| recording.expires_at_ms),
        limit_minutes: orbit_core::RECORDING_CEILING_MINUTES,
        audio,
        tracks: project_tracks(recording, signal),
        upload: MeetingUploadSnapshot {
            pending_chunks: pending_jobs,
            offline,
        },
        call_panel: None,
        call_preference: None,
        call_preference_unreadable: None,
        question: None,
        ask: None,
        voice_trigger: None,
        microphone_muted: false,
        recent: project_recent(recording),
        applications: Vec::new(),
        applications_state: MeetingApplicationsState::Unavailable,
        applications_detail: None,
    }
}

pub(super) fn meeting_audio_kind(audio: MeetingAudio) -> MeetingAudioKind {
    match audio {
        MeetingAudio::System => MeetingAudioKind::System,
        MeetingAudio::Application { .. } => MeetingAudioKind::Application,
        MeetingAudio::MicrophoneOnly => MeetingAudioKind::Microphone,
    }
}

pub(super) fn track_level(signal: Option<&TrackSignalView>, has_chunks: bool) -> MeetingTrackLevel {
    let Some(signal) = signal else {
        return if has_chunks {
            MeetingTrackLevel::Ok
        } else {
            MeetingTrackLevel::Silent
        };
    };
    if signal
        .since_last_samples_ms
        .is_some_and(|elapsed| elapsed >= 20_000)
        || signal.low_signal_ms >= 20_000
    {
        MeetingTrackLevel::Silent
    } else if signal.condition == SignalCondition::LowSignal {
        MeetingTrackLevel::Low
    } else if signal.received_frames > 0 {
        MeetingTrackLevel::Ok
    } else {
        MeetingTrackLevel::Silent
    }
}

pub(super) fn track_counts(total: u64, index: u64, count: u64) -> u64 {
    total / count + u64::from(index < total % count)
}

pub(super) fn project_tracks(
    recording: Option<&orbit_core::Recording>,
    signal: Option<&orbit_core::audio_signal::RecordingSignalView>,
) -> Vec<MeetingTrackSnapshot> {
    let Some(recording) = recording else {
        return Vec::new();
    };
    if matches!(
        recording.phase,
        CapturePhase::Idle | CapturePhase::Interrupted
    ) {
        return Vec::new();
    }
    let remote_expected = !matches!(recording.audio, MeetingAudio::MicrophoneOnly);
    let count = if remote_expected { 2 } else { 1 };
    let signal = signal.filter(|signal| signal.recording_id == recording.id);
    let mut tracks = vec![MeetingTrackSnapshot {
        name: MeetingTrackName::MicLocal,
        level: track_level(
            signal.map(|signal| &signal.mic_local),
            recording.sealed_chunks > 0,
        ),
        chunks_uploaded: track_counts(recording.sealed_chunks, 0, count),
        gaps: track_counts(recording.gaps, 0, count),
    }];
    if remote_expected {
        tracks.push(MeetingTrackSnapshot {
            name: MeetingTrackName::RemoteApp,
            level: track_level(
                signal.and_then(|signal| signal.remote_app.as_ref()),
                recording.sealed_chunks > 0,
            ),
            chunks_uploaded: track_counts(recording.sealed_chunks, 1, count),
            gaps: track_counts(recording.gaps, 1, count),
        });
    }
    tracks
}

pub(super) fn project_recent(
    recording: Option<&orbit_core::Recording>,
) -> Vec<RecentMeetingSnapshot> {
    let Some(recording) = recording.filter(|recording| {
        recording.phase == CapturePhase::Idle && recording.ended_at_ms.is_some()
    }) else {
        return Vec::new();
    };
    let ended_at_ms = recording.ended_at_ms.unwrap_or(recording.started_at_ms);
    vec![RecentMeetingSnapshot {
        recording_id: recording.id.clone(),
        title: recording.title.clone(),
        started_at_ms: recording.started_at_ms,
        duration_ms: ended_at_ms.saturating_sub(recording.started_at_ms),
        calendar_event_id: None,
        transcription: MeetingTranscriptionSnapshot {
            pending: 0,
            running: 0,
            completed: 0,
            failed: 0,
        },
        summary: MeetingSummaryState::None,
    }]
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests;
