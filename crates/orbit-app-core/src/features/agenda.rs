use super::FeatureInput;
use crate::AppSnapshot;
use orbit_core::{
    Connection,
    calendar::CalendarOccurrence,
    reads::ReadRequest,
    topics::{Cursor, Knowledge, TopicData, TopicId, agenda::Value},
};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

pub type View = crate::CalendarSnapshot;

/// The calendar view: the events of the period on screen only. The read
/// behind it also covers today for the Briefing, which projects the
/// whole read through [`project_with_read`].
pub fn project(input: &FeatureInput<'_>) -> View {
    let mut view = project_with_read(input).0;
    let range = view.range.clone();
    if range.end_ms > range.start_ms {
        view.events
            .retain(|event| event.start_ms < range.end_ms && event.end_ms > range.start_ms);
        // The range moves at once and its read arrives later: until a read
        // covers it, what is on screen is pending, not the new period's
        // events.
        if view.window.start_ms > range.start_ms || view.window.end_ms < range.end_ms {
            view.stale = true;
        }
    }
    view
}

pub(crate) fn project_with_read(input: &FeatureInput<'_>) -> (View, bool) {
    let mut view = input.local.calendar.clone();
    // A count in the head does not contain calendar identities or colors.
    // In particular, an old snapshot cannot fill a missing keyed listing.
    view.calendars.clear();
    // Write outcomes come from the head alone, never an old frame.
    view.writes.clear();
    let mut page_loaded = false;
    let Some(topic) = input.topics.topic(TopicId::Agenda) else {
        return (view, false);
    };
    if let TopicData::Agenda(knowledge) = &topic.knowledge {
        match knowledge {
            Knowledge::Unknown { why } => {
                if view.authorization != crate::CalendarAuthorizationSnapshot::NotDetermined {
                    view.authorization = crate::CalendarAuthorizationSnapshot::Unknown;
                }
                view.reading.state = crate::CalendarReadingState::Unknown;
                view.reading.blocker = None;
                view.reading.detail = Some(
                    if view.authorization == crate::CalendarAuthorizationSnapshot::NotDetermined {
                        "agenda_not_determined".into()
                    } else {
                        match why {
                            orbit_core::topics::Pending::NotObservedYet => {
                                "agenda_not_observed_yet".into()
                            }
                            orbit_core::topics::Pending::HelperStarting => {
                                "agenda_helper_starting".into()
                            }
                            orbit_core::topics::Pending::Reading { since_ms } => {
                                format!("agenda_reading_since:{since_ms}")
                            }
                            orbit_core::topics::Pending::Transcription {
                                completed,
                                pending,
                                failed,
                                queued,
                            } => {
                                format!(
                                    "transcription_pending:{completed}:{pending}:{failed}:{queued}"
                                )
                            }
                            orbit_core::topics::Pending::DaemonStarting => {
                                "agenda_daemon_starting".into()
                            }
                        }
                    },
                );
                view.calendars.clear();
                view.events.clear();
            }
            Knowledge::Absent { refusal } => {
                view.events.clear();
                view.calendars.clear();
                view.reading.state = crate::CalendarReadingState::Unavailable;
                view.reading.detail = Some(refusal.to_string());
                if matches!(
                    refusal,
                    orbit_core::refusal::Refusal::PermissionDenied { .. }
                ) {
                    if view.authorization != crate::CalendarAuthorizationSnapshot::NotDetermined {
                        view.authorization = crate::CalendarAuthorizationSnapshot::Denied;
                    }
                    view.reading.blocker = Some(crate::CalendarReadingBlocker::CalendarPermission);
                } else if matches!(refusal, orbit_core::refusal::Refusal::PrivacyStopped { .. }) {
                    view.authorization = crate::CalendarAuthorizationSnapshot::Unknown;
                    view.reading.blocker = Some(crate::CalendarReadingBlocker::PrivacyStopped);
                } else if matches!(refusal, orbit_core::refusal::Refusal::HelperNotRunning) {
                    view.authorization = crate::CalendarAuthorizationSnapshot::Unknown;
                    view.reading.blocker = Some(crate::CalendarReadingBlocker::HelperNotRunning);
                } else {
                    view.authorization = crate::CalendarAuthorizationSnapshot::Unknown;
                    view.reading.blocker = None;
                }
            }
            Knowledge::Present { value, .. } => {
                view.writes = value.writes.clone();
                let request_failed = view.reading.state == crate::CalendarReadingState::Unavailable
                    && view.reading.detail.as_deref().is_some_and(|detail| {
                        detail.starts_with(crate::calendar::AGENDA_REQUEST_FAILED_PREFIX)
                    });
                view.authorization = crate::CalendarAuthorizationSnapshot::Granted;
                if !request_failed
                    && !matches!(
                        view.reading.blocker,
                        Some(
                            crate::CalendarReadingBlocker::NotPaired
                                | crate::CalendarReadingBlocker::MacDisconnected
                        )
                    )
                {
                    view.reading.state = crate::CalendarReadingState::Available;
                    view.reading.blocker = None;
                    view.reading.detail = None;
                }
                if let Some(window) = value.read_window {
                    view.window.start_ms = window.start_unix_ms;
                    view.window.end_ms = window.end_unix_ms;
                }
                if let Some(timezone) = &value.timezone {
                    view.window.timezone.clone_from(timezone);
                }
                let listing = cached_calendars(input);
                if let Ok(Some(calendars)) = &listing
                    && value.calendars == u32::try_from(calendars.len()).ok()
                {
                    view.calendars.clone_from(calendars);
                }
                match cached_events(input, value) {
                    Ok(Some(events)) => {
                        page_loaded = true;
                        view.events = events;
                        for event in &mut view.events {
                            event.recording = super::recording::event_recording(event, input);
                        }
                        view.stale = false;
                        if request_failed {
                            view.reading.state = crate::CalendarReadingState::Available;
                            view.reading.detail = None;
                        }
                    }
                    Ok(None) => {
                        view.events.clear();
                        view.stale = !request_failed;
                    }
                    Err(refusal) => {
                        view.events.clear();
                        view.stale = false;
                        view.reading.state = crate::CalendarReadingState::Unavailable;
                        view.reading.blocker = None;
                        view.reading.detail = Some(refusal.to_string());
                    }
                }
                if let Err(cause) = listing {
                    view.reading.state = crate::CalendarReadingState::Unavailable;
                    view.reading.detail = Some(match view.reading.detail.take() {
                        Some(other) => format!("{other}; {cause}"),
                        None => cause,
                    });
                }
                if view.reading.state == crate::CalendarReadingState::Available {
                    match input
                        .topics
                        .topic(TopicId::Status)
                        .map(|topic| &topic.knowledge)
                    {
                        Some(TopicData::Status(Knowledge::Present { value, .. })) => {
                            if value.privacy_stopped {
                                view.reading.state = crate::CalendarReadingState::Unavailable;
                                view.reading.blocker =
                                    Some(crate::CalendarReadingBlocker::PrivacyStopped);
                            } else if value.hub == Connection::Revoked
                                || value
                                    .account
                                    .as_ref()
                                    .and_then(|account| account.device_id_short.as_ref())
                                    .is_none()
                            {
                                view.reading.state = crate::CalendarReadingState::Unavailable;
                                view.reading.blocker =
                                    Some(crate::CalendarReadingBlocker::NotPaired);
                            } else if value.hub != Connection::Online {
                                view.reading.state = crate::CalendarReadingState::Unavailable;
                                view.reading.blocker =
                                    Some(crate::CalendarReadingBlocker::MacDisconnected);
                            }
                        }
                        Some(TopicData::Status(Knowledge::Absent { refusal })) => {
                            view.reading.state = crate::CalendarReadingState::Unknown;
                            view.reading.detail = Some(format!("status: {refusal}"));
                        }
                        Some(TopicData::Status(Knowledge::Unknown { why })) => {
                            view.reading.state = crate::CalendarReadingState::Unknown;
                            view.reading.detail = Some(match why {
                                orbit_core::topics::Pending::NotObservedYet => {
                                    "status_not_observed_yet".into()
                                }
                                orbit_core::topics::Pending::HelperStarting => {
                                    "status_helper_starting".into()
                                }
                                orbit_core::topics::Pending::Reading { since_ms } => {
                                    format!("status_reading_since:{since_ms}")
                                }
                                orbit_core::topics::Pending::Transcription {
                                    completed,
                                    pending,
                                    failed,
                                    queued,
                                } => format!(
                                    "status_transcription_pending:{completed}:{pending}:{failed}:{queued}"
                                ),
                                orbit_core::topics::Pending::DaemonStarting => {
                                    "status_daemon_starting".into()
                                }
                            });
                        }
                        None => {
                            view.reading.state = crate::CalendarReadingState::Unknown;
                            view.reading.detail = Some("status_not_observed_yet".into());
                        }
                        Some(
                            TopicData::Agenda(_)
                            | TopicData::Permissions(_)
                            | TopicData::Recording(_)
                            | TopicData::Notes(_)
                            | TopicData::Assistant(_)
                            | TopicData::Control(_)
                            | TopicData::Notices(_),
                        ) => {}
                    }
                }
            }
        }
    }
    (view, page_loaded)
}

