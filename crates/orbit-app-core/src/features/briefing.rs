use super::FeatureInput;
use crate::{
    AppSnapshot, BriefingAgendaSnapshot, BriefingAttendeeSnapshot, BriefingGreetingSnapshot,
    BriefingNextSnapshot, BriefingPreviousSnapshot, BriefingRecordingState, BriefingSnapshot,
    BriefingSourceState, BriefingSourcesSnapshot, BriefingSummarySnapshot, CalendarEventSnapshot,
    CalendarEventStatus, CalendarReadingState, LocalZone,
    topics::{CachedTopic, TopicFreshness},
};
use orbit_core::topics::{Knowledge, Pending, TopicData, TopicId};
use std::collections::HashMap;

pub type View = Option<BriefingSnapshot>;

pub fn project(input: &FeatureInput<'_>) -> View {
    let published_zone = input
        .clock
        .timezone_name
        .as_ref()
        .ok()
        .map(|_| &input.clock.zone);
    let now_ms = input.clock.now_ms;
    let (calendar, page_loaded) = super::agenda::project_with_read(input);
    let agenda_cell = input.topics.topic(TopicId::Agenda);
    let agenda_topic = agenda_cell.and_then(|cell| {
        if let TopicData::Agenda(knowledge) = &cell.knowledge {
            Some(knowledge)
        } else {
            None
        }
    });
    let agenda_state = if let Some(stale) = stale_state(agenda_cell) {
        stale
    } else {
        match agenda_topic {
            Some(Knowledge::Present { .. }) if published_zone.is_none() => {
                BriefingSourceState::Unknown {
                    cause: "local_zone_unavailable".into(),
                }
            }
            Some(Knowledge::Present { value, .. })
                if page_loaded
                    && (calendar.reading.state != CalendarReadingState::Unavailable
                        || matches!(
                            calendar.reading.blocker,
                            Some(
                                crate::CalendarReadingBlocker::NotPaired
                                    | crate::CalendarReadingBlocker::MacDisconnected
                            )
                        )) =>
            {
                published_zone.map_or_else(
                    || BriefingSourceState::Unknown {
                        cause: "local_zone_unavailable".into(),
                    },
                    |zone| {
                        let day = zone.day(now_ms);
                        let today_start = zone.start_of_day(day);
                        let today_end = zone.start_of_day(day.saturating_add(1));
                        match value.read_window {
                            Some(window)
                                if window.start_unix_ms <= today_start
                                    && window.end_unix_ms >= today_end =>
                            {
                                BriefingSourceState::Present
                            }
                            Some(window) => BriefingSourceState::Unknown {
                                cause: format!(
                                    "agenda_window_excludes_today:window_start_ms={}:window_end_ms={}:today_start_ms={today_start}:today_end_ms={today_end}",
                                    window.start_unix_ms, window.end_unix_ms
                                ),
                            },
                            None => BriefingSourceState::Unknown {
                                cause: "agenda_window_missing".into(),
                            },
                        }
                    },
                )
            }
            Some(Knowledge::Present { .. }) => {
                let cause = if page_loaded || calendar.reading.state == CalendarReadingState::Unavailable {
                    calendar.reading.detail.clone().unwrap_or_else(|| "agenda_page_pending".into())
                } else {
                    "agenda_page_pending".into()
                };
                if calendar.reading.state == CalendarReadingState::Unavailable {
                    BriefingSourceState::Absent { cause }
                } else {
                    BriefingSourceState::Unknown { cause }
                }
            }
            Some(knowledge) => source_state(knowledge),
            None => unknown_topic(),
        }
    };
    let notes_cell = input.topics.topic(TopicId::Notes);
    let notes_topic = notes_cell.and_then(|cell| {
        if let TopicData::Notes(knowledge) = &cell.knowledge {
            Some(knowledge)
        } else {
            None
        }
    });
    // The notes list is the hub's, read by the client into the
    // local snapshot (`client::notes`).
    let notes_list = input.local.notes.as_ref().and_then(|notes| {
        notes
            .index
            .as_ref()
            .filter(|index| index.state == crate::NotesIndexState::Available)
            .map(|_| notes.list.as_slice())
    });
    let notes_state = if let Some(stale) = stale_state(notes_cell) {
        stale
    } else {
        match notes_topic {
            Some(Knowledge::Present { .. }) if notes_list.is_some() => BriefingSourceState::Present,
            Some(Knowledge::Present { .. }) => match input
                .local
                .notes
                .as_ref()
                .and_then(|notes| notes.index.as_ref())
                .filter(|index| index.state == crate::NotesIndexState::Failed)
            {
                Some(index) => BriefingSourceState::Absent {
                    cause: index
                        .detail
                        .clone()
                        .unwrap_or_else(|| "notes_list_failed".into()),
                },
                None => BriefingSourceState::Unknown {
                    cause: "notes_list_pending".into(),
                },
            },
            Some(knowledge) => source_state(knowledge),
            None => unknown_topic(),
        }
    };
    let recording_cell = input.topics.topic(TopicId::Recording);
    let recording_topic = recording_cell.and_then(|cell| {
        if let TopicData::Recording(knowledge) = &cell.knowledge {
            Some(knowledge)
        } else {
            None
        }
    });
    let recording_state = stale_state(recording_cell)
        .unwrap_or_else(|| recording_topic.map_or_else(unknown_topic, source_state));
    let note_lookup = notes_list.map(NoteLookup::new);
    let events = if agenda_state == BriefingSourceState::Present {
        published_zone.map_or_else(Vec::new, |zone| {
            let day = zone.day(now_ms);
            let mut events = calendar
                .events
                .iter()
                .filter(|event| {
                    event.all_day != Some(true)
                        && event.status != Some(CalendarEventStatus::Cancelled)
                        && zone.day(event.start_ms) == day
                })
                .collect::<Vec<_>>();
            events.sort_by_key(|event| event.start_ms);
            events
        })
    } else {
        Vec::new()
    };
    let mut upcoming = events
        .iter()
        .copied()
        .map(|event| next_event(event, note_lookup.as_ref()))
        .collect::<Vec<_>>();
    let notes = input.local.notes.as_ref();
    let series = notes.and_then(|notes| notes.series_previous.as_deref());
    let reminder = notes.and_then(|notes| notes.reminder.as_deref());
    // The next meeting is also the day's entry for it, the same in both.
    let next = upcoming
        .iter_mut()
        .find(|event| event.end_ms > now_ms)
        .map(|next| {
            *next = with_series(next.clone(), series);
            next.reminder = reminder
                .filter(|read| read.series_key == next.series_key)
                .map(briefing_reminder);
            next.clone()
        });
    let agenda = events.iter().map(|event| agenda_event(event)).collect();
    let greeting = BriefingGreetingSnapshot {
        name: input
            .local
            .profile
            .as_ref()
            .and_then(|profile| profile.name.clone()),
    };
    Some(BriefingSnapshot {
        greeting,
        summary: BriefingSummarySnapshot {
            meetings_today: events.len(),
            focus_minutes: if agenda_state == BriefingSourceState::Present {
                published_zone.map_or(0, |zone| focus_minutes(&events, now_ms, zone))
            } else {
                0
            },
        },
        next,
        upcoming: Some(upcoming),
        agenda,
        sources: Some(BriefingSourcesSnapshot {
            agenda: agenda_state,
            notes: notes_state,
            recording: recording_state,
        }),
    })
}

