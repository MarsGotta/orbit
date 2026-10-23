use super::{FeatureAvailability, FeatureInput};
use crate::topics::TopicFreshness;
use crate::{AppSnapshot, CallSnapshot, MeetingApplicationsState, MeetingSnapshot};
use orbit_core::topics::{Knowledge, TopicData, TopicId};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../apps/desktop/src/bridge/generated/")]
pub struct RecordingView {
    pub availability: FeatureAvailability,
    pub call_availability: FeatureAvailability,
    pub meeting: MeetingSnapshot,
    pub call: Option<CallSnapshot>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub freshness: Option<RecordingFreshness>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "state", rename_all = "snake_case")]
#[ts(export, export_to = "../../apps/desktop/src/bridge/generated/")]
pub enum RecordingFreshness {
    Current,
    Stale {
        #[ts(type = "number | null")]
        received_at_ms: Option<i64>,
    },
}

pub type View = RecordingView;

pub fn project(input: &FeatureInput<'_>) -> View {
    let mut meeting = AppSnapshot::default().meeting;
    let mut freshness = None;
    let mut availability = FeatureAvailability::missing(input.topics);
    if let Some(topic) = input.topics.topic(TopicId::Recording) {
        freshness = Some(match &topic.freshness {
            TopicFreshness::Current => RecordingFreshness::Current,
            TopicFreshness::Stale { .. } => RecordingFreshness::Stale {
                received_at_ms: topic.received_at_ms,
            },
        });
        if let TopicData::Recording(knowledge) = &topic.knowledge {
            match knowledge {
                Knowledge::Present { value, .. } => {
                    meeting = crate::projection::project_meeting_topic(value);
                    availability = match &topic.freshness {
                        TopicFreshness::Current => FeatureAvailability::Current,
                        TopicFreshness::Stale { since_ms } => FeatureAvailability::Stale {
                            since_ms: *since_ms,
                        },
                    };
                }
                Knowledge::Absent { refusal } => {
                    availability = FeatureAvailability::Absent {
                        cause: refusal.to_string(),
                    };
                    meeting.call_panel = None;
                    meeting.call_preference = None;
                    meeting.call_preference_unreadable = None;
                    meeting.applications.clear();
                    meeting.applications_state = MeetingApplicationsState::Unavailable;
                    meeting.applications_detail = Some(refusal.to_string());
                }
                Knowledge::Unknown { why } => {
                    availability = FeatureAvailability::unknown_pending(why);
                    meeting.call_panel = None;
                    meeting.call_preference = None;
                    meeting.call_preference_unreadable = None;
                    meeting.applications.clear();
                    meeting.applications_state = MeetingApplicationsState::Unavailable;
                    meeting.applications_detail =
                        Some(crate::projection::pending_inventory_detail(why));
                }
            }
        }
    } else {
        meeting.call_panel = None;
        meeting.call_preference = None;
        meeting.call_preference_unreadable = None;
        meeting.applications.clear();
        meeting.applications_state = MeetingApplicationsState::Unavailable;
        meeting.applications_detail = Some(crate::projection::pending_inventory_detail(
            &orbit_core::topics::Pending::NotObservedYet,
        ));
    }
    let (call, call_availability) = match input.topics.topic(TopicId::Assistant) {
        Some(topic) => match &topic.knowledge {
            TopicData::Assistant(Knowledge::Present { value, .. }) => match &value.call {
                Knowledge::Present { value: call, .. } => {
                    let last_line = value
                        .turn
                        .as_ref()
                        .and_then(|turn| turn.answer.as_ref())
                        .map(|message| message.text.clone());
                    let screen_granted = matches!(
                        input.topics.topic(TopicId::Permissions).map(|cell| &cell.knowledge),
                        Some(TopicData::Permissions(Knowledge::Present { value, .. }))
                            if value.screen_recording == orbit_core::PermissionState::Granted
                    );
                    let privacy_armed = matches!(
                        input.topics.topic(TopicId::Status).map(|cell| &cell.knowledge),
                        Some(TopicData::Status(Knowledge::Present { value, .. }))
                            if !value.privacy_stopped
                    );
                    let voice_unavailable = matches!(
                        value.voice,
                        orbit_core::topics::assistant::VoiceAvailability::Absent { .. }
                    );
                    (
                        call.as_ref().map(|call| CallSnapshot {
                            started_at_ms: call.started_at_ms,
                            seeing_screen: screen_granted && privacy_armed,
                            last_line,
                            speaking: call.phase == orbit_core::voice::CallPhase::Responding,
                            voice_unavailable,
                            expires_at_ms: call.expires_at_ms,
                            origin: match call.origin {
                                orbit_core::CallOrigin::App => crate::CallOrigin::App,
                                orbit_core::CallOrigin::Hotkey => crate::CallOrigin::Hotkey,
                            },
                            agent_id: value.agent_id.clone(),
                            idle_hangup_ms: orbit_core::voice::VoicePolicy::default()
                                .idle_timeout_ms,
                            microphone_muted: value.microphone_muted,
                        }),
                        FeatureAvailability::Current,
                    )
                }
                Knowledge::Unknown { why } => (None, FeatureAvailability::unknown_pending(why)),
                Knowledge::Absent { refusal } => (
                    None,
                    FeatureAvailability::Absent {
                        cause: refusal.to_string(),
                    },
                ),
            },
            TopicData::Assistant(Knowledge::Unknown { why }) => {
                (None, FeatureAvailability::unknown_pending(why))
            }
            TopicData::Assistant(Knowledge::Absent { refusal }) => (
                None,
                FeatureAvailability::Absent {
                    cause: refusal.to_string(),
                },
            ),
            TopicData::Status(_)
            | TopicData::Permissions(_)
            | TopicData::Recording(_)
            | TopicData::Agenda(_)
            | TopicData::Notes(_)
            | TopicData::Control(_)
            | TopicData::Notices(_) => (
                None,
                FeatureAvailability::Unknown {
                    cause: "assistant_topic_type_mismatch".into(),
                },
            ),
        },
        None => (None, FeatureAvailability::missing(input.topics)),
    };
    join_answer(&mut meeting, input);
    RecordingView {
        availability,
        call_availability,
        meeting,
        call,
        freshness,
    }
}