fn cached_calendars(
    input: &FeatureInput<'_>,
) -> Result<Option<Vec<crate::CalendarItemSnapshot>>, String> {
    let mut cursor = None;
    let mut visited = Vec::new();
    let mut calendars = Vec::new();
    loop {
        let request = ReadRequest::AgendaCalendars {
            cursor: cursor.clone(),
        };
        let page = match input.topics.read(&request, TopicId::Agenda) {
            Some(Knowledge::Present { value, .. }) => value,
            Some(Knowledge::Absent { refusal }) => {
                return Err(format!("AgendaCalendars: {refusal}"));
            }
            Some(Knowledge::Unknown { .. }) | None => return Ok(None),
        };
        for (index, raw) in page.items.iter().enumerate() {
            let entry: orbit_core::calendar::CalendarEntry = serde_json::from_value(raw.clone())
                .map_err(|error| {
                    format!("AgendaCalendars: calendario {index} ilegible: {error}")
                })?;
            calendars.push(crate::CalendarItemSnapshot {
                // The calendar's real color; the palette token only
                // when macOS gave none.
                color: entry.color.clone().unwrap_or_else(|| {
                    crate::projection::stable_calendar_color(&entry.reference.calendar_id)
                }),
                id: entry.reference.calendar_id,
                title: entry.title,
                source_id: entry.reference.source_id,
                source_title: Some(entry.source_title),
                can_edit: entry.can_edit,
                is_default: entry.is_default,
            });
        }
        match &page.next {
            Some(next @ Cursor::Id { .. }) if !visited.contains(next) => {
                visited.push(next.clone());
                cursor = Some(next.clone());
            }
            Some(_) => return Err("AgendaCalendars: cursor repetido o de otra lectura".into()),
            None => {
                if u64::try_from(calendars.len()).ok() != Some(page.total) {
                    return Err(format!(
                        "AgendaCalendars: {} calendarios leídos de {} anunciados",
                        calendars.len(),
                        page.total
                    ));
                }
                return Ok(Some(calendars));
            }
        }
    }
}

