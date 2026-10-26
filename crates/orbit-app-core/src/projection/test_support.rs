use super::*;
pub(crate) use orbit_core::{
    Recording,
    audio_signal::{SignalCondition, TrackSignalView},
    calendar::{CalendarFieldStatus, CalendarInvitee, CalendarOccurrence, CalendarReference},
};
pub(crate) use serde_json::{Value, json};

pub(crate) fn recording(phase: CapturePhase) -> Recording {
    Recording {
        id: "recording-1".into(),
        title: "Revisión semanal".into(),
        started_at_ms: 1_234,
        ended_at_ms: None,
        expires_at_ms: 61_234,
        application_pid: None,
        audio: orbit_core::protocol::MeetingAudio::System,
        phase,
        sealed_chunks: 0,
        gaps: 0,
        tracks: Vec::new(),
        finalizing_since_ms: None,
        ceiling_notice_given: false,
        muted_spans: Vec::new(),
    }
}

pub(crate) fn track(condition: SignalCondition, frames: u64, elapsed_ms: u64) -> TrackSignalView {
    TrackSignalView {
        condition,
        received_frames: frames,
        since_last_samples_ms: Some(elapsed_ms),
        low_signal_ms: 0,
        speech_quiet_ms: None,
        voiced_frames: None,
    }
}

pub(crate) fn calendar_reference(id: &str) -> CalendarReference {
    CalendarReference {
        calendar_id: id.into(),
        source_id: "fixture".into(),
    }
}

pub(crate) fn calendar_occurrence(
    observation_id: &str,
    calendar_id: &str,
    title: &str,
    start_ms: i64,
    end_ms: i64,
) -> CalendarOccurrence {
    CalendarOccurrence {
        observation_id: observation_id.into(),
        calendar: calendar_reference(calendar_id),
        calendar_item_id: format!("item-{observation_id}"),
        event_id: Some(format!("event-{observation_id}")),
        original_occurrence_unix_ms: None,
        start_unix_ms: start_ms,
        end_unix_ms: end_ms,
        event_timezone: Some("Europe/Madrid".into()),
        interpreted_timezone: "Europe/Madrid".into(),
        is_floating: false,
        is_all_day: false,
        is_detached: false,
        title: title.into(),
        status: "confirmed".into(),
        last_modified_unix_ms: None,
        invitees: None,
        invitees_status: CalendarFieldStatus::NotRequested,
        event_url: None,
        event_url_status: CalendarFieldStatus::NotRequested,
        location: None,
        notes: None,
        notes_left_out: None,
        organizer: None,
        availability: None,
        recurrence: None,
        can_edit: None,
        alarms_minutes: None,
        alarms_not_editable: None,
    }
}

pub(crate) const DAY_2026_09_16_UTC_MS: i64 = 1_789_516_800_000;
pub(crate) const MADRID_OFFSET_MS: i64 = 7_200_000;
pub(crate) const HOUR_MS: i64 = 3_600_000;

/// Europe/Madrid from 2025 to 2027 as the tz database has it: CET (+1),
/// and CEST (+2) from the last Sunday of March to the last Sunday of
/// October, changing at 01:00 UTC. The zone of the person the
/// projection is for, never the zone of the machine running the suite.
pub(crate) fn madrid() -> LocalZone {
    let transitions = [
        (1_743_296_400_000, 2 * HOUR_MS), // 2025-03-30T01:00Z
        (1_761_440_400_000, HOUR_MS),     // 2025-10-26T01:00Z
        (1_774_746_000_000, 2 * HOUR_MS), // 2026-03-29T01:00Z
        (1_792_890_000_000, HOUR_MS),     // 2026-10-25T01:00Z
        (1_806_195_600_000, 2 * HOUR_MS), // 2027-03-28T01:00Z
        (1_824_944_400_000, HOUR_MS),     // 2027-10-31T01:00Z
    ];
    LocalZone::with_transitions(
        HOUR_MS,
        transitions
            .into_iter()
            .map(|(at_ms, offset_ms)| crate::ZoneTransition { at_ms, offset_ms })
            .collect(),
    )
}

