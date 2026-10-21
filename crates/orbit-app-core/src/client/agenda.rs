use super::*;
use orbit_core::{
    reads::ReadRequest,
    topics::{Knowledge, TopicData, TopicId},
};

impl DaemonClient {
    pub(super) async fn send_calendar(
        &self,
        command: crate::AgendaCommand,
    ) -> Result<AppSnapshot, AppError> {
        if let crate::AgendaCommand::CalendarSetRange {
            view,
            anchor_ms,
            days,
            week_start,
        } = command
        {
            // The period on screen moves at once, before waiting for
            // a read already running; that read, or this one, then reads the
            // latest period asked for.
            let range = {
                let mut calendar = self.inner.calendar.lock().await;
                calendar
                    .set_range(crate::calendar::RangeRequest {
                        view,
                        anchor_ms,
                        days,
                        week_start,
                    })
                    .map_err(AppError::calendar)?;
                calendar.range()
            };
            self.update_local(|snapshot| snapshot.calendar.range = range);
            let _guard = self.inner.calendar_io.lock().await;
            self.read_visible_range().await?;
            return Ok(self.local_snapshot());
        }
        if command == crate::AgendaCommand::CalendarShowToday {
            self.show_today().await?;
            return Ok(self.local_snapshot());
        }
        let _guard = self.inner.calendar_io.lock().await;
        match command {
            crate::AgendaCommand::CalendarRefresh => {
                self.read_visible_range().await?;
            }
            crate::AgendaCommand::CalendarSetRange { .. }
            | crate::AgendaCommand::CalendarShowToday => {}
        }
        Ok(self.local_snapshot())
    }

    /// The window the daemon last confirmed for the agenda, or, before any,
    /// the one app-core asked for. `None` while the agenda is not readable.
    async fn recorded_window(&self) -> Option<(i64, i64)> {
        let cache = self.inner.topics.read().await;
        let TopicData::Agenda(Knowledge::Present { value, .. }) =
            &cache.topic(TopicId::Agenda)?.knowledge
        else {
            return None;
        };
        Some(value.read_window.map_or_else(
            || {
                let snapshot = self.inner.snapshots.borrow();
                (
                    snapshot.calendar.window.start_ms,
                    snapshot.calendar.window.end_ms,
                )
            },
            |window| (window.start_unix_ms, window.end_unix_ms),
        ))
    }

    /// The Briefing asks for today. See `TodayRead`.
    async fn show_today(&self) -> Result<(), AppError> {
        let Some(read) = self.recorded_window().await else {
            return Ok(());
        };
        let range = {
            let mut calendar = self.inner.calendar.lock().await;
            match calendar.today_read(read) {
                crate::calendar::TodayRead::Covered => return Ok(()),
                crate::calendar::TodayRead::ReadAgain => None,
                crate::calendar::TodayRead::ReturnToToday => {
                    calendar.return_to_today();
                    Some(calendar.range())
                }
            }
        };
        if let Some(range) = range {
            self.update_local(|snapshot| snapshot.calendar.range = range);
        }
        let _guard = self.inner.calendar_io.lock().await;
        self.read_visible_range().await
    }

    /// On the clock's tick, after local midnight, the range is
    /// read again when the read on record stopped holding the new day.
    pub(super) async fn read_new_day(&self) -> Result<(), AppError> {
        let Some(read) = self.recorded_window().await else {
            return Ok(());
        };
        if self.inner.calendar.lock().await.today_read(read)
            != crate::calendar::TodayRead::ReadAgain
        {
            return Ok(());
        }
        let _guard = self.inner.calendar_io.lock().await;
        self.read_visible_range().await
    }

