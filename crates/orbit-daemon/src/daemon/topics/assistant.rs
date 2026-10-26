use super::{ProjectTopic, TopicSource};
use orbit_core::{
    refusal::{Refusal, Resource},
    topics::{
        Knowledge, Pending, TopicData, TopicId,
        assistant::{Activity, Value, VoiceAbsence, VoiceAvailability},
    },
};

pub(super) struct Projector;

impl ProjectTopic for Projector {
    fn project(&self, source: &TopicSource<'_>) -> TopicData {
        let state = source.state;
        // The open conversation and the turn; its messages are read
        // from the hub.
        let value = Value {
            session_id: state.session_id(),
            agent_id: state.agent_id().map(str::to_owned),
            turn: state.durable.assistant.turn.clone().map(Box::new),
            // Only the open conversation's, never every conversation's.
            pending_voice: state
                .durable
                .open_conversation_pending_voice()
                .cloned()
                .collect(),
            pending: state.durable.assistant.pending_approval.clone(),
            history_revision: source
                .runtime
                .and_then(|runtime| runtime.assistant.snapshot_revision),
            waiting_sessions: source
                .runtime
                .map(|runtime| runtime.assistant.waiting_sessions.clone())
                .unwrap_or_default(),
            voice_active: state.durable.voice.voice_active,
            turn_in_flight: state.durable.assistant.active_request.is_some(),
            activity: activity(source),
            // The voice that reads this conversation's answers, as its
            // last reading left it; another agent's voice says nothing here.
            voice: match source.runtime.and_then(|runtime| {
                runtime
                    .assistant
                    .speech_voices
                    .get(&runtime.reading_voice())
                    .copied()
            }) {
                Some(true) => VoiceAvailability::Present,
                Some(false) => VoiceAvailability::Absent {
                    code: VoiceAbsence::SpeechUnavailable,
                },
                None => VoiceAvailability::Unknown {
                    why: Pending::NotObservedYet,
                },
            },
            speaking: source.runtime.and_then(|runtime| {
                runtime
                    .assistant
                    .read_aloud
                    .as_ref()
                    .map(|read| read.message_id.clone())
            }),
            call: Knowledge::Present {
                value: state.durable.voice.call.clone(),
                as_of_ms: None,
            },
            short_voice_note: state.views.short_voice_note.clone(),
            work: source
                .runtime
                .and_then(|runtime| runtime.assistant.work.clone())
                .map(Box::new),
            microphone_muted: source
                .runtime
                .is_some_and(|runtime| runtime.call_microphone_muted()),
        };
        let data = TopicData::Assistant(Knowledge::Present {
            value,
            as_of_ms: None,
        });
        match serde_json::to_vec(&data) {
            Ok(encoded) if encoded.len() <= TopicId::Assistant.max_bytes() => data,
            Ok(encoded) => TopicData::absent(
                TopicId::Assistant,
                Refusal::TooLarge {
                    resource: Resource::Message,
                    bytes: encoded.len() as u64,
                    max_bytes: TopicId::Assistant.max_bytes() as u64,
                },
            ),
            Err(_error) => TopicData::absent(
                TopicId::Assistant,
                Refusal::EncodeFailed {
                    resource: orbit_core::refusal::Resource::IpcResponse,
                },
            ),
        }
    }
}

fn activity(source: &TopicSource<'_>) -> Option<Activity> {
    if source
        .runtime
        .is_some_and(|runtime| runtime.assistant.voice_task.active().is_some())
    {
        return Some(Activity::Transcribing);
    }
    let request_id = source.state.durable.assistant.active_request.as_ref()?;
    if let Some(retry) = source
        .runtime
        .and_then(|runtime| runtime.assistant.retry.clone())
    {
        return Some(retry);
    }
    let responding = source
        .state
        .durable
        .assistant
        .turn
        .as_ref()
        .is_some_and(|turn| {
            &turn.user.id == request_id
                && turn.answer.as_ref().is_some_and(|answer| !answer.complete)
        });
    Some(if responding {
        Activity::Responding
    } else {
        Activity::Thinking
    })
}

#[cfg(test)]
#[path = "assistant_tests.rs"]
mod tests;