/// The question's answer is its turn's (followed by the
/// turn's id): its text while it arrives (responding) and once complete. The
/// recording topic says where the turn is; the assistant topic carries the
/// words.
fn join_answer(meeting: &mut crate::MeetingSnapshot, input: &FeatureInput<'_>) {
    let Some(question) = meeting.question.as_mut() else {
        return;
    };
    let Some(TopicData::Assistant(Knowledge::Present { value, .. })) = input
        .topics
        .topic(TopicId::Assistant)
        .map(|topic| &topic.knowledge)
    else {
        return;
    };
    let Some(message) = value
        .turn
        .as_ref()
        .filter(|turn| turn.user.id == question.id)
        .and_then(|turn| turn.answer.as_ref())
    else {
        return;
    };
    if message.text.trim().is_empty() {
        return;
    }
    question.answer = Some(message.text.clone());
    if question.stage == crate::MeetingQuestionStage::Thinking {
        question.stage = crate::MeetingQuestionStage::Responding;
    }
}

pub fn apply(view: &View, snapshot: &mut AppSnapshot) {
    snapshot.recording_availability = Some(view.availability.clone());
    snapshot.call_availability = Some(view.call_availability.clone());
    snapshot.meeting = view.meeting.clone();
    snapshot.call = view.call.clone();
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "command", rename_all = "snake_case", deny_unknown_fields)]
#[ts(export, export_to = "../../apps/desktop/src/bridge/generated/")]
pub enum RecordingCommand {
    #[serde(skip)]
    #[ts(skip)]
    MeetingStart { title: String },
    #[serde(rename = "meeting_start")]
    MeetingStartSelected {
        #[serde(default)]
        #[ts(as = "Option<String>", optional)]
        title: String,
        /// Accepted from an older interface and ignored: every recording
        /// runs to the one safety ceiling (`orbit_core::RECORDING_CEILING_MINUTES`).
        #[serde(default, skip_serializing)]
        #[ts(skip)]
        minutes: Option<u16>,
        #[serde(default)]
        #[ts(optional)]
        audio: Option<crate::MeetingAudio>,
        /// Stable identity of the occurrence selected in Hoy or Calendario.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        event: Option<crate::MeetingEventReference>,
        /// Last resort for an event without a stable reference.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        event_id: Option<String>,
    },
}

