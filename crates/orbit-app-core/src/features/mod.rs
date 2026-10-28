//! Function projections over typed topics and local presentation state.
#![deny(clippy::wildcard_enum_match_arm)]
use crate::{
    AppSnapshot, LoginItemState, ProfileSettingsSnapshot, UpdateSnapshot, calendar::LocalClock,
    topics::TopicCache,
};
pub use orbit_core::features::FeatureId;
use orbit_core::topics::{Knowledge, Pending, TopicData, TopicId};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

pub mod agenda;
pub mod assistant;
pub mod briefing;
pub mod control;
pub mod local;
pub mod note;
pub mod notes;
pub mod permissions;
pub mod recording;
pub mod status;

pub struct FeatureInput<'a> {
    pub topics: &'a TopicCache,
    pub local: LocalAppState<'a>,
    pub clock: LocalClock,
    pub copy: crate::PresentationCopy,
}

pub(crate) fn feature_availability(
    feature: FeatureId,
    input: &FeatureInput<'_>,
) -> Option<FeatureAvailability> {
    match feature {
        FeatureId::Notes => {
            let Some(topic) = input.topics.topic(TopicId::Notes) else {
                return Some(FeatureAvailability::missing(input.topics));
            };
            if let crate::topics::TopicFreshness::Stale { since_ms } = &topic.freshness {
                return Some(FeatureAvailability::Stale {
                    since_ms: *since_ms,
                });
            }
            let TopicData::Notes(knowledge) = &topic.knowledge else {
                return Some(FeatureAvailability::Unknown {
                    cause: "notes_topic_type_mismatch".into(),
                });
            };
            Some(match knowledge {
                // Whether the list read reached the hub is in
                // `notes.index`; the feature is current once its head is.
                Knowledge::Present { .. } => FeatureAvailability::Current,
                Knowledge::Absent { refusal } => FeatureAvailability::Absent {
                    cause: refusal.to_string(),
                },
                Knowledge::Unknown { why } => FeatureAvailability::unknown_pending(why),
            })
        }
        FeatureId::Status
        | FeatureId::Permissions
        | FeatureId::Agenda
        | FeatureId::Note
        | FeatureId::Recording
        | FeatureId::Assistant
        | FeatureId::Control
        | FeatureId::Briefing
        | FeatureId::Local => None,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "state", rename_all = "snake_case")]
#[ts(export, export_to = "../../apps/desktop/src/bridge/generated/")]
pub enum FeatureAvailability {
    Current,
    Stale {
        #[ts(type = "number | null")]
        since_ms: Option<i64>,
    },
    Unknown {
        cause: String,
    },
    Absent {
        cause: String,
    },
}

impl FeatureAvailability {
    fn unknown_pending(why: &Pending) -> Self {
        // The UI owns catalogue copy. These stable codes retain the measurements
        // carried by Pending without exposing Rust Debug variant names.
        let cause = match why {
            Pending::NotObservedYet => "pending:not_observed_yet".into(),
            Pending::HelperStarting => "pending:helper_starting".into(),
            Pending::Reading { since_ms } => format!("pending:reading:{since_ms}"),
            Pending::Transcription {
                completed,
                pending,
                failed,
                queued,
            } => format!("pending:transcription:{completed}:{pending}:{failed}:{queued}"),
            Pending::DaemonStarting => "pending:daemon_starting".into(),
        };
        Self::Unknown { cause }
    }

    pub fn missing(topics: &TopicCache) -> Self {
        use crate::topics::TopicConnection;
        let cause = match topics.connection() {
            TopicConnection::Connecting => "topic_connecting".into(),
            TopicConnection::Ready => "topic_not_observed_yet".into(),
            TopicConnection::Ended { cause } => cause.clone(),
        };
        Self::Unknown { cause }
    }
}

#[derive(Clone, Copy)]
pub struct LocalAppState<'a> {
    pub calendar: &'a crate::CalendarSnapshot,
    pub notes: &'a Option<crate::NotesSnapshot>,
    pub profile: &'a Option<ProfileSettingsSnapshot>,
    pub login_item: &'a LoginItemState,
    pub update: &'a UpdateSnapshot,
    pub onboarding_done: bool,
    /// The voice note the user last asked to hear, kept by the app.
    pub voice_audio: &'a Option<crate::VoiceNoteAudioSnapshot>,
}

impl<'a> FeatureInput<'a> {
    pub(super) fn localized_notes(&self) -> Option<crate::NotesSnapshot> {
        let mut notes = self.local.notes.clone();
        if let Some(notes) = notes.as_mut() {
            notes::localize_details(notes, self.copy);
        }
        notes
    }

    pub fn with_copy(mut self, copy: crate::PresentationCopy) -> Self {
        self.copy = copy;
        self
    }

    pub fn new(snapshot: &'a AppSnapshot, topics: &'a TopicCache) -> Self {
        Self::with_clock(snapshot, topics, LocalClock::from_snapshot(snapshot))
    }

    pub fn with_clock(
        snapshot: &'a AppSnapshot,
        topics: &'a TopicCache,
        clock: LocalClock,
    ) -> Self {
        Self {
            topics,
            copy: crate::PresentationCopy::default(),
            local: LocalAppState {
                calendar: &snapshot.calendar,
                notes: &snapshot.notes,
                profile: &snapshot.settings.profile,
                login_item: &snapshot.settings.login_item,
                update: &snapshot.settings.update,
                onboarding_done: snapshot.settings.onboarding_done,
                voice_audio: &snapshot.chat.voice_audio,
            },
            clock,
        }
    }
}