fn unknown_topic() -> BriefingSourceState {
    BriefingSourceState::Unknown {
        cause: "topic_not_observed".into(),
    }
}

fn stale_state(cell: Option<&CachedTopic>) -> Option<BriefingSourceState> {
    let cell = cell?;
    match &cell.freshness {
        TopicFreshness::Current => None,
        TopicFreshness::Stale { since_ms } => Some(BriefingSourceState::Unknown {
            cause: since_ms.map_or_else(
                || "topic_stale".into(),
                |since| format!("topic_stale_since:{since}"),
            ),
        }),
    }
}

fn source_state<T>(knowledge: &Knowledge<T>) -> BriefingSourceState {
    match knowledge {
        Knowledge::Present { .. } => BriefingSourceState::Present,
        Knowledge::Absent { refusal } => BriefingSourceState::Absent {
            cause: refusal.to_string(),
        },
        Knowledge::Unknown { why } => BriefingSourceState::Unknown {
            cause: match why {
                Pending::NotObservedYet => "topic_not_observed".into(),
                Pending::HelperStarting => "helper_starting".into(),
                Pending::Reading { since_ms } => format!("reading_since:{since_ms}"),
                Pending::Transcription {
                    completed,
                    pending,
                    failed,
                    queued,
                } => format!(
                    "transcription_completed:{completed}_pending:{pending}_failed:{failed}_queued:{queued}"
                ),
                Pending::DaemonStarting => "daemon_starting".into(),
            },
        },
    }
}

/// A title's words, as the hub matches them too (`orbit_core::series_title`).
pub(super) fn series_key(value: &str) -> String {
    orbit_core::series_title::title_key(value)
}

struct NoteLookup<'a> {
    by_series: HashMap<String, Vec<&'a crate::NoteListItem>>,
}

impl<'a> NoteLookup<'a> {
    fn new(entries: &'a [crate::NoteListItem]) -> Self {
        let mut lookup = Self {
            by_series: HashMap::new(),
        };
        for entry in entries {
            lookup
                .by_series
                .entry(series_key(&entry.title))
                .or_default()
                .push(entry);
        }
        for series in lookup.by_series.values_mut() {
            series.sort_by_key(|entry| entry.started_at_ms);
        }
        lookup
    }

    fn previous(&self, event: &CalendarEventSnapshot) -> Option<&'a crate::NoteListItem> {
        let series = self.by_series.get(&series_key(&event.title))?;
        let index = series.partition_point(|entry| entry.started_at_ms < event.start_ms);
        index
            .checked_sub(1)
            .and_then(|previous| series.get(previous).copied())
    }
}

fn next_event(
    event: &CalendarEventSnapshot,
    lookup: Option<&NoteLookup<'_>>,
) -> BriefingNextSnapshot {
    let previous = lookup
        .and_then(|lookup| lookup.previous(event))
        .map(|entry| BriefingPreviousSnapshot {
            recording_id: entry.recording_id.clone(),
            // The list gives the title; the summary is in the note itself.
            title: Some(entry.title.clone()),
            summary: None,
            open_items: Vec::new(),
        });
    let colors = ["orange", "blue", "accent", "green"];
    BriefingNextSnapshot {
        event_id: event.id.clone(),
        event: event
            .calendar_item_id
            .as_ref()
            .zip(event.occurrence_start_ms)
            .map(|(id, start)| crate::MeetingEventReference {
                calendar_item_id: id.clone(),
                occurrence_start_ms: start,
            }),
        title: event.title.clone(),
        start_ms: event.start_ms,
        end_ms: event.end_ms,
        attendees: event
            .attendees
            .as_deref()
            .unwrap_or_default()
            .iter()
            .enumerate()
            .map(|(index, attendee)| BriefingAttendeeSnapshot {
                name: attendee.name.clone(),
                initial: attendee
                    .name
                    .chars()
                    .find(|character| character.is_alphanumeric())
                    .map(|character| character.to_uppercase().collect())
                    .unwrap_or_default(),
                color: colors[index % colors.len()].into(),
            })
            .collect(),
        url: event.url.clone(),
        series_key: series_key(&event.title),
        previous,
        previous_actions: None,
        reminder: None,
        recording: event
            .recording
            .clone()
            .unwrap_or(BriefingRecordingState::None),
    }
}