fn cached_events(
    input: &FeatureInput<'_>,
    head: &Value,
) -> Result<Option<Vec<crate::CalendarEventSnapshot>>, orbit_core::refusal::Refusal> {
    if head.read_id.is_none() {
        return Ok(None);
    }
    let Some(window) = head.read_window else {
        return Ok(None);
    };
    let mut cursor = None;
    let mut visited: Vec<Cursor> = Vec::new();
    let mut events = Vec::new();
    loop {
        let request = ReadRequest::AgendaRange {
            start_ms: window.start_unix_ms,
            end_ms: window.end_unix_ms,
            cursor: cursor.clone(),
        };
        let page = match input.topics.read(&request, TopicId::Agenda) {
            Some(Knowledge::Present { value, .. }) => value,
            Some(Knowledge::Absent { refusal }) => return Err(refusal.clone()),
            Some(Knowledge::Unknown { .. }) | None => return Ok(None),
        };
        for (index, raw) in page.items.iter().enumerate() {
            let event =
                serde_json::from_value::<CalendarOccurrence>(raw.clone()).map_err(|error| {
                    orbit_core::refusal::Refusal::DaemonReplyInvalid {
                        field: "calendar.events".into(),
                        issue: orbit_core::refusal::DaemonReplyIssue::Decode {
                            error: format!("calendar.events[{index}]: {error}"),
                        },
                    }
                })?;
            events.push(crate::projection::project_calendar_occurrence(&event));
        }
        match &page.next {
            Some(next) if !visited.contains(next) => {
                visited.push(next.clone());
                cursor = Some(next.clone());
            }
            Some(_) => {
                return Err(orbit_core::refusal::Refusal::DaemonReplyInvalid {
                    field: "calendar.events".into(),
                    issue: orbit_core::refusal::DaemonReplyIssue::CursorRepeated,
                });
            }
            None => {
                if u64::try_from(events.len()).ok() != Some(page.total) {
                    return Err(orbit_core::refusal::Refusal::DaemonReplyInvalid {
                        field: "calendar.events".into(),
                        issue: orbit_core::refusal::DaemonReplyIssue::CountMismatch {
                            delivered: events.len() as u64,
                            total: page.total,
                        },
                    });
                }
                return Ok(Some(events));
            }
        }
    }
}

