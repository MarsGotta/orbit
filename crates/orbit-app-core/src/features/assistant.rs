use super::{FeatureAvailability, FeatureInput};
use crate::AppSnapshot;
use crate::topics::TopicFreshness;
use crate::{
    AssistantActivity, ChatMessage, ChatPending, ChatRole, VoiceNote, VoiceSnapshot, VoiceState,
};
use orbit_core::topics::{
    Knowledge, TopicData, TopicId,
    assistant::{Activity, VoiceAbsence, VoiceAvailability},
};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

pub type View = crate::ChatSnapshot;

pub fn project(input: &FeatureInput<'_>) -> View {
    let mut view = View {
        availability: Some(FeatureAvailability::missing(input.topics)),
        session_id: None,
        turn: Vec::new(),
        voice_audio: input.local.voice_audio.clone(),
        pending_contract: None,
        voice: VoiceSnapshot {
            state: VoiceState::Unknown,
            level: 0,
            availability: Some(FeatureAvailability::missing(input.topics)),
        },
        activity: None,
    };
    let Some(cell) = input.topics.topic(TopicId::Assistant) else {
        return view;
    };
    let TopicData::Assistant(knowledge) = &cell.knowledge else {
        return view;
    };
    let value = match knowledge {
        Knowledge::Present { value, .. } => value,
        Knowledge::Unknown { why } => {
            let availability = FeatureAvailability::unknown_pending(why);
            view.availability = Some(availability.clone());
            view.voice.availability = Some(availability);
            return view;
        }
        Knowledge::Absent { refusal } => {
            let cause = refusal.to_string();
            view.availability = Some(FeatureAvailability::Absent {
                cause: cause.clone(),
            });
            view.voice.availability = Some(FeatureAvailability::Absent { cause });
            return view;
        }
    };
    view.availability = Some(match &cell.freshness {
        TopicFreshness::Current => FeatureAvailability::Current,
        TopicFreshness::Stale { since_ms } => FeatureAvailability::Stale {
            since_ms: *since_ms,
        },
    });
    view.session_id = value.session_id.clone();
    view.turn = value
        .turn
        .iter()
        .flat_map(|turn| std::iter::once(&turn.user).chain(turn.answer.as_ref()))
        .map(|message| ChatMessage {
            id: message.id.clone(),
            role: match message.role.as_str() {
                "user" => ChatRole::User,
                "assistant" => ChatRole::Assistant,
                _ => ChatRole::System,
            },
            text: message.text.clone(),
            at_ms: i64::try_from(message.at_ms).unwrap_or(i64::MAX),
            streaming: value.turn_in_flight && !message.complete,
            voice: message.voice.as_ref().map(|voice| VoiceNote {
                duration_ms: voice.duration_ms,
                transcript: voice.transcript.clone(),
                failure: voice
                    .failure
                    .as_ref()
                    .map(|failure| crate::VoiceTranscriptionFailure {
                        code: failure.code.clone(),
                        provider_status: failure.provider_status,
                        hub_status: failure.hub_status,
                        timeout_ms: failure.timeout_ms,
                        bytes: failure.bytes.map(|bytes| bytes.to_string()),
                        max_bytes: failure.max_bytes.map(|bytes| bytes.to_string()),
                        attempts: failure.attempts,
                    }),
                audio_available: voice.audio_available,
            }),
            receipts: Vec::new(),
        })
        .collect();
    view.pending_contract = value.pending.as_ref().map(|pending| {
        if pending.clarification {
            ChatPending::Clarify {
                id: pending.request_id.clone(),
                text: pending.description.clone(),
            }
        } else {
            ChatPending::Approval {
                id: pending.request_id.clone(),
                text: pending.description.clone(),
            }
        }
    });
    let (voice_state, voice_availability) = match &value.voice {
        VoiceAvailability::Unknown { why } => (
            VoiceState::Unknown,
            FeatureAvailability::unknown_pending(why),
        ),
        VoiceAvailability::Absent {
            code: VoiceAbsence::SpeechUnavailable,
        } => (
            VoiceState::Unknown,
            FeatureAvailability::Absent {
                cause: "speech_unavailable".into(),
            },
        ),
        VoiceAvailability::Present => (
            if value.voice_active {
                VoiceState::Listening
            } else {
                VoiceState::Idle
            },
            FeatureAvailability::Current,
        ),
    };
    view.voice.state = voice_state;
    view.voice.availability = Some(voice_availability);
    view.activity = value.activity.as_ref().map(|activity| match activity {
        Activity::Transcribing => AssistantActivity::Transcribing,
        Activity::Thinking => AssistantActivity::Thinking,
        Activity::Responding => AssistantActivity::Responding,
    });
    view
}

pub fn apply(view: &View, snapshot: &mut AppSnapshot) {
    snapshot.chat = view.clone();
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "command", rename_all = "snake_case")]
#[ts(export, export_to = "../../apps/desktop/src/bridge/generated/")]
pub enum AssistantCommand {
    VoiceNoteAudio { message_id: String },
}