    /// Reads the period on screen, widened to today, and waits for
    /// the agenda topic to confirm exactly that window with a new read.
    async fn read_visible_range(&self) -> Result<(), AppError> {
        let (start_unix_ms, end_unix_ms) = self.inner.calendar.lock().await.read_window();
        let window = CalendarWindow {
            start_unix_ms,
            end_unix_ms,
        };
        if window.validate().is_err() {
            return Err(AppError::calendar(window_refusal(window)));
        }
        let (prior_id, prior_revision) = {
            let cache = self.inner.topics.read().await;
            let head = cache.topic(TopicId::Agenda);
            let id = if let Some(TopicData::Agenda(Knowledge::Present { value, .. })) =
                head.map(|head| &head.knowledge)
            {
                value.read_id.clone()
            } else {
                None
            };
            (id, head.map(|head| head.revision))
        };
        self.update_local(|snapshot| {
            snapshot.calendar.window.start_ms = start_unix_ms;
            snapshot.calendar.window.end_ms = end_unix_ms;
            snapshot.calendar.stale = true;
        });
        let mut changes = self.watch_frames();
        let response = self
            .inner
            .daemon
            .request_command(Command::Calendar {
                request: CalendarCommand::Read { window },
            })
            .await
            .map_err(|error| AppError::from_calendar(error.as_ref()))?;
        // A repeated denial need not advance a watch revision. The response
        // names the actor's current head after admitting this read; propagate
        // that refusal even when it is identical to the previous one. A cached
        // head from before the command cannot establish this causal barrier.
        if let Some(receiver) = self.inner.daemon.topics().get(&TopicId::Agenda) {
            let frame = receiver.borrow();
            if response.epoch.as_deref() == Some(frame.epoch.as_str())
                && response
                    .revisions
                    .as_ref()
                    .and_then(|revisions| revisions.get(&TopicId::Agenda))
                    == Some(&frame.revision)
                && let TopicData::Agenda(Knowledge::Absent { refusal }) = &frame.data
            {
                return Err(AppError::from_calendar(refusal));
            }
        }
        tokio::time::timeout(READ_WAIT, async {
            loop {
                let result = {
                    let cache = self.inner.topics.read().await;
                    completed_agenda_read(
                        cache.topic(TopicId::Agenda).map(|head| &head.knowledge),
                        window,
                        prior_id.as_deref(),
                        prior_revision,
                        cache.topic(TopicId::Agenda).map(|head| head.revision),
                    )
                };
                if let Some(result) = result {
                    return result;
                }
                if changes.changed().await.is_err() {
                    return Err(AppError::calendar("Se perdió la suscripción al tema agenda."));
                }
            }
        })
        .await
        .map_err(|_| {
            AppError::calendar(format!(
                "el tema agenda no confirmó la ventana {start_unix_ms}..{end_unix_ms} con un read_id nuevo en {} ms",
                READ_WAIT.as_millis()
            ))
        })?
    }
}

/// Why `window` cannot be asked for, with its numbers: an instant outside
/// the calendar the daemon reads, or an end that does not pass the start.
fn window_refusal(window: CalendarWindow) -> String {
    use orbit_core::calendar::{MAX_UNIX_MS, MIN_UNIX_MS};
    let CalendarWindow {
        start_unix_ms: start,
        end_unix_ms: end,
    } = window;
    let outside = |instant: i64| !(MIN_UNIX_MS..=MAX_UNIX_MS).contains(&instant);
    if outside(start) || outside(end) {
        format!(
            "ventana de calendario con un instante inválido: start_ms={start}, end_ms={end}; \
             cada instante debe estar entre {MIN_UNIX_MS} y {MAX_UNIX_MS}"
        )
    } else {
        format!(
            "ventana de calendario vacía: end_ms={end} no supera a start_ms={start} \
             ({} ms)",
            end.saturating_sub(start)
        )
    }
}

fn completed_agenda_read(
    data: Option<&TopicData>,
    window: CalendarWindow,
    prior_id: Option<&str>,
    prior_revision: Option<u64>,
    current_revision: Option<u64>,
) -> Option<Result<(), AppError>> {
    match data {
        Some(TopicData::Agenda(Knowledge::Present { value, .. }))
            if value.read_window == Some(window)
                && value
                    .read_id
                    .as_deref()
                    .is_some_and(|id| Some(id) != prior_id) =>
        {
            Some(Ok(()))
        }
        Some(TopicData::Agenda(Knowledge::Absent { refusal }))
            if current_revision.is_some_and(|revision| Some(revision) != prior_revision) =>
        {
            Some(Err(AppError::from_calendar(refusal)))
        }
        Some(TopicData::Agenda(
            Knowledge::Unknown { .. } | Knowledge::Present { .. } | Knowledge::Absent { .. },
        ))
        | None => None,
        Some(
            TopicData::Status(_)
            | TopicData::Permissions(_)
            | TopicData::Recording(_)
            | TopicData::Notes(_)
            | TopicData::Assistant(_)
            | TopicData::Control(_)
            | TopicData::Notices(_),
        ) => None,
    }
}