pub fn apply(view: &View, snapshot: &mut AppSnapshot) {
    snapshot.calendar = view.clone();
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "command", rename_all = "snake_case")]
#[ts(export, export_to = "../../apps/desktop/src/bridge/generated/")]
pub enum AgendaCommand {
    /// Show this view around `anchor_ms`, any instant of the period.
    /// `days` counts only for the N-day grid (1–14). Replaces
    /// `calendar_go_to_week`.
    CalendarSetRange {
        view: crate::CalendarViewKind,
        #[ts(type = "number")]
        anchor_ms: i64,
        days: u8,
        week_start: crate::CalendarWeekStart,
    },
    /// The Briefing is on screen and the read does not hold today.
    /// Read again if the same range now would, or return to today in the
    /// same view when the range is beyond `TODAY_REACH_DAYS`.
    CalendarShowToday,
    CalendarRefresh,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::topics::TopicCache;
    use orbit_core::{
        calendar::CalendarAuthorization,
        refusal::{Permission, Refusal},
        topics::{TopicData, TopicFrame, TopicId},
    };

    /// The period moved before its read arrived. The events of
    /// the previous read are not shown as the new period's: pending until
    /// a read covers the range.
    #[test]
    fn a_range_the_read_does_not_cover_is_pending() {
        let read = orbit_core::calendar::CalendarWindow {
            start_unix_ms: 1_000_000_000_000,
            end_unix_ms: 1_000_604_800_000,
        };
        let mut topics = TopicCache::default();
        topics.observe_at(
            TopicFrame {
                epoch: "fixture".into(),
                revision: 1,
                data: TopicData::Agenda(Knowledge::Present {
                    value: Value {
                        invalidation: 0,
                        calendars: Some(0),
                        read_window: Some(read),
                        read_id: Some("read".into()),
                        timezone: Some("UTC".into()),
                        writes: Vec::new(),
                    },
                    as_of_ms: None,
                }),
            },
            Some(1),
        );
        assert!(topics.remember_read(
            &ReadRequest::AgendaRange {
                start_ms: read.start_unix_ms,
                end_ms: read.end_unix_ms,
                cursor: None,
            },
            TopicId::Agenda,
            Knowledge::Present {
                value: orbit_core::topics::Page {
                    items: Vec::new(),
                    next: None,
                    total: 0,
                    revision: 1,
                    store_version: None,
                },
                as_of_ms: None,
            },
        ));
        let mut snapshot = crate::AppSnapshot::empty(1, crate::DaemonConnection::Ready);
        snapshot.calendar.range.start_ms = read.start_unix_ms;
        snapshot.calendar.range.end_ms = read.end_unix_ms;
        assert!(!project(&FeatureInput::new(&snapshot, &topics)).stale);
        // The next week, asked for and not read yet.
        snapshot.calendar.range.start_ms = read.end_unix_ms;
        snapshot.calendar.range.end_ms = read.end_unix_ms + 604_800_000;
        assert!(project(&FeatureInput::new(&snapshot, &topics)).stale);
        // Half inside is still not covered.
        snapshot.calendar.range.start_ms = read.start_unix_ms + 86_400_000;
        snapshot.calendar.range.end_ms = read.end_unix_ms + 86_400_000;
        assert!(project(&FeatureInput::new(&snapshot, &topics)).stale);
    }