/// The next meeting with what its calendar series says of the earlier one:
/// its note, and the tasks Hoy shows as «Pendiente de la última vez». A
/// series read for another meeting, or none yet, is unknown; the hub's
/// «no earlier meeting» is a fact. The title match stays the earlier note
/// when the series has none or cannot be read.
fn with_series(
    mut next: BriefingNextSnapshot,
    read: Option<&crate::SeriesPreviousRead>,
) -> BriefingNextSnapshot {
    use crate::{BriefingActionSnapshot, BriefingPreviousActions};
    let Some(series) = next.event.as_ref().map(|event| &event.calendar_item_id) else {
        return next;
    };
    let read =
        read.filter(|read| read.calendar_item_id == *series && read.before_ms == next.start_ms);
    let actions = match read.map(|read| &read.answer) {
        None => BriefingPreviousActions::Unknown {
            cause: "series_not_read".into(),
        },
        Some(Err(cause)) => BriefingPreviousActions::Unknown {
            cause: cause.clone(),
        },
        Some(Ok(answer)) => match &answer.previous {
            None => BriefingPreviousActions::NoPreviousMeeting,
            Some(meeting) => {
                let note = meeting.note.as_ref();
                next.previous = Some(BriefingPreviousSnapshot {
                    recording_id: meeting.recording_id.clone(),
                    title: Some(meeting.title.clone()),
                    summary: note.and_then(|note| note.short_summary.clone()),
                    open_items: Vec::new(),
                });
                match note.and_then(|note| Some((note.revision, note.action_items.as_ref()?))) {
                    Some((revision, items)) => BriefingPreviousActions::Known {
                        recording_id: meeting.recording_id.clone(),
                        note_revision: revision,
                        items: items
                            .iter()
                            .map(|item| BriefingActionSnapshot {
                                id: item.id.clone(),
                                owner: item.owner.clone(),
                                text: item.text.clone(),
                                due: item.due.clone(),
                                done: item.done,
                            })
                            .collect(),
                    },
                    None => BriefingPreviousActions::NoteNotWritten {
                        recording_id: meeting.recording_id.clone(),
                        note_state: note.and_then(|note| {
                            serde_json::to_value(note.state)
                                .ok()
                                .and_then(|state| state.as_str().map(str::to_owned))
                        }),
                    },
                }
            }
        },
    };
    next.previous_actions = Some(actions);
    next
}

/// The hub's reminder of the next meeting, or why it is not known.
fn briefing_reminder(read: &crate::ReminderRead) -> crate::BriefingReminder {
    match &read.answer {
        Ok(Some(reminder)) => crate::BriefingReminder::Known {
            text: reminder.text.clone(),
            edited_at_ms: reminder.edited_at_ms,
        },
        Ok(None) => crate::BriefingReminder::None,
        Err(cause) => crate::BriefingReminder::Unknown {
            cause: cause.clone(),
        },
    }
}

fn agenda_event(event: &CalendarEventSnapshot) -> BriefingAgendaSnapshot {
    BriefingAgendaSnapshot {
        event_id: event.id.clone(),
        start_ms: event.start_ms,
        title: event.title.clone(),
        color: event
            .calendar_id
            .as_deref()
            .map_or_else(|| "gray".into(), crate::projection::stable_calendar_color),
        people_label: String::new(),
        minutes: u16::try_from(
            event
                .end_ms
                .saturating_sub(event.start_ms)
                .div_euclid(60_000),
        )
        .unwrap_or(u16::MAX),
        people_count: event.attendees.as_ref().map(Vec::len),
    }
}

fn focus_minutes(events: &[&CalendarEventSnapshot], now_ms: i64, zone: &LocalZone) -> u16 {
    let midnight = zone.day(now_ms).saturating_mul(86_400_000);
    let start = zone.instant_at(midnight.saturating_add(9 * 60 * 60_000));
    let end = zone.instant_at(midnight.saturating_add(19 * 60 * 60_000));
    let mut occupied = events
        .iter()
        .filter_map(|event| {
            let from = event.start_ms.max(start);
            let to = event.end_ms.saturating_add(10 * 60_000).min(end);
            (from < to).then_some((from, to))
        })
        .collect::<Vec<_>>();
    occupied.sort_by_key(|interval| interval.0);
    let mut used = 0_i64;
    let mut current: Option<(i64, i64)> = None;
    for (from, to) in occupied {
        current = match current {
            Some((begin, finish)) if from <= finish => Some((begin, finish.max(to))),
            Some((begin, finish)) => {
                used = used.saturating_add(finish.saturating_sub(begin));
                Some((from, to))
            }
            None => Some((from, to)),
        };
    }
    if let Some((begin, finish)) = current {
        used = used.saturating_add(finish.saturating_sub(begin));
    }
    u16::try_from(
        end.saturating_sub(start)
            .saturating_sub(used)
            .div_euclid(60_000),
    )
    .unwrap_or(u16::MAX)
}