pub(crate) fn map_command(
    command: RecordingCommand,
    calendar: &crate::CalendarSnapshot,
    clock: &crate::calendar::LocalClock,
) -> Result<orbit_core::protocol::Command, crate::AppError> {
    use orbit_core::protocol::{Command as DaemonCommand, MeetingAudio};
    let utc_offset_ms = matches!(&clock.timezone_name, Ok(name) if !name.trim().is_empty())
        .then(|| clock.zone.offset_ms_at(clock.now_ms));
    Ok(match command {
        RecordingCommand::MeetingStart { title } => DaemonCommand::MeetingStart {
            title: orbit_core::protocol::recording_title(title, clock.now_ms, utc_offset_ms),
            audio: MeetingAudio::System,
            duration_minutes: None,
            event: None,
        },
        RecordingCommand::MeetingStartSelected {
            title,
            minutes: _,
            audio,
            event,
            event_id,
        } => {
            let selected = match event {
                Some(reference) => Some((
                    calendar.events.iter().find(|event| {
                        event.calendar_item_id.as_ref() == Some(&reference.calendar_item_id)
                            && event.occurrence_start_ms == Some(reference.occurrence_start_ms)
                    }),
                    format!(
                        "«{}» (ocurrencia {} ms)",
                        reference.calendar_item_id, reference.occurrence_start_ms
                    ),
                )),
                None => event_id.map(|id| {
                    (
                        calendar.events.iter().find(|event| event.id == id),
                        format!("«{id}»"),
                    )
                }),
            };
            let event = selected.map(|(event, reference)| {
                event.ok_or_else(|| crate::AppError {
                    code: "meeting_event_not_found".into(),
                    message: format!("No se encuentra el evento {reference} entre los {} eventos leídos; actualiza el calendario.", calendar.events.len()),
                    refusal: None,
                })
            }).transpose()?;
            let audio = audio.unwrap_or_default();
            DaemonCommand::MeetingStart {
                title: event.map_or_else(
                    || orbit_core::protocol::recording_title(title, clock.now_ms, utc_offset_ms),
                    |event| event.title.clone(),
                ),
                audio,
                // The event's end never stops the recording: meetings run over.
                duration_minutes: None,
                event: event.and_then(|event| {
                    Some(orbit_core::call_detection::LinkedEvent {
                        calendar_item_id: event.calendar_item_id.clone()?,
                        occurrence_start_ms: event.occurrence_start_ms?,
                        title: event.title.clone(),
                        starts_at_ms: event.start_ms,
                        ends_at_ms: event.end_ms,
                        attendees: event
                            .attendees
                            .as_deref()
                            .unwrap_or_default()
                            .iter()
                            .map(|guest| orbit_core::call_detection::Attendee {
                                name: guest.name.clone(),
                                email: guest.email.clone().unwrap_or_default(),
                                is_self: guest.is_self,
                            })
                            .collect(),
                        recurring: Some(event.recurrence.is_some()),
                    })
                }),
            }
        }
    })
}

/// A missing or stale recording topic is unknown. The topic has no linked
/// event, so both views match the series and active phase only while the
/// calendar occurrence is in progress.
pub(super) fn event_recording(
    event: &crate::CalendarEventSnapshot,
    input: &FeatureInput<'_>,
) -> Option<crate::BriefingRecordingState> {
    let topic = input.topics.topic(TopicId::Recording)?;
    if topic.freshness != TopicFreshness::Current {
        return None;
    }
    let TopicData::Recording(Knowledge::Present { value, .. }) = &topic.knowledge else {
        return None;
    };
    Some(
        if value.recording.as_ref().is_some_and(|current| {
            event.start_ms <= input.clock.now_ms
                && input.clock.now_ms < event.end_ms
                && super::briefing::series_key(&current.title)
                    == super::briefing::series_key(&event.title)
                && matches!(
                    current.phase,
                    orbit_core::CapturePhase::Starting
                        | orbit_core::CapturePhase::Recording
                        | orbit_core::CapturePhase::Paused
                        | orbit_core::CapturePhase::Finalizing
                )
        }) {
            crate::BriefingRecordingState::Recording
        } else {
            crate::BriefingRecordingState::None
        },
    )
}