/// `hour:minute` on 16 September 2026 in Madrid (CEST, two hours ahead of
/// UTC). It used to add the offset instead of subtracting it, which only
/// held while the projection read every hour in UTC.
pub(crate) fn at_local(hour: u32, minute: u32) -> i64 {
    DAY_2026_09_16_UTC_MS - MADRID_OFFSET_MS + i64::from(hour * 60 + minute) * 60_000
}

pub(crate) fn briefing_event(
    event_id: &str,
    title: &str,
    start_ms: i64,
    end_ms: i64,
    invitees: &[&str],
    url: Option<&str>,
) -> CalendarOccurrence {
    CalendarOccurrence {
        observation_id: format!("observation-{event_id}"),
        calendar: CalendarReference {
            calendar_id: "work".into(),
            source_id: "local".into(),
        },
        calendar_item_id: format!("item-{event_id}"),
        event_id: Some(event_id.into()),
        original_occurrence_unix_ms: None,
        start_unix_ms: start_ms,
        end_unix_ms: end_ms,
        event_timezone: None,
        interpreted_timezone: "Europe/Madrid".into(),
        is_floating: false,
        is_all_day: false,
        is_detached: false,
        title: title.into(),
        status: "confirmed".into(),
        last_modified_unix_ms: None,
        invitees: (!invitees.is_empty()).then(|| {
            invitees
                .iter()
                .map(|name| CalendarInvitee {
                    name: Some((*name).into()),
                    participant_url: format!("mailto:{name}@company.example.test"),
                    email: Some(format!("{name}@company.example.test")),
                    role: "required".into(),
                    response_status: "accepted".into(),
                    participant_type: "person".into(),
                    is_current_user: *name == "alina.perez",
                })
                .collect()
        }),
        invitees_status: CalendarFieldStatus::Provided,
        event_url: url.map(str::to_owned),
        event_url_status: CalendarFieldStatus::Provided,
        location: None,
        notes: None,
        notes_left_out: None,
        organizer: None,
        availability: None,
        recurrence: None,
        can_edit: None,
        alarms_minutes: None,
        alarms_not_editable: None,
    }
}

pub(crate) struct BriefingFixture {
    pub events: Vec<CalendarOccurrence>,
    /// The notes list the client read.
    pub recordings: Vec<crate::NoteListItem>,
    /// The next meeting's series, as the client read it.
    pub series: Option<crate::SeriesPreviousRead>,
    /// A meeting's reminder, as the client read it from the hub.
    pub reminder: Option<crate::ReminderRead>,
}

pub(crate) fn briefing_fixture(events: Vec<CalendarOccurrence>) -> BriefingFixture {
    BriefingFixture {
        events,
        recordings: Vec::new(),
        series: None,
        reminder: None,
    }
}

pub(crate) fn recording_summary(
    recording_id: &str,
    title: &str,
    started_at_ms: i64,
) -> crate::NoteListItem {
    crate::NoteListItem {
        recording_id: recording_id.into(),
        title: title.into(),
        started_at_ms,
        duration_ms: 900_000,
        state: crate::NoteListState::Ready,
        group: crate::NoteDayGroup::Yesterday,
        headline: None,
    }
}