pub fn apply(view: &View, snapshot: &mut AppSnapshot) {
    snapshot.briefing = view.clone();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DaemonConnection, topics::TopicCache};
    use orbit_core::{
        reads::ReadRequest,
        topics::{Knowledge, TopicData, TopicFrame, TopicId, agenda::Value},
    };

    fn local_snapshot(now_ms: i64) -> crate::AppSnapshot {
        let mut snapshot = crate::AppSnapshot::empty(1, DaemonConnection::Ready);
        snapshot.briefing_clock_ms = now_ms;
        snapshot.system_timezone = "UTC".into();
        snapshot
    }

    fn paired_status(
        topics: &mut TopicCache,
        now_ms: i64,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let value = serde_json::from_value(serde_json::json!({
            "hub": "online", "account": { "device_id_short": "test" },
            "privacy_stopped": false, "helper_connected": true
        }))?;
        topics.observe_at(
            TopicFrame {
                epoch: "briefing-status".into(),
                revision: 1,
                data: TopicData::Status(Knowledge::Present {
                    value,
                    as_of_ms: None,
                }),
            },
            Some(now_ms),
        );
        Ok(())
    }

    fn agenda_fixture(
        now: i64,
        start: i64,
        end: i64,
        page: bool,
    ) -> Result<(crate::AppSnapshot, TopicCache), Box<dyn std::error::Error>> {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../contracts/fixtures/calendar-local-read-v1.json"
        ))?;
        let native: orbit_core::calendar::CalendarSnapshot =
            serde_json::from_value(fixture["snapshot"].clone())?;
        let mut snapshot = local_snapshot(now);
        snapshot.calendar.authorization = crate::CalendarAuthorizationSnapshot::Granted;
        snapshot.calendar.reading.state = crate::CalendarReadingState::Available;
        snapshot.calendar.reading.blocker = None;
        let mut topics = TopicCache::default();
        paired_status(&mut topics, now)?;
        topics.observe_at(
            TopicFrame {
                epoch: "briefing-second".into(),
                revision: 1,
                data: TopicData::Agenda(Knowledge::Present {
                    value: Value {
                        invalidation: 0,
                        calendars: Some(1),
                        read_window: Some(orbit_core::calendar::CalendarWindow {
                            start_unix_ms: start,
                            end_unix_ms: end,
                        }),
                        read_id: Some(native.snapshot_id),
                        timezone: Some("UTC".into()),
                        writes: Vec::new(),
                    },
                    as_of_ms: None,
                }),
            },
            Some(now),
        );
        if page {
            let items: Vec<serde_json::Value> = Vec::new();
            let total = items.len() as u64;
            assert!(topics.remember_read(
                &ReadRequest::AgendaRange {
                    start_ms: start,
                    end_ms: end,
                    cursor: None,
                },
                TopicId::Agenda,
                Knowledge::Present {
                    value: orbit_core::topics::Page::<serde_json::Value> {
                        items,
                        next: None,
                        total,
                        revision: 1,
                        store_version: None,
                    },
                    as_of_ms: None,
                },
            ));
        }
        Ok((snapshot, topics))
    }

    #[test]
    fn other_week_is_unknown_with_window_dates() -> Result<(), Box<dyn std::error::Error>> {
        let now = 1_790_593_200_000_i64;
        let start = now + 7 * 86_400_000;
        let end = start + 7 * 86_400_000;
        let (snapshot, topics) = agenda_fixture(now, start, end, true)?;
        let view = project(&FeatureInput::new(&snapshot, &topics)).ok_or("briefing view")?;
        assert!(
            matches!(view.sources.as_ref().map(|sources| &sources.agenda),
            Some(BriefingSourceState::Unknown { cause })
                if cause.contains(&start.to_string()) && cause.contains(&end.to_string()))
        );
        assert!(view.next.is_none());
        assert_eq!(view.summary.meetings_today, 0);
        Ok(())
    }

    #[test]
    fn present_head_does_not_make_unread_page_present() -> Result<(), Box<dyn std::error::Error>> {
        let now = 1_790_593_200_000_i64;
        let today_start = now.div_euclid(86_400_000) * 86_400_000;
        let today_end = today_start + 86_400_000;
        let start = today_start;
        let end = today_end;
        for (blocker, detail) in [
            (None, None),
            (
                Some(crate::CalendarReadingBlocker::NotPaired),
                Some("agenda_request_failed:offline"),
            ),
        ] {
            let (mut snapshot, topics) = agenda_fixture(now, start, end, false)?;
            let read_window = match topics.topic(TopicId::Agenda).map(|head| &head.knowledge) {
                Some(TopicData::Agenda(Knowledge::Present { value, .. })) => value
                    .read_window
                    .as_ref()
                    .ok_or("fixture read window missing")?,
                _ => return Err("fixture agenda head missing".into()),
            };
            assert!(
                read_window.start_unix_ms <= today_start && read_window.end_unix_ms >= today_end,
                "fixture's published agenda window must cover today"
            );
            if detail.is_some() {
                snapshot.calendar.reading.state = crate::CalendarReadingState::Unavailable;
            }
            snapshot.calendar.reading.detail = detail.map(str::to_owned);
            snapshot.calendar.reading.blocker = blocker.clone();
            let view = project(&FeatureInput::new(&snapshot, &topics)).ok_or("briefing view")?;
            let expected = match detail {
                Some(cause) => BriefingSourceState::Absent {
                    cause: cause.into(),
                },
                None => BriefingSourceState::Unknown {
                    cause: "agenda_page_pending".into(),
                },
            };
            assert_eq!(
                view.sources.as_ref().map(|sources| &sources.agenda),
                Some(&expected),
                "blocker: {blocker:?}"
            );
        }
        Ok(())
    }

    #[test]
    fn missing_zone_is_visible_instead_of_utc() -> Result<(), Box<dyn std::error::Error>> {
        let now = 1_790_593_200_000_i64;
        let start = now.div_euclid(86_400_000) * 86_400_000;
        let (mut snapshot, topics) = agenda_fixture(now, start, start + 86_400_000, true)?;
        snapshot.notes = None;
        let view = project(&FeatureInput::with_clock(
            &snapshot,
            &topics,
            crate::calendar::LocalClock {
                now_ms: now,
                zone: crate::LocalZone::utc(),
                timezone_name: Err("timezone_os_failed".into()),
            },
        ))
        .ok_or("briefing view")?;
        assert!(
            matches!(view.sources.as_ref().map(|sources| &sources.agenda),
            Some(BriefingSourceState::Unknown { cause }) if cause == "local_zone_unavailable")
        );
        Ok(())
    }

    #[test]
    fn missing_zone_does_not_hide_calendar_refusal() -> Result<(), Box<dyn std::error::Error>> {
        let mut snapshot = crate::AppSnapshot::empty(1, crate::DaemonConnection::Ready);
        snapshot.notes = None;
        let mut topics = TopicCache::default();
        topics.observe_at(
            TopicFrame {
                epoch: "briefing-second".into(),
                revision: 1,
                data: TopicData::Agenda(Knowledge::Absent {
                    refusal: orbit_core::refusal::Refusal::PermissionDenied {
                        permission: orbit_core::refusal::Permission::Calendar,
                    },
                }),
            },
            Some(1),
        );
        let view = project(&FeatureInput::new(&snapshot, &topics)).ok_or("briefing view")?;
        assert!(
            matches!(view.sources.as_ref().map(|sources| &sources.agenda),
            Some(BriefingSourceState::Absent { cause }) if serde_json::from_str::<serde_json::Value>(cause).is_ok_and(|v| v["code"] == "permission_denied" && v["permission"] == "calendar"))
        );
        Ok(())
    }

    #[test]
    fn briefing_does_not_reuse_legacy_facts_when_agenda_is_unknown()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut snapshot = crate::AppSnapshot::empty(1, crate::DaemonConnection::Ready);
        snapshot.briefing = project(&FeatureInput::new(&snapshot, &TopicCache::default()));
        let legacy = snapshot.briefing.as_mut().ok_or("legacy fixture")?;
        legacy.summary.meetings_today = 99;
        legacy.summary.focus_minutes = 99;
        let mut topics = TopicCache::default();
        topics.observe_at(
            TopicFrame {
                epoch: "briefing".into(),
                revision: 1,
                data: TopicData::not_observed(TopicId::Agenda),
            },
            Some(1),
        );
        let view = project(&FeatureInput::new(&snapshot, &topics)).ok_or("briefing view")?;
        assert_eq!(view.summary.meetings_today, 0);
        assert_eq!(view.summary.focus_minutes, 0);
        assert!(view.agenda.is_empty());
        Ok(())
    }

    #[test]
    fn next_meeting_comes_from_paged_agenda_four_hours_ahead()
    -> Result<(), Box<dyn std::error::Error>> {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../contracts/fixtures/calendar-local-read-v1.json"
        ))?;
        let native: orbit_core::calendar::CalendarSnapshot =
            serde_json::from_value(fixture["snapshot"].clone())?;
        let mut event = native.events.first().ok_or("fixture event")?.clone();
        let now = 1_790_593_200_000_i64; // 2026-09-28 11:00 UTC
        event.start_unix_ms = now + 4 * 60 * 60_000;
        event.end_unix_ms = event.start_unix_ms + 30 * 60_000;
        let mut past_event = event.clone();
        past_event.calendar_item_id = "past-morning".into();
        past_event.start_unix_ms = now - 2 * 60 * 60_000;
        past_event.end_unix_ms = now - 60 * 60_000;
        let start = now.div_euclid(86_400_000) * 86_400_000;
        let end = start + 86_400_000;
        let mut snapshot = local_snapshot(now);
        snapshot.calendar.authorization = crate::CalendarAuthorizationSnapshot::Granted;
        snapshot.calendar.reading.state = crate::CalendarReadingState::Available;
        snapshot.calendar.reading.blocker = None;
        let mut topics = TopicCache::default();
        paired_status(&mut topics, now)?;
        topics.observe_at(
            TopicFrame {
                epoch: "briefing".into(),
                revision: 1,
                data: TopicData::Agenda(Knowledge::Present {
                    value: Value {
                        invalidation: 0,
                        calendars: Some(1),
                        read_window: Some(orbit_core::calendar::CalendarWindow {
                            start_unix_ms: start,
                            end_unix_ms: end,
                        }),
                        read_id: Some(native.snapshot_id),
                        timezone: Some("UTC".into()),
                        writes: Vec::new(),
                    },
                    as_of_ms: None,
                }),
            },
            Some(now),
        );
        assert!(topics.remember_read(
            &ReadRequest::AgendaRange {
                start_ms: start,
                end_ms: end,
                cursor: None
            },
            TopicId::Agenda,
            Knowledge::Present {
                value: orbit_core::topics::Page {
                    items: vec![
                        serde_json::to_value(&past_event)?,
                        serde_json::to_value(&event)?
                    ],
                    next: None,
                    total: 2,
                    revision: 1,
                    store_version: None,
                },
                as_of_ms: None,
            },
        ));
        let view = project(&FeatureInput::new(&snapshot, &topics)).ok_or("briefing view")?;
        assert_eq!(view.summary.meetings_today, 2);
        assert_eq!(
            view.next.as_ref().map(|next| next.start_ms),
            Some(now + 4 * 60 * 60_000)
        );
        assert_eq!(view.agenda.len(), 2);
        assert_eq!(view.upcoming.as_ref().map(Vec::len), Some(2));
        assert!(matches!(
            view.sources.as_ref().map(|sources| &sources.notes),
            Some(BriefingSourceState::Unknown { .. })
        ));

        let previous = crate::NoteListItem {
            recording_id: "earlier-meeting".into(),
            title: event.title.clone(),
            started_at_ms: event.start_unix_ms - 86_400_000,
            duration_ms: 60_000,
            state: crate::NoteListState::Ready,
            group: crate::NoteDayGroup::Yesterday,
            headline: None,
        };
        topics.observe_at(
            TopicFrame {
                epoch: "briefing".into(),
                revision: 1,
                data: TopicData::Notes(Knowledge::Present {
                    value: orbit_core::topics::notes::Value::default(),
                    as_of_ms: None,
                }),
            },
            Some(now),
        );
        if let Some(notes) = snapshot.notes.as_mut() {
            notes.list = vec![previous];
            notes.index = Some(Box::new(crate::NotesIndexSnapshot {
                state: crate::NotesIndexState::Available,
                more: false,
                detail: None,
            }));
        }
        let with_previous =
            project(&FeatureInput::new(&snapshot, &topics)).ok_or("briefing with prior note")?;
        assert_eq!(
            with_previous
                .next
                .as_ref()
                .and_then(|next| next.previous.as_ref())
                .map(|note| note.recording_id.as_str()),
            Some("earlier-meeting")
        );
        let previous = serde_json::to_value(
            with_previous
                .next
                .as_ref()
                .and_then(|next| next.previous.as_ref())
                .ok_or("previous note")?,
        )?;
        assert_eq!(previous["title"], event.title);
        assert_eq!(previous["summary"], serde_json::Value::Null);

        let mut later = local_snapshot(now + 60_000);
        later.calendar.authorization = crate::CalendarAuthorizationSnapshot::Granted;
        later.calendar.reading.state = crate::CalendarReadingState::Available;
        later.calendar.reading.blocker = None;
        // The notes list lives in the local snapshot; a quiet minute
        // keeps it.
        later.notes.clone_from(&snapshot.notes);
        let frames = crate::features::project_changed(
            &FeatureInput::new(&snapshot, &topics),
            &FeatureInput::new(&later, &topics),
            || crate::features::FrameRevision {
                epoch: 1,
                counter: 1,
            },
        );
        assert!(
            !frames
                .iter()
                .any(|frame| frame.feature() == crate::features::FeatureId::Briefing),
            "a quiet minute must emit zero Briefing revisions"
        );

        let refusal = orbit_core::refusal::Refusal::TooLarge {
            resource: orbit_core::refusal::Resource::IpcResponse,
            bytes: 70_000,
            max_bytes: 65_536,
        };
        assert!(topics.remember_read(
            &ReadRequest::AgendaRange {
                start_ms: start,
                end_ms: end,
                cursor: None
            },
            TopicId::Agenda,
            Knowledge::Absent {
                refusal: refusal.clone()
            },
        ));
        let refused =
            project(&FeatureInput::new(&snapshot, &topics)).ok_or("briefing with refused page")?;
        assert!(refused.agenda.is_empty());
        assert!(refused.next.is_none());
        assert!(
            matches!(refused.sources.as_ref().map(|sources| &sources.agenda),
            Some(BriefingSourceState::Absent { cause }) if cause.contains("70000") && cause.contains("65536"))
        );
        Ok(())
    }

    /// Hoy carries the observation id; app-core resolves all the data the
    /// daemon needs to link it, including a detached occurrence and its guests.
    #[test]
    fn briefing_and_calendar_record_the_same_occurrence() -> Result<(), Box<dyn std::error::Error>>
    {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../contracts/fixtures/calendar-local-read-v1.json"
        ))?;
        let native: orbit_core::calendar::CalendarSnapshot =
            serde_json::from_value(fixture["snapshot"].clone())?;
        let now = 1_790_593_200_000_i64;
        let start = now.div_euclid(86_400_000) * 86_400_000;
        let end = start + 86_400_000;
        let mut weekly = serde_json::to_value(native.events.first().ok_or("fixture event")?)?;
        weekly["start_unix_ms"] = serde_json::json!(now + 60 * 60_000);
        weekly["end_unix_ms"] = serde_json::json!(now + 90 * 60_000);
        weekly["recurrence"] = serde_json::json!({"frequency": "weekly", "interval": 1});
        weekly["invitees"][1]["is_current_user"] = serde_json::json!(true);
        let mut single = weekly.clone();
        single["observation_id"] = serde_json::json!("44444444-4444-4444-8444-444444444444");
        single["calendar_item_id"] = serde_json::json!("single-item");
        single["start_unix_ms"] = serde_json::json!(now + 2 * 60 * 60_000);
        single["end_unix_ms"] = serde_json::json!(now + 3 * 60 * 60_000);
        single
            .as_object_mut()
            .ok_or("event object")?
            .remove("recurrence");
        let (snapshot, mut topics) = agenda_fixture(now, start, end, false)?;
        assert!(topics.remember_read(
            &ReadRequest::AgendaRange {
                start_ms: start,
                end_ms: end,
                cursor: None,
            },
            TopicId::Agenda,
            Knowledge::Present {
                value: orbit_core::topics::Page {
                    items: vec![weekly, single],
                    next: None,
                    total: 2,
                    revision: 1,
                    store_version: None,
                },
                as_of_ms: None,
            },
        ));
        let input = FeatureInput::new(&snapshot, &topics);
        let (calendar, _) = super::super::agenda::project_with_read(&input);
        let view = project(&input).ok_or("briefing view")?;
        let upcoming = view.upcoming.as_ref().ok_or("upcoming meetings")?;
        assert_eq!(upcoming.len(), 2);
        let mut links = Vec::new();
        for meeting in upcoming {
            let event = calendar
                .events
                .iter()
                .find(|event| event.id == meeting.event_id)
                .ok_or("the calendar's event")?;
            assert!(event.calendar_item_id.is_some());
            let command = super::super::recording::RecordingCommand::MeetingStartSelected {
                title: String::new(),
                minutes: None,
                audio: None,
                event: meeting.event.clone(),
                event_id: Some(meeting.event_id.clone()),
            };
            let orbit_core::protocol::Command::MeetingStart {
                event: Some(link), ..
            } = super::super::recording::map_command(command, &calendar, &input.clock)?
            else {
                return Err("the event was not linked".into());
            };
            assert_eq!(
                Some(&link.calendar_item_id),
                event.calendar_item_id.as_ref()
            );
            assert_eq!(Some(link.occurrence_start_ms), event.occurrence_start_ms);
            assert_eq!(link.recurring, Some(event.recurrence.is_some()));
            let guests = event.attendees.as_deref().unwrap_or_default();
            assert_eq!(meeting.attendees.len(), guests.len());
            assert_eq!(link.attendees.len(), guests.len());
            for ((shown, linked), guest) in
                meeting.attendees.iter().zip(&link.attendees).zip(guests)
            {
                assert_eq!(shown.name, guest.name);
                assert_eq!(linked.name, guest.name);
                assert_eq!(linked.email, guest.email.clone().unwrap_or_default());
                assert_eq!(linked.is_self, guest.is_self);
            }
            links.push(link);
        }
        assert_eq!(links[0].recurring, Some(true), "the weekly one repeats");
        assert_eq!(links[1].recurring, Some(false), "the single one does not");
        assert!(
            links[0].attendees.iter().any(|guest| guest.is_self),
            "Alina stays marked"
        );
        // The detached occurrence keeps the date that names it.
        assert_eq!(
            links[0].occurrence_start_ms, 1_699_160_400_000,
            "the original date of the occurrence, not its start"
        );
        assert_eq!(view.next.as_ref(), upcoming.first());
        Ok(())
    }

    #[test]
    fn full_agenda_page_debug_measurement() -> Result<(), Box<dyn std::error::Error>> {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../contracts/fixtures/calendar-local-read-v1.json"
        ))?;
        let native: orbit_core::calendar::CalendarSnapshot =
            serde_json::from_value(fixture["snapshot"].clone())?;
        let mut event = native.events.first().ok_or("fixture event")?.clone();
        let now = 1_790_593_200_000_i64;
        event.start_unix_ms = now + 60_000;
        event.end_unix_ms = now + 120_000;
        let start = now.div_euclid(86_400_000) * 86_400_000;
        let end = start + 86_400_000;
        let request = ReadRequest::AgendaRange {
            start_ms: start,
            end_ms: end,
            cursor: None,
        };
        let ceiling = request.id().max_bytes();
        let mut items = Vec::new();
        let mut estimated = serde_json::to_vec(&Knowledge::Present {
            value: orbit_core::topics::Page::<serde_json::Value> {
                items: Vec::new(),
                next: None,
                total: 0,
                revision: 1,
                store_version: None,
            },
            as_of_ms: None,
        })?
        .len();
        for index in 0..500 {
            event.observation_id = format!("measurement-{index}");
            let item = serde_json::to_value(&event)?;
            let item_bytes = serde_json::to_vec(&item)?.len();
            let candidate = estimated
                + usize::from(!items.is_empty())
                + item_bytes
                + (items.len() + 1).to_string().len()
                - items.len().to_string().len();
            if candidate > ceiling {
                break;
            }
            estimated = candidate;
            items.push(item);
        }
        let answer = Knowledge::Present {
            value: orbit_core::topics::Page {
                total: items.len() as u64,
                items,
                next: None,
                revision: 1,
                store_version: None,
            },
            as_of_ms: None,
        };
        let actual = serde_json::to_vec(&answer)?.len();
        assert!(
            actual >= 120 * 1024 && actual <= ceiling,
            "page {actual} of {ceiling} bytes"
        );
        let mut topics = TopicCache::default();
        topics.observe_at(
            TopicFrame {
                epoch: "measurement".into(),
                revision: 1,
                data: TopicData::Agenda(Knowledge::Present {
                    value: Value {
                        invalidation: 0,
                        calendars: Some(1),
                        read_window: Some(orbit_core::calendar::CalendarWindow {
                            start_unix_ms: start,
                            end_unix_ms: end,
                        }),
                        read_id: Some(native.snapshot_id),
                        timezone: Some("UTC".into()),
                        writes: Vec::new(),
                    },
                    as_of_ms: None,
                }),
            },
            Some(now),
        );
        assert!(topics.remember_read(&request, TopicId::Agenda, answer));
        let linked_notes = (0..100)
            .map(|index| crate::NoteListItem {
                recording_id: format!("linked-{index}"),
                title: event.title.clone(),
                started_at_ms: now - 86_400_000 + index * 60_000,
                duration_ms: 60_000,
                state: crate::NoteListState::Ready,
                group: crate::NoteDayGroup::Yesterday,
                headline: None,
            })
            .collect::<Vec<_>>();
        topics.observe_at(
            TopicFrame {
                epoch: "measurement".into(),
                revision: 1,
                data: TopicData::Notes(Knowledge::Present {
                    value: orbit_core::topics::notes::Value::default(),
                    as_of_ms: None,
                }),
            },
            Some(now),
        );
        let mut snapshot = local_snapshot(now);
        snapshot.calendar.authorization = crate::CalendarAuthorizationSnapshot::Granted;
        snapshot.calendar.reading.state = crate::CalendarReadingState::Available;
        snapshot.calendar.reading.blocker = None;
        snapshot.settings.hub.paired = true;
        if let Some(notes) = snapshot.notes.as_mut() {
            notes.list = linked_notes;
            notes.index = Some(Box::new(crate::NotesIndexSnapshot {
                state: crate::NotesIndexState::Available,
                more: false,
                detail: None,
            }));
        }
        let input = FeatureInput::new(&snapshot, &topics);
        let mut micros = Vec::new();
        for _ in 0..30 {
            let start = std::time::Instant::now();
            let view = project(&input).ok_or("briefing view")?;
            assert_eq!(view.agenda.len(), answer_len(&topics, &request));
            micros.push(start.elapsed().as_micros());
        }
        micros.sort_unstable();
        eprintln!(
            "BRIEFING_PAGE_DEBUG repetitions=30 bytes={actual} ceiling={ceiling} p99_us={} worst_us={}",
            micros[29], micros[29]
        );
        Ok(())
    }

    fn answer_len(topics: &TopicCache, request: &ReadRequest) -> usize {
        match topics.read(request, TopicId::Agenda) {
            Some(Knowledge::Present { value, .. }) => value.items.len(),
            Some(Knowledge::Unknown { .. } | Knowledge::Absent { .. }) | None => 0,
        }
    }
    #[test]
    fn hoy_and_calendar_share_recording_phases_and_unknown_stays_unknown()
    -> Result<(), Box<dyn std::error::Error>> {
        let now = 1_790_593_200_000_i64;
        let start = now.div_euclid(86_400_000) * 86_400_000;
        let end = start + 86_400_000;
        let (snapshot, mut topics) = agenda_fixture(now, start, end, false)?;
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../contracts/fixtures/calendar-local-read-v1.json"
        ))?;
        let native: orbit_core::calendar::CalendarSnapshot =
            serde_json::from_value(fixture["snapshot"].clone())?;
        let mut event = native.events.first().ok_or("fixture event")?.clone();
        event.title = "Comité".into();
        event.start_unix_ms = now - 20 * 60_000;
        event.end_unix_ms = now + 40 * 60_000;
        assert!(topics.remember_read(
            &ReadRequest::AgendaRange {
                start_ms: start,
                end_ms: end,
                cursor: None
            },
            TopicId::Agenda,
            Knowledge::Present {
                value: orbit_core::topics::Page {
                    items: vec![serde_json::to_value(&event)?],
                    next: None,
                    total: 1,
                    revision: 1,
                    store_version: None,
                },
                as_of_ms: None,
            },
        ));
        let unknown = super::super::agenda::project(&FeatureInput::new(&snapshot, &topics));
        assert!(serde_json::to_value(&unknown.events[0])?["recording"].is_null());
        for (index, phase) in [
            orbit_core::CapturePhase::Starting,
            orbit_core::CapturePhase::Recording,
            orbit_core::CapturePhase::Paused,
            orbit_core::CapturePhase::Finalizing,
            orbit_core::CapturePhase::Interrupted,
            orbit_core::CapturePhase::Idle,
        ]
        .into_iter()
        .enumerate()
        {
            let mut current = crate::projection::test_support::recording(phase.clone());
            // The topic has no event link: the existing Rust rule matches series,
            // including numeric dates, rather than exact titles.
            current.title = "Comité 2026-10-19".into();
            topics.observe_at(
                TopicFrame {
                    epoch: "briefing-second".into(),
                    revision: index as u64 + 1,
                    data: TopicData::Recording(Knowledge::Present {
                        value: orbit_core::topics::recording::Value {
                            recording: Some(Box::new(current)),
                            recording_signal: None,
                            pending_jobs: 0,
                            offline: false,
                            call: None,
                            question: None,
                            voice_trigger: None,
                            applications: Knowledge::Unknown {
                                why: orbit_core::topics::Pending::NotObservedYet,
                            },
                        },
                        as_of_ms: None,
                    }),
                },
                Some(now),
            );
            let input = FeatureInput::new(&snapshot, &topics);
            let calendar = super::super::agenda::project(&input);
            let today = project(&input).ok_or("briefing")?;
            let expected = if matches!(
                phase,
                orbit_core::CapturePhase::Interrupted | orbit_core::CapturePhase::Idle
            ) {
                "none"
            } else {
                "recording"
            };
            assert_eq!(
                serde_json::to_value(&calendar.events[0])?["recording"],
                expected,
                "{phase:?}"
            );
            assert_eq!(
                serde_json::to_value(today.next.ok_or("next meeting")?)?["recording"],
                expected,
                "{phase:?}"
            );
        }
        let agenda = topics.topic(TopicId::Agenda).ok_or("agenda topic")?.clone();
        let request = ReadRequest::AgendaRange {
            start_ms: start,
            end_ms: end,
            cursor: None,
        };
        let page = topics
            .read(&request, TopicId::Agenda)
            .ok_or("agenda page")?
            .clone();
        topics.ended("fixture disconnected".into());
        // The agenda recovers before recording: the event stays visible, but
        // a recording topic that is still stale cannot answer yes or no.
        topics.observe_at(
            TopicFrame {
                epoch: "briefing-second".into(),
                revision: agenda.revision,
                data: agenda.knowledge,
            },
            Some(now),
        );
        assert!(topics.remember_read(&request, TopicId::Agenda, page));
        let calendar = super::super::agenda::project(&FeatureInput::new(&snapshot, &topics));
        assert!(serde_json::to_value(&calendar.events[0])?["recording"].is_null());
        Ok(())
    }
}