#[cfg(test)]
mod mapping_tests {
    use super::*;
    use orbit_core::protocol::{Command, MeetingAudio};

    fn map_command(command: RecordingCommand) -> Result<Command, crate::AppError> {
        let snapshot = crate::AppSnapshot::default();
        super::map_command(
            command,
            &snapshot.calendar,
            &crate::calendar::LocalClock::from_snapshot(&snapshot),
        )
    }

    #[test]
    fn microphone_selection_reaches_microphone_only() {
        let result = map_command(RecordingCommand::MeetingStartSelected {
            title: "Daily".into(),
            minutes: Some(37),
            audio: Some(crate::MeetingAudio::MicrophoneOnly),
            event: None,
            event_id: None,
        });
        assert!(
            matches!(result, Ok(Command::MeetingStart { audio: MeetingAudio::MicrophoneOnly, duration_minutes: None, title, event: None }) if title == "Daily")
        );
    }

    /// The calendar never sets when a recording stops: an event that ends
    /// in twenty minutes gives the daemon no duration at all.
    #[test]
    fn a_recording_from_an_event_takes_no_ceiling_from_it() -> Result<(), Box<dyn std::error::Error>>
    {
        let mut snapshot = crate::AppSnapshot::default();
        snapshot
            .calendar
            .events
            .push(serde_json::from_value(serde_json::json!({
                "id": "event-1", "calendar_item_id": "planning-series",
                "occurrence_start_ms": 1_790_000_000_000_i64,
                "title": "Planificación", "start_ms": 1_790_000_000_000_i64,
                "end_ms": 1_790_003_000_000_i64,
            }))?);
        let clock = crate::calendar::LocalClock {
            now_ms: 1_790_001_800_000,
            zone: crate::LocalZone::utc(),
            timezone_name: Ok("UTC".into()),
        };
        let command: RecordingCommand = serde_json::from_value(serde_json::json!({
            "command": "meeting_start", "event_id": "event-1"
        }))?;
        let mapped =
            serde_json::to_value(super::map_command(command, &snapshot.calendar, &clock)?)?;
        assert_eq!(mapped["title"], "Planificación");
        assert!(
            mapped.get("duration_minutes").is_none(),
            "the event's end must not stop the recording: {mapped}"
        );
        Ok(())
    }

