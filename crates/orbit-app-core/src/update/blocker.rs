//! Facts that hold an update back. Human copy belongs to the shell.
use super::UpdateService;
use crate::{
    UpdateBlocker,
    topics::{TopicCache, TopicFreshness},
};
use orbit_core::{
    CapturePhase,
    topics::{Knowledge, TopicData, TopicId},
};

impl UpdateService {
    pub fn installation_blocker(topics: &TopicCache) -> Option<String> {
        Self::installation_blocker_reason(topics).map(|reason| {
            crate::PresentationCopy::default().text(crate::PresentationText::UpdateBlocked(reason))
        })
    }

    pub fn installation_blocker_reason(topics: &TopicCache) -> Option<UpdateBlocker> {
        for topic in [TopicId::Recording, TopicId::Assistant] {
            let Some(cell) = topics.topic(topic) else {
                return Some(UpdateBlocker::NotObserved { topic });
            };
            if !matches!(cell.freshness, TopicFreshness::Current) {
                return Some(UpdateBlocker::Stale { topic });
            }
        }
        recording_blocker(topics).or_else(|| call_blocker(topics))
    }
}

fn recording_blocker(topics: &TopicCache) -> Option<UpdateBlocker> {
    let topic = TopicId::Recording;
    let Some(TopicData::Recording(recording)) = topics.topic(topic).map(|cell| &cell.knowledge)
    else {
        return Some(UpdateBlocker::Unknown { topic });
    };
    let value = match recording {
        Knowledge::Present { value, .. } => value,
        Knowledge::Unknown { why } => {
            return Some(UpdateBlocker::Pending {
                topic,
                why: why.clone(),
            });
        }
        Knowledge::Absent { refusal } => {
            return Some(UpdateBlocker::Refused {
                topic,
                refusal: Box::new(refusal.clone()),
            });
        }
    };
    // Only live capture: paused, interrupted or closed recordings lose no
    // capture to a restart. Finalizing still drains captured audio.
    if value.recording.as_ref().is_some_and(|recording| {
        matches!(
            recording.phase,
            CapturePhase::Starting | CapturePhase::Recording | CapturePhase::Finalizing
        )
    }) {
        return Some(UpdateBlocker::Recording);
    }
    value
        .call
        .as_ref()
        .is_some_and(|call| call.panel.is_some())
        .then_some(UpdateBlocker::Call)
}

fn call_blocker(topics: &TopicCache) -> Option<UpdateBlocker> {
    let topic = TopicId::Assistant;
    let Some(TopicData::Assistant(assistant)) = topics.topic(topic).map(|cell| &cell.knowledge)
    else {
        return Some(UpdateBlocker::Unknown { topic });
    };
    let value = match assistant {
        Knowledge::Present { value, .. } => value,
        Knowledge::Unknown { why } => {
            return Some(UpdateBlocker::Pending {
                topic,
                why: why.clone(),
            });
        }
        Knowledge::Absent { refusal } => {
            return Some(UpdateBlocker::Refused {
                topic,
                refusal: Box::new(refusal.clone()),
            });
        }
    };
    match &value.call {
        Knowledge::Present { value: None, .. } => None,
        Knowledge::Present { value: Some(_), .. } => Some(UpdateBlocker::Call),
        Knowledge::Unknown { why } => Some(UpdateBlocker::Pending {
            topic,
            why: why.clone(),
        }),
        Knowledge::Absent { refusal } => Some(UpdateBlocker::Refused {
            topic,
            refusal: Box::new(refusal.clone()),
        }),
    }
}