    /// Each write's outcome reaches the calendar view from the
    /// agenda head — saved with the occurrence it left, its outcome unknown
    /// with its code and notice — and goes away with the head when the
    /// agenda stops being readable.
    #[test]
    fn write_outcomes_reach_the_view_from_the_agenda_head() {
        use orbit_core::{
            calendar_control::{CalendarWriteOperation, CalendarWriteStage},
            topics::agenda::{WriteOutcome, WriteTarget},
        };
        let unknown = "No se sabe si el cambio llegó al calendario (write_deadline)";
        let mut topics = TopicCache::default();
        let head = Value {
            invalidation: 0,
            calendars: Some(0),
            read_window: None,
            read_id: None,
            timezone: Some("UTC".into()),
            writes: vec![
                WriteOutcome {
                    request_id: "create-1".into(),
                    operation: CalendarWriteOperation::Create,
                    stage: CalendarWriteStage::Saved,
                    code: None,
                    notice: None,
                    saved: Some(WriteTarget {
                        calendar_item_id: "item-new".into(),
                        occurrence_start_ms: 1_790_848_800_000,
                        start_ms: 1_790_848_800_000,
                        end_ms: 1_790_850_600_000,
                    }),
                },
                WriteOutcome {
                    request_id: "move-1".into(),
                    operation: CalendarWriteOperation::Update,
                    stage: CalendarWriteStage::OutcomeUnknown,
                    code: Some("calendar_write_outcome_unknown".into()),
                    notice: Some(unknown.into()),
                    saved: None,
                },
            ],
        };
        topics.observe_at(
            TopicFrame {
                epoch: "fixture".into(),
                revision: 1,
                data: TopicData::Agenda(Knowledge::Present {
                    value: head,
                    as_of_ms: None,
                }),
            },
            Some(1),
        );
        let snapshot = crate::AppSnapshot::empty(1, crate::DaemonConnection::Ready);
        let view = project(&FeatureInput::new(&snapshot, &topics));
        assert_eq!(
            view.writes,
            vec![
                crate::WriteOutcome {
                    request_id: "create-1".into(),
                    operation: crate::CalendarWriteOperation::Create,
                    stage: crate::CalendarWriteStage::Saved,
                    code: None,
                    notice: None,
                    saved: Some(crate::WriteTarget {
                        calendar_item_id: "item-new".into(),
                        occurrence_start_ms: 1_790_848_800_000,
                        start_ms: 1_790_848_800_000,
                        end_ms: 1_790_850_600_000,
                    }),
                },
                crate::WriteOutcome {
                    request_id: "move-1".into(),
                    operation: crate::CalendarWriteOperation::Update,
                    stage: crate::CalendarWriteStage::OutcomeUnknown,
                    code: Some("calendar_write_outcome_unknown".into()),
                    notice: Some(unknown.into()),
                    saved: None,
                },
            ]
        );

        topics.observe_at(
            TopicFrame {
                epoch: "fixture".into(),
                revision: 2,
                data: TopicData::Agenda(Knowledge::Absent {
                    refusal: Refusal::PrivacyStopped { since_ms: None },
                }),
            },
            Some(2),
        );
        let mut stopped = snapshot.clone();
        stopped.calendar.writes = view.writes;
        let view = project(&FeatureInput::new(&stopped, &topics));
        assert!(view.writes.is_empty(), "{:?}", view.writes);
    }