pub(crate) fn projected_briefing(fixture: &BriefingFixture, now_ms: i64) -> Value {
    use orbit_core::{
        reads::ReadRequest,
        topics::{Knowledge, TopicData, TopicFrame, TopicId, agenda, notes},
    };
    let zone = madrid();
    let day = zone.day(now_ms);
    let start_ms = zone.start_of_day(day);
    let end_ms = zone.start_of_day(day + 1);
    let events = fixture.events.as_slice();
    let mut topics = crate::topics::TopicCache::default();
    let head = agenda::Value {
        invalidation: 0,
        calendars: None,
        read_window: Some(orbit_core::calendar::CalendarWindow {
            start_unix_ms: start_ms,
            end_unix_ms: end_ms,
        }),
        read_id: Some("briefing-fixture".into()),
        timezone: Some("Europe/Madrid".into()),
        writes: Vec::new(),
    };
    topics.observe_at(
        TopicFrame {
            epoch: "briefing-fixture".into(),
            revision: 1,
            data: TopicData::Agenda(Knowledge::Present {
                value: head,
                as_of_ms: None,
            }),
        },
        Some(now_ms),
    );
    let items = events
        .iter()
        .map(|event| serde_json::to_value(event).expect("event fixture"))
        .collect::<Vec<_>>();
    assert!(topics.remember_read(
        &ReadRequest::AgendaRange {
            start_ms,
            end_ms,
            cursor: None
        },
        TopicId::Agenda,
        Knowledge::Present {
            value: orbit_core::topics::Page {
                total: items.len() as u64,
                items,
                next: None,
                revision: 1,
                store_version: None,
            },
            as_of_ms: None
        }
    ));
    topics.observe_at(
        TopicFrame {
            epoch: "briefing-fixture".into(),
            revision: 2,
            data: TopicData::Notes(Knowledge::Present {
                value: notes::Value::default(),
                as_of_ms: None,
            }),
        },
        Some(now_ms),
    );
    let status = serde_json::from_value(json!({
        "hub": "online", "account": { "device_id_short": "test" },
        "privacy_stopped": false, "helper_connected": true
    }))
    .expect("paired status fixture");
    topics.observe_at(
        TopicFrame {
            epoch: "briefing-fixture".into(),
            revision: 4,
            data: TopicData::Status(Knowledge::Present {
                value: status,
                as_of_ms: None,
            }),
        },
        Some(now_ms),
    );
    let mut local = crate::AppSnapshot::empty(12, DaemonConnection::Ready);
    local.calendar.authorization = crate::CalendarAuthorizationSnapshot::Granted;
    if let Some(notes) = local.notes.as_mut() {
        notes.list = fixture.recordings.clone();
        notes.series_previous = fixture.series.clone().map(Box::new);
        notes.reminder = fixture.reminder.clone().map(Box::new);
        notes.index = Some(Box::new(crate::NotesIndexSnapshot {
            state: crate::NotesIndexState::Available,
            more: false,
            detail: None,
        }));
    }
    let input = crate::features::FeatureInput::with_clock(
        &local,
        &topics,
        crate::calendar::LocalClock {
            now_ms,
            zone,
            timezone_name: Ok("Europe/Madrid".into()),
        },
    );
    serde_json::to_value(crate::features::briefing::project(&input).expect("briefing view"))
        .expect("typed briefing fixture serializes")
}
pub(crate) const UTC_2026_09_15_T0630Z: i64 = 1_789_453_800_000; // 08:30 CEST, Tue 15 Sep
pub(crate) const UTC_2026_09_15_T2130Z: i64 = 1_789_507_800_000; // 23:30 CEST, Tue 15 Sep
pub(crate) const UTC_2026_09_15_T2215Z: i64 = 1_789_510_500_000; // 00:15 CEST, Wed 16 Sep
pub(crate) const UTC_2026_09_15_T2230Z: i64 = 1_789_511_400_000; // 00:30 CEST, Wed 16 Sep
pub(crate) const UTC_2026_09_15_T2300Z: i64 = 1_789_513_200_000; // 01:00 CEST, Wed 16 Sep
pub(crate) const UTC_2026_09_16_T0500Z: i64 = 1_789_534_800_000; // 07:00 CEST, Wed 16 Sep
pub(crate) const UTC_2026_09_16_T0700Z: i64 = 1_789_542_000_000; // 09:00 CEST, Wed 16 Sep
pub(crate) const UTC_2026_09_16_T0800Z: i64 = 1_789_545_600_000; // 10:00 CEST, Wed 16 Sep
pub(crate) const UTC_2026_10_24_T2230Z: i64 = 1_792_881_000_000; // 00:30 CEST, Sun 25 Oct
pub(crate) const UTC_2026_10_25_T0030Z: i64 = 1_792_888_200_000; // 02:30 CEST, the first one
pub(crate) const UTC_2026_10_25_T0130Z: i64 = 1_792_891_800_000; // 02:30 CET, the second one
pub(crate) const UTC_2026_10_25_T0900Z: i64 = 1_792_918_800_000; // 10:00 CET, Sun 25 Oct
pub(crate) const UTC_2026_10_25_T2230Z: i64 = 1_792_967_400_000; // 23:30 CET, Sun 25 Oct
pub(crate) const UTC_2026_10_25_T2330Z: i64 = 1_792_971_000_000; // 00:30 CET, Mon 26 Oct
pub(crate) const HALF_HOUR_MS: i64 = 30 * 60_000;