    #[test]
    fn record_on_an_event_uses_its_reference_guests_and_title()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut snapshot = crate::AppSnapshot::default();
        let event: crate::CalendarEventSnapshot = serde_json::from_value(serde_json::json!({
            "id": "event-1", "calendar_item_id": "committee-series",
            "occurrence_start_ms": 1_790_000_000_000_i64,
            "title": "Comité", "start_ms": 1_790_000_600_000_i64,
            "end_ms": 1_790_004_200_000_i64,
            "attendees": [{"name": "Lucía", "email": "lucia@example.com"},
                {"name": "Alina", "is_self": true}]
        }))?;
        snapshot.calendar.events.push(event);
        let clock = crate::calendar::LocalClock {
            now_ms: 1_790_001_800_000, // Twenty minutes after the event began.
            zone: crate::LocalZone::utc(),
            timezone_name: Ok("UTC".into()),
        };
        let command: RecordingCommand = serde_json::from_value(serde_json::json!({
            "command": "meeting_start", "event_id": "event-1"
        }))?;
        let Command::MeetingStart {
            title,
            duration_minutes,
            event: Some(link),
            ..
        } = super::map_command(command.clone(), &snapshot.calendar, &clock)?
        else {
            panic!("the selected event was not linked");
        };
        assert_eq!(title, "Comité");
        assert_eq!(duration_minutes, None);
        assert_eq!(link.calendar_item_id, "committee-series");
        assert_eq!(link.occurrence_start_ms, 1_790_000_000_000);
        assert_eq!(link.starts_at_ms, 1_790_000_600_000);
        assert_eq!(link.ends_at_ms, 1_790_004_200_000);
        assert_eq!(link.attendees[0].name, "Lucía");
        assert_eq!(link.attendees[0].email, "lucia@example.com");
        assert!(!link.attendees[0].is_self);
        assert!(link.attendees[1].is_self);
        assert_eq!(link.recurring, Some(false));
        assert_eq!(link.check(), Ok(()));
        snapshot.calendar.events[0].recurrence = Some(serde_json::from_value(
            serde_json::json!({"frequency":"weekly","interval":1}),
        )?);
        let Command::MeetingStart {
            event: Some(link), ..
        } = super::map_command(command.clone(), &snapshot.calendar, &clock)?
        else {
            panic!("the repeating event was not linked");
        };
        assert_eq!(link.recurring, Some(true));
        // An observation with no stable identity records without inventing one.
        snapshot.calendar.events[0].calendar_item_id = None;
        assert!(matches!(
            super::map_command(command, &snapshot.calendar, &clock)?,
            Command::MeetingStart {
                event: None,
                duration_minutes: None,
                ..
            }
        ));
        Ok(())
    }

    #[test]
    fn automatic_titles_use_the_local_hour_for_the_tray_dialog_and_calendar()
    -> Result<(), Box<dyn std::error::Error>> {
        let snapshot = crate::AppSnapshot::default();
        for (offset_ms, expected) in [(7_200_000, "Reunión 12:20"), (-14_400_000, "Reunión 06:20")]
        {
            let clock = crate::calendar::LocalClock {
                now_ms: 1_792_405_200_000,
                zone: crate::LocalZone::fixed(offset_ms),
                timezone_name: Ok("fixture".into()),
            };
            for command in [
                RecordingCommand::MeetingStart {
                    title: String::new(),
                },
                serde_json::from_value(serde_json::json!({"command":"meeting_start"}))?,
                serde_json::from_value(
                    serde_json::json!({"command":"meeting_start","title":"","minutes":60,"audio":"microphone_only"}),
                )?,
            ] {
                assert!(
                    matches!(super::map_command(command, &snapshot.calendar, &clock)?,
                    Command::MeetingStart { title, event: None, duration_minutes: None, .. } if title == expected)
                );
            }
        }
        Ok(())
    }

    #[test]
    fn opened_event_survives_a_new_observation_id() -> Result<(), Box<dyn std::error::Error>> {
        let mut snapshot = crate::AppSnapshot::default();
        let opened: crate::CalendarEventSnapshot = serde_json::from_value(serde_json::json!({
            "id": "old-observation", "calendar_item_id": "committee",
            "occurrence_start_ms": 1_790_000_000_000_i64,
            "title": "Comité", "start_ms": 1_790_000_600_000_i64,
            "end_ms": 1_790_004_200_000_i64
        }))?;
        let command: RecordingCommand = serde_json::from_value(serde_json::json!({
            "command": "meeting_start", "event_id": opened.id,
            "event": {
                "calendar_item_id": opened.calendar_item_id,
                "occurrence_start_ms": opened.occurrence_start_ms
            }
        }))?;
        let mut refreshed = opened.clone();
        refreshed.id = "new-observation".into();
        refreshed.title = "Comité actualizado".into();
        snapshot.calendar.events = vec![refreshed];
        let clock = crate::calendar::LocalClock {
            now_ms: 1_790_001_800_000,
            zone: crate::LocalZone::utc(),
            timezone_name: Ok("UTC".into()),
        };
        let mapped = super::map_command(command.clone(), &snapshot.calendar, &clock);
        assert!(
            mapped.is_ok(),
            "refresh must not lose the occurrence: {mapped:?}"
        );
        let Command::MeetingStart {
            title,
            event: Some(link),
            duration_minutes,
            ..
        } = mapped?
        else {
            return Err("the selected occurrence was not linked".into());
        };
        assert_eq!(title, "Comité actualizado");
        assert_eq!(duration_minutes, None);
        assert_eq!(link.calendar_item_id, "committee");
        assert_eq!(link.occurrence_start_ms, 1_790_000_000_000);
        // A read id cannot override the stable reference, even if it is reused.
        let mut other = opened;
        other.occurrence_start_ms = Some(1_790_086_400_000);
        snapshot.calendar.events.insert(0, other);
        assert!(
            matches!(super::map_command(command.clone(), &snapshot.calendar, &clock)?,
            Command::MeetingStart { event: Some(link), .. } if link.occurrence_start_ms == 1_790_000_000_000)
        );
        snapshot.calendar.events.pop();
        let Err(error) = super::map_command(command, &snapshot.calendar, &clock) else {
            return Err("a missing occurrence must not record a different one".into());
        };
        assert_eq!(error.code, "meeting_event_not_found");
        assert!(error.message.contains("committee"));
        assert!(error.message.contains("1790000000000"));
        assert!(error.message.contains("1 eventos"));
        Ok(())
    }

    #[test]
    fn unknown_timezone_keeps_automatic_titles_free_of_an_assumed_hour()
    -> Result<(), Box<dyn std::error::Error>> {
        let snapshot = crate::AppSnapshot::default();
        for timezone_name in [Err("timezone_os_failed".into()), Ok(String::new())] {
            let clock = crate::calendar::LocalClock {
                now_ms: 1_792_405_200_000,
                zone: crate::LocalZone::utc(),
                timezone_name,
            };
            for (title, expected) in [("", "Reunión"), ("Actas", "Actas")] {
                let mapped = super::map_command(
                    RecordingCommand::MeetingStart {
                        title: title.into(),
                    },
                    &snapshot.calendar,
                    &clock,
                )?;
                assert!(
                    matches!(&mapped, Command::MeetingStart { title, .. } if title == expected),
                    "{mapped:?}"
                );
            }
        }
        Ok(())
    }

    #[test]
    fn a_missing_selected_event_names_its_id_and_the_number_read()
    -> Result<(), Box<dyn std::error::Error>> {
        let snapshot = crate::AppSnapshot::default();
        let command = serde_json::from_value(
            serde_json::json!({"command":"meeting_start","event_id":"missing"}),
        )?;
        let Err(error) = super::map_command(
            command,
            &snapshot.calendar,
            &crate::calendar::LocalClock::from_snapshot(&snapshot),
        ) else {
            return Err("a missing event cannot become an unlinked meeting".into());
        };
        assert_eq!(error.code, "meeting_event_not_found");
        assert!(error.message.contains("missing"));
        assert!(error.message.contains("0 eventos"));
        Ok(())
    }
}