macro_rules! features {
    ($($variant:ident => $module:ident),+ $(,)?) => {
        #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
        #[serde(tag = "feature", rename_all = "snake_case")]
        #[ts(export, export_to = "../../apps/desktop/src/bridge/generated/")]
        pub enum FeatureFrame {
            $($variant {
                revision: FrameRevision,
                view: Box<$module::View>,
                #[serde(default, skip_serializing_if = "Option::is_none")]
                #[ts(optional)]
                availability: Option<FeatureAvailability>,
            }),+
        }

        impl FeatureFrame {
            pub fn feature(&self) -> FeatureId {
                match self { $(Self::$variant { .. } => FeatureId::$variant),+ }
            }

            pub fn revision(&self) -> FrameRevision {
                match self { $(Self::$variant { revision, .. } => *revision),+ }
            }

            pub fn apply(&self, snapshot: &mut AppSnapshot) {
                match self { $(Self::$variant { view, .. } => $module::apply(view, snapshot)),+ }
            }
        }

        pub fn project_all(input: &FeatureInput<'_>, revision: FrameRevision) -> Vec<FeatureFrame> {
            vec![$(FeatureFrame::$variant {
                revision,
                view: Box::new($module::project(input)),
                availability: feature_availability(FeatureId::$variant, input),
            }),+]
        }

        pub fn project_changed(
            before: &FeatureInput<'_>,
            after: &FeatureInput<'_>,
            mut next_revision: impl FnMut() -> FrameRevision,
        ) -> Vec<FeatureFrame> {
            let mut frames = Vec::new();
            $(if $module::project(before) != $module::project(after)
                || feature_availability(FeatureId::$variant, before)
                    != feature_availability(FeatureId::$variant, after) {
                frames.push(FeatureFrame::$variant {
                    revision: next_revision(),
                    view: Box::new($module::project(after)),
                    availability: feature_availability(FeatureId::$variant, after),
                });
            })+
            frames
        }
    };
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../apps/desktop/src/bridge/generated/")]
pub struct FrameRevision {
    #[ts(type = "number")]
    pub epoch: u64,
    #[ts(type = "number")]
    pub counter: u64,
}

features! {
    Status => status,
    Permissions => permissions,
    Agenda => agenda,
    Notes => notes,
    Note => note,
    Recording => recording,
    Assistant => assistant,
    Control => control,
    Briefing => briefing,
    Local => local,
}

#[cfg(test)]
mod command_mapping_tests {
    use super::*;
    use orbit_core::protocol::{Command, MeetingAudio};

    #[test]
    fn missing_notes_frame_names_what_is_unknown() -> Result<(), Box<dyn std::error::Error>> {
        let snapshot = crate::AppSnapshot::empty(1, crate::DaemonConnection::Connecting);
        let topics = crate::topics::TopicCache::default();
        let frames = project_all(
            &FeatureInput::new(&snapshot, &topics),
            FrameRevision {
                epoch: 1,
                counter: 1,
            },
        );
        let frame = frames
            .iter()
            .find(|frame| frame.feature() == FeatureId::Notes)
            .ok_or("notes frame")?;
        let FeatureFrame::Notes { availability, .. } = frame else {
            return Err("unexpected feature frame".into());
        };
        assert_eq!(
            availability,
            &Some(FeatureAvailability::Unknown {
                cause: "topic_connecting".into()
            })
        );
        let mut topics = crate::topics::TopicCache::default();
        topics.observe_at(
            orbit_core::topics::TopicFrame {
                epoch: "notes-present".into(),
                revision: 1,
                data: orbit_core::topics::TopicData::Notes(Knowledge::Present {
                    value: orbit_core::topics::notes::Value::default(),
                    as_of_ms: None,
                }),
            },
            Some(1),
        );
        let frame = project_all(
            &FeatureInput::new(&snapshot, &topics),
            FrameRevision {
                epoch: 1,
                counter: 2,
            },
        )
        .into_iter()
        .find(|frame| frame.feature() == FeatureId::Notes)
        .ok_or("notes frame")?;
        let FeatureFrame::Notes { availability, .. } = frame else {
            return Err("notes frame lost its type".into());
        };
        assert_eq!(availability, Some(FeatureAvailability::Current));
        Ok(())
    }

    #[test]
    fn recording_preserves_the_selected_application() {
        let snapshot = crate::AppSnapshot::default();
        let result = recording::map_command(
            recording::RecordingCommand::MeetingStartSelected {
                title: "Revisión".into(),
                minutes: None,
                audio: Some(crate::MeetingAudio::Application {
                    application_pid: 4242,
                }),
                event: None,
                event_id: None,
            },
            &snapshot.calendar,
            &crate::calendar::LocalClock::from_snapshot(&snapshot),
        );
        assert!(matches!(result,
            Ok(Command::MeetingStart {
                title,
                duration_minutes: None,
                audio: MeetingAudio::Application { application_pid: 4242 },
                event: None,
            }) if title == "Revisión"
        ));
    }
}