pub(super) async fn hydrate_agenda(
    daemon: &LocalClient,
    topics: &RwLock<crate::topics::TopicCache>,
) {
    let (revision, window) = {
        let cache = topics.read().await;
        let Some(head) = cache.topic(TopicId::Agenda) else {
            return;
        };
        let TopicData::Agenda(Knowledge::Present { value, .. }) = &head.knowledge else {
            return;
        };
        if value.read_id.is_none() {
            return;
        }
        let Some(window) = value.read_window else {
            return;
        };
        (head.revision, window)
    };
    let mut cursor = None;
    let mut visited = Vec::new();
    loop {
        let request = ReadRequest::AgendaRange {
            start_ms: window.start_unix_ms,
            end_ms: window.end_unix_ms,
            cursor,
        };
        let response = match daemon.read_topic(request.clone()).await {
            Ok(response) => response,
            Err(error) => {
                refuse_agenda_read(
                    topics,
                    &request,
                    revision,
                    format!("AgendaRange: no se pudo leer la ventana: {error}"),
                )
                .await;
                return;
            }
        };
        if response.revisions.get(&TopicId::Agenda) != Some(&revision) {
            return;
        }
        if response.read != Some(request.id()) {
            refuse_agenda_read(
                topics,
                &request,
                revision,
                format!(
                    "AgendaRange: respuesta para {:?}, se pidió {:?}",
                    response.read,
                    request.id()
                ),
            )
            .await;
            return;
        }
        let Some(answer) = response.answer else {
            refuse_agenda_read(
                topics,
                &request,
                revision,
                "AgendaRange: falta la respuesta del daemon".into(),
            )
            .await;
            return;
        };
        let next = match &answer {
            Knowledge::Present { value, .. } => value.next.clone(),
            Knowledge::Absent { .. } | Knowledge::Unknown { .. } => None,
        };
        if !topics
            .write()
            .await
            .remember_read(&request, TopicId::Agenda, answer)
        {
            return;
        }
        match next {
            Some(next) if !visited.contains(&next) => {
                visited.push(next.clone());
                cursor = Some(next);
            }
            Some(_) => return,
            None => return,
        }
    }
}

pub(super) async fn hydrate_agenda_calendars(
    daemon: &LocalClient,
    topics: &RwLock<crate::topics::TopicCache>,
) {
    let revision = {
        let cache = topics.read().await;
        let Some(head) = cache.topic(TopicId::Agenda) else {
            return;
        };
        if !matches!(head.knowledge, TopicData::Agenda(Knowledge::Present { .. })) {
            return;
        }
        head.revision
    };
    let mut cursor = None;
    let mut visited = Vec::new();
    loop {
        let request = ReadRequest::AgendaCalendars {
            cursor: cursor.clone(),
        };
        let response = match daemon.read_topic(request.clone()).await {
            Ok(response) => response,
            Err(error) => {
                refuse_agenda_read(
                    topics,
                    &request,
                    revision,
                    format!("AgendaCalendars: no se pudo leer la lista: {error}"),
                )
                .await;
                return;
            }
        };
        if response.revisions.get(&TopicId::Agenda) != Some(&revision) {
            return;
        }
        if response.read != Some(request.id()) {
            refuse_agenda_read(
                topics,
                &request,
                revision,
                format!(
                    "AgendaCalendars: respuesta para {:?}, se pidió {:?}",
                    response.read,
                    request.id()
                ),
            )
            .await;
            return;
        }
        let Some(answer) = response.answer else {
            refuse_agenda_read(
                topics,
                &request,
                revision,
                "AgendaCalendars: falta la respuesta del daemon".into(),
            )
            .await;
            return;
        };
        let next = match &answer {
            Knowledge::Present { value, .. } => value.next.clone(),
            Knowledge::Absent { .. } | Knowledge::Unknown { .. } => None,
        };
        if !topics
            .write()
            .await
            .remember_read(&request, TopicId::Agenda, answer)
        {
            return;
        }
        match next {
            Some(next @ orbit_core::topics::Cursor::Id { .. }) if !visited.contains(&next) => {
                visited.push(next.clone());
                cursor = Some(next);
            }
            Some(_) | None => return,
        }
    }
}