#[cfg(test)]
mod topic_tests {
    use super::*;
    use orbit_core::{
        CapturePhase, Recording,
        topics::{TopicData, TopicFrame, TopicId},
    };

    fn recording_badge(
        title: &str,
        event_title: &str,
        now: i64,
        start: i64,
        end: i64,
    ) -> Result<Option<crate::BriefingRecordingState>, serde_json::Error> {
        let mut current = crate::projection::test_support::recording(CapturePhase::Recording);
        current.title = title.into();
        let mut topics = crate::topics::TopicCache::default();
        topics.observe_at(
            serde_json::from_value(serde_json::json!({
                "epoch":"badge", "revision":1,
                "data":{"topic":"recording","knowledge":{"state":"present","as_of_ms":now,"value":{
                    "recording":current, "recording_signal":null, "pending_jobs":0, "offline":false,
                    "applications":{"state":"unknown","why":"not_observed_yet"}
                }}}
            }))?,
            Some(now),
        );
        let event = serde_json::from_value(serde_json::json!({
            "id":"event-1", "title":event_title, "start_ms":start, "end_ms":end
        }))?;
        let local = crate::AppSnapshot::default();
        let input = FeatureInput::with_clock(
            &local,
            &topics,
            crate::calendar::LocalClock {
                now_ms: now,
                zone: crate::LocalZone::utc(),
                timezone_name: Ok("UTC".into()),
            },
        );
        Ok(event_recording(&event, &input))
    }

    #[test]
    fn daily_recording_marks_only_the_occurrence_in_progress() -> Result<(), serde_json::Error> {
        let now = 1_792_405_200_000;
        for (start, end, expected) in [
            (now, now + 60_000, crate::BriefingRecordingState::Recording),
            (
                now + 86_400_000,
                now + 90_000_000,
                crate::BriefingRecordingState::None,
            ),
            (now - 60_000, now, crate::BriefingRecordingState::None),
        ] {
            assert_eq!(
                recording_badge("Daily", "Daily", now, start, end)?,
                Some(expected),
                "{start}..{end} at {now}"
            );
        }
        Ok(())
    }