    #[test]
    fn an_unread_calendar_listing_does_not_reuse_old_colors() {
        let mut snapshot = crate::AppSnapshot::empty(1, crate::DaemonConnection::Ready);
        snapshot
            .calendar
            .calendars
            .push(crate::CalendarItemSnapshot {
                id: "old-calendar".into(),
                title: "Calendario antiguo".into(),
                source_id: "old-source".into(),
                source_title: Some("Cuenta antigua".into()),
                color: "blue".into(),
                can_edit: None,
                is_default: None,
            });
        let mut topics = TopicCache::default();
        topics.observe_at(
            TopicFrame {
                epoch: "fixture".into(),
                revision: 2,
                data: TopicData::Agenda(Knowledge::Present {
                    value: Value {
                        invalidation: 0,
                        calendars: Some(1),
                        read_window: None,
                        read_id: None,
                        timezone: Some("Europe/Madrid".into()),
                        writes: Vec::new(),
                    },
                    as_of_ms: None,
                }),
            },
            Some(1),
        );

        let view = project(&FeatureInput::new(&snapshot, &topics));
        assert!(
            view.calendars.is_empty(),
            "a head count without an AgendaCalendars read cannot supply colors"
        );

        assert!(topics.remember_read(
            &ReadRequest::AgendaCalendars { cursor: None },
            TopicId::Agenda,
            Knowledge::Present {
                value: orbit_core::topics::Page {
                    items: vec![serde_json::json!({
                        "reference": {"calendar_id": "new-calendar", "source_id": "new-source"},
                        "title": "Calendario nuevo",
                        "source_title": "Cuenta nueva",
                        "calendar_type": "caldav"
                    })],
                    next: None,
                    total: 1,
                    revision: 2,
                    store_version: None,
                },
                as_of_ms: None,
            },
        ));
        let view = project(&FeatureInput::new(&snapshot, &topics));
        assert_eq!(view.calendars.len(), 1);
        assert_eq!(view.calendars[0].id, "new-calendar");
        assert_eq!(
            view.calendars[0].color,
            crate::projection::stable_calendar_color("new-calendar")
        );
    }

    #[test]
    fn refused_and_unreadable_calendar_listings_carry_their_causes() {
        let snapshot = crate::AppSnapshot::default();
        let mut topics = TopicCache::default();
        topics.observe_at(
            TopicFrame {
                epoch: "fixture".into(),
                revision: 2,
                data: TopicData::Agenda(Knowledge::Present {
                    value: Value {
                        invalidation: 0,
                        calendars: Some(1),
                        read_window: None,
                        read_id: None,
                        timezone: None,
                        writes: Vec::new(),
                    },
                    as_of_ms: None,
                }),
            },
            Some(1),
        );
        let request = ReadRequest::AgendaCalendars { cursor: None };
        let refusal = Refusal::Legacy {
            text: "permiso de lectura rechazado: 1 de 1 solicitudes".into(),
        };
        assert!(topics.remember_read(
            &request,
            TopicId::Agenda,
            Knowledge::Absent {
                refusal: refusal.clone(),
            },
        ));
        let refused = project(&FeatureInput::new(&snapshot, &topics));
        assert_eq!(
            refused.reading.state,
            crate::CalendarReadingState::Unavailable
        );
        assert!(
            refused
                .reading
                .detail
                .as_deref()
                .is_some_and(|cause| cause.contains("1 de 1"))
        );
        assert!(refused.calendars.is_empty());

        assert!(topics.remember_read(
            &request,
            TopicId::Agenda,
            Knowledge::Present {
                value: orbit_core::topics::Page {
                    items: vec![serde_json::json!({"title": "Sin referencia"})],
                    next: None,
                    total: 1,
                    revision: 2,
                    store_version: None,
                },
                as_of_ms: None,
            },
        ));
        let unreadable = project(&FeatureInput::new(&snapshot, &topics));
        assert_eq!(
            unreadable.reading.state,
            crate::CalendarReadingState::Unavailable
        );
        assert!(
            unreadable
                .reading
                .detail
                .as_deref()
                .is_some_and(|cause| cause.contains("AgendaCalendars: calendario 0 ilegible"))
        );
        assert!(unreadable.calendars.is_empty());
    }