async fn refuse_agenda_read(
    topics: &RwLock<crate::topics::TopicCache>,
    request: &ReadRequest,
    revision: u64,
    cause: String,
) {
    let mut cache = topics.write().await;
    if cache.topic(TopicId::Agenda).map(|head| head.revision) == Some(revision) {
        cache.remember_read(
            request,
            TopicId::Agenda,
            Knowledge::Absent {
                refusal: orbit_core::refusal::Refusal::DaemonReplyInvalid {
                    field: "calendar.events".into(),
                    issue: orbit_core::refusal::DaemonReplyIssue::Decode { error: cause },
                },
            },
        );
    }
}

#[cfg(test)]
mod completion_tests {
    use super::*;
    use orbit_core::{refusal::Refusal, topics::agenda::Value};

    #[test]
    fn a_refused_window_names_its_cause_and_numbers() {
        let invalid = window_refusal(CalendarWindow {
            start_unix_ms: orbit_core::calendar::MAX_UNIX_MS + 1,
            end_unix_ms: orbit_core::calendar::MAX_UNIX_MS + 2,
        });
        assert!(invalid.contains("instante inválido"), "{invalid}");
        assert!(invalid.contains("start_ms=253402300800000"), "{invalid}");
        assert!(invalid.contains("253402300799999"), "{invalid}");
        let empty = window_refusal(CalendarWindow {
            start_unix_ms: 1_000,
            end_unix_ms: 400,
        });
        assert!(
            empty.contains("end_ms=400 no supera a start_ms=1000"),
            "{empty}"
        );
        assert!(empty.contains("(-600 ms)"), "{empty}");
    }

    #[test]
    fn next_week_waits_for_the_requested_window_and_a_new_read_id_or_refusal() {
        let requested = CalendarWindow {
            start_unix_ms: 7,
            end_unix_ms: 14,
        };
        let previous = CalendarWindow {
            start_unix_ms: 0,
            end_unix_ms: 7,
        };
        let mut value = Value {
            invalidation: 0,
            calendars: Some(2),
            read_window: Some(previous),
            read_id: Some("old".into()),
            timezone: None,
            writes: Vec::new(),
        };
        let present = |value: Value| {
            TopicData::Agenda(Knowledge::Present {
                value,
                as_of_ms: None,
            })
        };
        assert!(
            completed_agenda_read(
                Some(&present(value.clone())),
                requested,
                Some("old"),
                Some(4),
                Some(4)
            )
            .is_none()
        );
        value.read_window = Some(requested);
        assert!(
            completed_agenda_read(
                Some(&present(value.clone())),
                requested,
                Some("old"),
                Some(4),
                Some(5)
            )
            .is_none()
        );
        value.read_id = Some("new".into());
        assert!(matches!(
            completed_agenda_read(
                Some(&present(value)),
                requested,
                Some("old"),
                Some(4),
                Some(5)
            ),
            Some(Ok(()))
        ));
        let refused = TopicData::Agenda(Knowledge::Absent {
            refusal: Refusal::Legacy {
                text: "ventana 7..14: lectura rechazada".into(),
            },
        });
        assert!(
            completed_agenda_read(Some(&refused), requested, Some("old"), Some(4), Some(4))
                .is_none()
        );
        let error = completed_agenda_read(Some(&refused), requested, Some("old"), Some(4), Some(5))
            .and_then(Result::err);
        assert!(error.is_some_and(|error| error.to_string().contains("ventana 7..14")));
    }
}

#[cfg(test)]
mod refusal_tests {
    use super::*;

    #[test]
    fn agenda_topic_preserves_typed_refusal_and_calendar_context()
    -> Result<(), Box<dyn std::error::Error>> {
        let refusal = orbit_core::refusal::Refusal::NotPaired;
        let data = TopicData::Agenda(Knowledge::Absent {
            refusal: refusal.clone(),
        });
        let window = CalendarWindow {
            start_unix_ms: 0,
            end_unix_ms: 1,
        };
        let error = completed_agenda_read(Some(&data), window, None, Some(1), Some(2))
            .ok_or("new absent revision must complete the read")?
            .err()
            .ok_or("absent agenda must refuse the read")?;
        assert_eq!(error.code, "calendar_error");
        assert_eq!(error.message, format!("calendar_error: {refusal}"));
        let error = crate::CommandError::from_app_error(
            crate::AppCommand::Agenda(crate::AgendaCommand::CalendarRefresh),
            error,
        );
        assert_eq!(*error.refusal, refusal);
        Ok(())
    }
}