    #[test]
    fn manual_recording_does_not_mark_a_meeting_two_days_ahead() -> Result<(), serde_json::Error> {
        let now = 1_792_405_200_000;
        assert_eq!(
            recording_badge(
                "Reunión 12:20",
                "Reunión",
                now,
                now + 2 * 86_400_000,
                now + 2 * 86_400_000 + 60_000
            )?,
            Some(crate::BriefingRecordingState::None)
        );
        assert_eq!(
            recording_badge("Reunión 12:20", "Reunión", now, now - 60_000, now + 60_000)?,
            Some(crate::BriefingRecordingState::Recording)
        );
        Ok(())
    }

    #[test]
    fn topic_starting_overrides_a_stale_legacy_phase() -> Result<(), serde_json::Error> {
        let snapshot = crate::AppSnapshot::empty(1, crate::DaemonConnection::Ready);
        let recording = Recording {
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
            muted_spans: Vec::new(),
        };
        let frame: TopicFrame = serde_json::from_value(serde_json::json!({
            "epoch": "synthetic", "revision": 1,
            "data": {"topic": "recording", "knowledge": {
                "state": "present", "value": {
                    "recording": recording,
                    "recording_signal": null,
                    "pending_jobs": 0,
                    "offline": true,
                    "applications": {"state": "unknown", "why": "not_observed_yet"}
                },
                "as_of_ms": null
            }}
        }))?;
        let mut topics = crate::topics::TopicCache::default();
        topics.observe_at(frame, Some(50));
        let view = project(&FeatureInput::new(&snapshot, &topics));
        assert_eq!(view.meeting.phase, crate::MeetingPhase::Starting);
        Ok(())
    }

    #[test]
    fn unobserved_inventory_is_not_presented_as_empty() {
        let mut snapshot = crate::AppSnapshot::empty(1, crate::DaemonConnection::Ready);
        snapshot.meeting.applications_state = crate::MeetingApplicationsState::Empty;
        let mut topics = crate::topics::TopicCache::default();
        topics.observe_at(
            TopicFrame {
                epoch: "synthetic".into(),
                revision: 1,
                data: TopicData::not_observed(TopicId::Recording),
            },
            Some(50),
        );
        let view = project(&FeatureInput::new(&snapshot, &topics));
        assert_eq!(
            view.meeting.applications_state,
            crate::MeetingApplicationsState::Unavailable
        );
        assert!(
            view.meeting
                .applications_detail
                .as_deref()
                .is_some_and(|text| text.contains("observado"))
        );
    }

    /// The offer the daemon published reaches the panel, and its
    /// «Grabar» names that offer.
    #[test]
    fn a_published_offer_reaches_the_record_command() -> Result<(), Box<dyn std::error::Error>> {
        let snapshot = crate::AppSnapshot::empty(1, crate::DaemonConnection::Ready);
        let frame: TopicFrame = serde_json::from_value(serde_json::json!({
            "epoch": "synthetic", "revision": 1,
            "data": {"topic": "recording", "knowledge": {
                "state": "present", "as_of_ms": null,
                "value": {
                    "recording": null, "recording_signal": null,
                    "pending_jobs": 0, "offline": false,
                    "call": {"preference": "notify", "panel": {"state": "offer",
                        "call_id": "call-1", "application": "zoom.us",
                        "event_title": "Comité"}},
                    "applications": {"state": "unknown", "why": "not_observed_yet"}
                }
            }}
        }))?;
        let mut topics = crate::topics::TopicCache::default();
        topics.observe_at(frame, Some(50));
        let view = project(&FeatureInput::new(&snapshot, &topics));
        assert_eq!(
            view.meeting.call_preference,
            Some(crate::CallPreference::Notify)
        );
        let Some(crate::CallPanelSnapshot::Offer {
            call_id,
            application,
            event_title,
        }) = view.meeting.call_panel.clone()
        else {
            return Err("published offer missing".into());
        };
        assert_eq!(application, "zoom.us");
        assert_eq!(event_title.as_deref(), Some("Comité"));
        assert!(matches!(
            serde_json::from_value::<orbit_core::protocol::Command>(serde_json::json!({"command":"call_record", "call_id":call_id})),
            Ok(orbit_core::protocol::Command::CallRecord { call_id }) if call_id == "call-1"
        ));
        Ok(())
    }