    #[test]
    fn unknown_agenda_head_never_turns_an_old_empty_snapshot_into_a_fact() {
        let mut snapshot = crate::AppSnapshot::empty(1, crate::DaemonConnection::Ready);
        snapshot.calendar.reading.state = crate::CalendarReadingState::Available;
        let mut topics = TopicCache::default();
        topics.observe_at(
            TopicFrame {
                epoch: "fixture".into(),
                revision: 1,
                data: TopicData::not_observed(TopicId::Agenda),
            },
            Some(1),
        );
        let view = project(&FeatureInput::new(&snapshot, &topics));
        assert_eq!(view.reading.state, crate::CalendarReadingState::Unknown);
        snapshot.calendar.authorization = crate::CalendarAuthorizationSnapshot::NotDetermined;
        let view = project(&FeatureInput::new(&snapshot, &topics));
        assert_eq!(
            view.authorization,
            crate::CalendarAuthorizationSnapshot::NotDetermined
        );
    }

    #[test]
    fn view_and_deimos_receive_the_same_permission_cause() {
        let mut topics = TopicCache::default();
        let access = orbit_core::topics::agenda::access(CalendarAuthorization::Denied, false);
        let Knowledge::Absent { refusal } = access else {
            panic!("denied macOS permission must be absent");
        };
        assert_eq!(
            refusal,
            Refusal::PermissionDenied {
                permission: Permission::Calendar
            }
        );
        topics.observe_at(
            TopicFrame {
                epoch: "fixture".into(),
                revision: 1,
                data: TopicData::Agenda(Knowledge::Absent { refusal }),
            },
            Some(1),
        );
        let snapshot = crate::AppSnapshot::empty(1, crate::DaemonConnection::Ready);
        let view = project(&FeatureInput::new(&snapshot, &topics));
        assert_eq!(
            view.authorization,
            crate::CalendarAuthorizationSnapshot::Denied
        );
        assert_eq!(
            view.reading.blocker,
            Some(crate::CalendarReadingBlocker::CalendarPermission)
        );
        assert_eq!(
            view.reading.detail.as_deref(),
            Some(
                Refusal::PermissionDenied {
                    permission: Permission::Calendar
                }
                .to_string()
                .as_str()
            )
        );
    }

    #[test]
    fn unavailable_eventkit_is_not_a_missing_app_in_the_view() {
        let Knowledge::Absent { refusal } =
            orbit_core::topics::agenda::access(CalendarAuthorization::Unavailable, false)
        else {
            panic!("unavailable EventKit must have a cause");
        };
        let mut topics = TopicCache::default();
        topics.observe_at(
            TopicFrame {
                epoch: "fixture".into(),
                revision: 1,
                data: TopicData::Agenda(Knowledge::Absent { refusal }),
            },
            Some(1),
        );
        let snapshot = crate::AppSnapshot::empty(1, crate::DaemonConnection::Ready);
        let view = project(&FeatureInput::new(&snapshot, &topics));
        assert_eq!(view.reading.state, crate::CalendarReadingState::Unavailable);
        assert_ne!(
            view.reading.blocker,
            Some(crate::CalendarReadingBlocker::HelperNotRunning)
        );
        let detail = view.reading.detail.as_deref().unwrap_or("");
        assert!(
            serde_json::from_str::<serde_json::Value>(detail)
                .is_ok_and(|value| value["code"] == "calendar_backend_unavailable")
        );
        assert!(!detail.contains("Orbit.app no está en marcha"));
    }