    /// A daemon that predates the call detection publishes no
    /// preference: unknown, never «Avisarme».
    #[test]
    fn an_absent_call_view_is_an_unknown_preference() -> Result<(), serde_json::Error> {
        let snapshot = crate::AppSnapshot::empty(1, crate::DaemonConnection::Ready);
        let frame: TopicFrame = serde_json::from_value(serde_json::json!({
            "epoch": "synthetic", "revision": 1,
            "data": {"topic": "recording", "knowledge": {
                "state": "present", "as_of_ms": null,
                "value": {
                    "recording": null, "recording_signal": null,
                    "pending_jobs": 0, "offline": false,
                    "applications": {"state": "unknown", "why": "not_observed_yet"}
                }
            }}
        }))?;
        let mut topics = crate::topics::TopicCache::default();
        topics.observe_at(frame, Some(50));
        let view = project(&FeatureInput::new(&snapshot, &topics));
        assert_eq!(view.meeting.call_preference, None);
        assert_eq!(view.meeting.call_panel, None);
        Ok(())
    }

    #[test]
    fn stale_recording_topic_exposes_its_received_time() -> Result<(), serde_json::Error> {
        let snapshot = crate::AppSnapshot::empty(1, crate::DaemonConnection::Ready);
        let frame: TopicFrame = serde_json::from_value(serde_json::json!({
            "epoch": "synthetic", "revision": 1,
            "data": {"topic": "recording", "knowledge": {
                "state": "present", "as_of_ms": null,
                "value": {
                    "recording": null, "recording_signal": null,
                    "pending_jobs": 0, "offline": false,
                    "applications": {"state": "unknown", "why": "not_observed_yet"}
                }
            }}
        }))?;
        let mut topics = crate::topics::TopicCache::default();
        topics.observe_at(frame, Some(12_000));
        topics.ended("El daemon terminó".into());
        let view = project(&FeatureInput::new(&snapshot, &topics));
        let json = serde_json::to_value(view)?;
        assert_eq!(
            json["freshness"],
            serde_json::json!({"state": "stale", "received_at_ms": 12_000})
        );
        Ok(())
    }

    #[test]
    fn no_recording_topic_does_not_claim_current_freshness() -> Result<(), serde_json::Error> {
        let snapshot = crate::AppSnapshot::empty(1, crate::DaemonConnection::Ready);
        let topics = crate::topics::TopicCache::default();
        let view = project(&FeatureInput::new(&snapshot, &topics));
        assert!(serde_json::to_value(view)?.get("freshness").is_none());
        Ok(())
    }

    #[test]
    fn inventory_refusal_keeps_starting_phase_and_the_call_panel() -> Result<(), serde_json::Error>
    {
        let snapshot = crate::AppSnapshot::empty(1, crate::DaemonConnection::Ready);
        let recording = Recording {
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
            muted_spans: Vec::new(),
        };
        let frame: TopicFrame = serde_json::from_value(serde_json::json!({
            "epoch": "synthetic", "revision": 1,
            "data": {"topic": "recording", "knowledge": {
                "state": "present", "as_of_ms": null,
                "value": {
                    "recording": recording, "recording_signal": null,
                    "pending_jobs": 0, "offline": false,
                    "call": {"preference": "record", "panel": {"state": "ending",
                        "recording_id": "recording-1", "application": "zoom.us",
                        "ends_at_ms": 60_000}},
                    "applications": {"state": "absent", "refusal": {
                        "code": "too_many", "resource": "source", "count": 33, "max": 32}}
                }
            }}
        }))?;
        let mut topics = crate::topics::TopicCache::default();
        topics.observe_at(frame, Some(50));
        let view = project(&FeatureInput::new(&snapshot, &topics));
        assert_eq!(view.meeting.phase, crate::MeetingPhase::Starting);
        assert_eq!(
            view.meeting.call_panel,
            Some(crate::CallPanelSnapshot::Ending {
                recording_id: "recording-1".into(),
                application: "zoom.us".into(),
                ends_at_ms: 60_000,
            })
        );
        assert_eq!(
            view.meeting.applications_state,
            crate::MeetingApplicationsState::Unavailable
        );
        assert!(
            view.meeting
                .applications_detail
                .as_deref()
                .is_some_and(|detail| detail.contains("33") && detail.contains("32"))
        );
        Ok(())
    }
}