    #[test]
    fn agenda_projection_joins_successive_keyset_pages() -> Result<(), Box<dyn std::error::Error>> {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../contracts/fixtures/calendar-local-read-v1.json"
        ))?;
        let native: orbit_core::calendar::CalendarSnapshot =
            serde_json::from_value(fixture["snapshot"].clone())?;
        let first = native.events.first().ok_or("event in synthetic fixture")?;
        let mut second = first.clone();
        second.observation_id = "second-fixture-event".into();
        second.start_unix_ms += 60_000;
        second.end_unix_ms += 60_000;
        let cursor = Cursor::StartAndId {
            start_ms: first.start_unix_ms,
            id: first.observation_id.clone(),
        };
        let mut topics = TopicCache::default();
        topics.observe_at(
            TopicFrame {
                epoch: "fixture".into(),
                revision: 1,
                data: TopicData::Agenda(Knowledge::Present {
                    value: Value {
                        invalidation: 0,
                        calendars: Some(1),
                        read_window: Some(orbit_core::calendar::CalendarWindow {
                            start_unix_ms: native.window.start_unix_ms,
                            end_unix_ms: native.window.end_unix_ms,
                        }),
                        read_id: Some(native.snapshot_id.clone()),
                        timezone: Some(native.system_timezone.clone()),
                        writes: Vec::new(),
                    },
                    as_of_ms: None,
                }),
            },
            Some(1),
        );
        topics.observe_at(
            TopicFrame {
                epoch: "fixture".into(),
                revision: 1,
                data: TopicData::Status(Knowledge::Present {
                    value: orbit_core::topics::status::Value {
                        attachments: None,
                        assistant_name: None,
                        agents: Vec::new(),
                        default_agent_id: None,
                        addable_agents: None,
                        hub: Connection::Offline,
                        account: None,
                        privacy_stopped: false,
                        helper_connected: true,
                        error_reports: None,
                    },
                    as_of_ms: None,
                }),
            },
            Some(1),
        );
        for (request_cursor, occurrence, next) in [
            (None, first.clone(), Some(cursor.clone())),
            (Some(cursor), second, None),
        ] {
            assert!(topics.remember_read(
                &ReadRequest::AgendaRange {
                    start_ms: native.window.start_unix_ms,
                    end_ms: native.window.end_unix_ms,
                    cursor: request_cursor,
                },
                TopicId::Agenda,
                Knowledge::Present {
                    value: orbit_core::topics::Page {
                        items: vec![serde_json::to_value(occurrence)?],
                        next,
                        total: 2,
                        revision: 1,
                        store_version: None,
                    },
                    as_of_ms: None,
                },
            ));
        }
        let snapshot = crate::AppSnapshot::empty(1, crate::DaemonConnection::Ready);
        let view = project(&FeatureInput::new(&snapshot, &topics));
        assert_eq!(view.events.len(), 2);
        assert!(!view.stale);
        assert_eq!(view.reading.state, crate::CalendarReadingState::Unavailable);
        assert_eq!(
            view.reading.blocker,
            Some(crate::CalendarReadingBlocker::NotPaired)
        );
        let refusal = Refusal::StorageFailed {
            operation: orbit_core::refusal::Operation::Calendar,
            cause: None,
        };
        assert!(topics.remember_read(
            &ReadRequest::AgendaRange {
                start_ms: native.window.start_unix_ms,
                end_ms: native.window.end_unix_ms,
                cursor: None,
            },
            TopicId::Agenda,
            Knowledge::Absent {
                refusal: refusal.clone(),
            },
        ));
        let refused = project(&FeatureInput::new(&snapshot, &topics));
        assert_eq!(
            refused.reading.detail.as_deref(),
            Some(refusal.to_string().as_str())
        );
        assert!(!refused.stale);
        Ok(())
    }
}
