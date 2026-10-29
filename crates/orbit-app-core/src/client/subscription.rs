use super::*;

pub(super) fn spawn_all(daemon: LocalClient, channels: &Channels) -> (AbortHandle, AbortHandle) {
    let topics = Arc::clone(&channels.topics);
    let frames = Arc::clone(&channels.frames);
    let topic_snapshots = channels.snapshots.clone();
    let task = tokio::spawn(async move {
        topic_subscription_loop(daemon, topic_snapshots, topics, frames).await;
    });
    let clock_snapshots = channels.snapshots.clone();
    let clock_frames = Arc::clone(&channels.frames);
    let clock = Arc::clone(&channels.clock);
    let maintenance = tokio::spawn(async move {
        clock_loop(clock_snapshots, clock_frames, clock).await;
    });
    (task.abort_handle(), maintenance.abort_handle())
}

/// The clock's tick: every minute, or when an injected clock says so.
///
/// One minute protects local-day and zone labels from staying stale longer
/// than the smallest minute the app paints ("10:00", "en 5 min"). It is not a
/// limit on instant precision or how often the system zone can change.
struct ClockTicks {
    interval: tokio::time::Interval,
    injected: Option<watch::Receiver<()>>,
}

impl ClockTicks {
    fn new(clock: &dyn CalendarClock) -> Self {
        const CLOCK_TICK: Duration = Duration::from_secs(60);
        let mut interval = tokio::time::interval(CLOCK_TICK);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        Self {
            interval,
            injected: clock.subscribe_ticks(),
        }
    }

    async fn next(&mut self) {
        loop {
            let Some(ticks) = self.injected.as_mut() else {
                self.interval.tick().await;
                return;
            };
            tokio::select! {
                _ = self.interval.tick() => return,
                result = ticks.changed() => {
                    if result.is_err() {
                        self.injected = None;
                        continue;
                    }
                    return;
                }
            }
        }
    }
}

async fn clock_loop(
    snapshots: watch::Sender<AppSnapshot>,
    frames: Arc<StdMutex<FramePublisher>>,
    clock: Arc<dyn CalendarClock>,
) {
    let mut ticks = ClockTicks::new(clock.as_ref());
    loop {
        ticks.next().await;
        publish_clock_tick(&snapshots, &frames, clock.as_ref());
    }
}

/// On each tick, the agenda is read again after local midnight
/// if the read on record no longer holds the new day. A read the daemon
/// refuses reaches the agenda view through its own topic.
pub(super) async fn calendar_day_loop(
    inner: std::sync::Weak<ClientInner>,
    clock: Arc<dyn CalendarClock>,
) {
    let mut ticks = ClockTicks::new(clock.as_ref());
    loop {
        ticks.next().await;
        let Some(inner) = inner.upgrade() else {
            return;
        };
        if let Err(error) = (DaemonClient { inner }).read_new_day().await {
            drop(error);
        }
    }
}

async fn topic_subscription_loop(
    daemon: LocalClient,
    snapshots: watch::Sender<AppSnapshot>,
    topics: Arc<RwLock<crate::topics::TopicCache>>,
    frames: Arc<StdMutex<FramePublisher>>,
) {
    let mut watchers = tokio::task::JoinSet::new();
    for mut receiver in daemon.topics().into_values() {
        let daemon = daemon.clone();
        let snapshots = snapshots.clone();
        let topics = Arc::clone(&topics);
        let frames = Arc::clone(&frames);
        watchers.spawn(async move {
            loop {
                let frame = receiver.borrow_and_update().clone();
                let agenda = frame.id() == orbit_core::topics::TopicId::Agenda;
                let control = frame.id() == orbit_core::topics::TopicId::Control;
                topics.write().await.observe(frame);
                if agenda {
                    start_agenda_window(&daemon, &snapshots, &topics).await;
                    hydrate_agenda_calendars(&daemon, &topics).await;
                    hydrate_agenda(&daemon, &topics).await;
                }
                if control {
                    hydrate_control(&daemon, &topics).await;
                }
                publish_topic_change(&snapshots, &topics, &frames).await;
                if receiver.changed().await.is_err() {
                    break;
                }
            }
        });
    }
    let ended = daemon.ended().await;
    watchers.abort_all();
    let mut cache = topics.write().await;
    // The last publication (including orbit_closing) can precede the thread's
    // end without giving the app's watcher another turn. Keep it before stale.
    for receiver in daemon.topics().into_values() {
        cache.observe(receiver.borrow().clone());
    }
    cache.ended(ended.describe());
    drop(cache);
    publish_topic_change(&snapshots, &topics, &frames).await;
}

async fn hydrate_control(daemon: &LocalClient, topics: &RwLock<crate::topics::TopicCache>) {
    use orbit_core::{
        reads::ReadRequest,
        refusal::Refusal,
        topics::{Knowledge, TopicData, TopicId, control::Engine},
    };
    let revision = {
        let cache = topics.read().await;
        let Some(head) = cache.topic(TopicId::Control) else {
            return;
        };
        let TopicData::Control(Knowledge::Present { value, .. }) = &head.knowledge else {
            return;
        };
        if !matches!(
            value.engine,
            Knowledge::Present {
                value: Engine::Ready { .. },
                ..
            }
        ) {
            return;
        }
        head.revision
    };
    let request = ReadRequest::EngineTools { cursor: None };
    let answer = match daemon.read_topic(request.clone()).await {
        Ok(response) if response.revisions.get(&TopicId::Control) == Some(&revision) => {
            response.answer.unwrap_or(Knowledge::Absent {
                refusal: Refusal::DaemonReplyInvalid {
                    field: "engine.tools".into(),
                    issue: orbit_core::refusal::DaemonReplyIssue::Missing,
                },
            })
        }
        Ok(_) => return,
        Err(error) => Knowledge::Absent {
            refusal: Refusal::DaemonReplyInvalid {
                field: "engine.tools".into(),
                issue: orbit_core::refusal::DaemonReplyIssue::Decode {
                    error: error.to_string(),
                },
            },
        },
    };
    let _ = topics
        .write()
        .await
        .remember_read(&request, TopicId::Control, answer);
}

async fn start_agenda_window(
    daemon: &LocalClient,
    snapshots: &watch::Sender<AppSnapshot>,
    topics: &RwLock<crate::topics::TopicCache>,
) {
    let needs_window = {
        let cache = topics.read().await;
        matches!(
            cache.topic(orbit_core::topics::TopicId::Agenda),
            Some(crate::topics::CachedTopic {
                knowledge: orbit_core::topics::TopicData::Agenda(
                    orbit_core::topics::Knowledge::Present { value, .. }
                ),
                ..
            }) if value.read_window.is_none()
        )
    };
    if !needs_window {
        return;
    }
    let window = {
        let snapshot = snapshots.borrow();
        CalendarWindow {
            start_unix_ms: snapshot.calendar.window.start_ms,
            end_unix_ms: snapshot.calendar.window.end_ms,
        }
    };
    if window.validate().is_err() {
        return;
    }
    let refusal = match daemon
        .request_command(Command::Calendar {
            request: CalendarCommand::Read { window },
        })
        .await
    {
        Ok(_) => None,
        Err(error) => Some(error.to_string()),
    };
    snapshots.send_modify(|snapshot| {
        if let Some(cause) = &refusal {
            snapshot.calendar.reading.state = crate::CalendarReadingState::Unavailable;
            snapshot.calendar.reading.blocker = None;
            snapshot.calendar.reading.detail = Some(format!(
                "{}{cause}",
                crate::calendar::AGENDA_REQUEST_FAILED_PREFIX
            ));
            snapshot.calendar.stale = false;
        } else if snapshot
            .calendar
            .reading
            .detail
            .as_deref()
            .is_some_and(|detail| detail.starts_with(crate::calendar::AGENDA_REQUEST_FAILED_PREFIX))
        {
            snapshot.calendar.reading.state = crate::CalendarReadingState::Unknown;
            snapshot.calendar.reading.detail = None;
        }
    });
}

pub(super) async fn publish_topic_change(
    snapshots: &watch::Sender<AppSnapshot>,
    topics: &RwLock<crate::topics::TopicCache>,
    frames: &StdMutex<FramePublisher>,
) {
    let cache = topics.read().await;
    let mut publisher = match frames.lock() {
        Ok(publisher) => publisher,
        Err(poisoned) => poisoned.into_inner(),
    };
    // Clock publication and snapshot readers take the frame lock first.
    let snapshot = snapshots.borrow();
    publisher.observe_topics(&snapshot, &cache);
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod topic_tests {
    use super::*;
    use crate::client_fixture;
    use orbit_core::{
        protocol::{Response, TopicResponse},
        refusal::Refusal,
        topics::{Knowledge, TopicData, TopicFrame, TopicId},
    };
    use tokio::sync::mpsc;

    // This bounds a stuck synthetic IPC fixture, not the daemon's read time.
    // The neighboring topic test measured three deliveries within 0.04 s at
    // load 39-41; five seconds leaves room for shared-machine scheduling.
    const IPC_FIXTURE_WAIT: Duration = Duration::from_secs(5);

    #[tokio::test]
    async fn accepted_agenda_window_waits_for_its_topic() {
        let (calls, mut incoming) = mpsc::channel::<orbit_ipc::Call>(2);
        let endpoint = client_fixture::channel(calls, orbit_ipc::TopicChannels::new());
        let daemon = tokio::spawn(async move {
            let call = incoming.recv().await.unwrap();
            assert!(matches!(call.request.body, Command::Calendar { .. }));
            call.reply
                .send(Response {
                    v: 1,
                    id: call.request.id,
                    ok: true,
                    status: None,
                    epoch: Some("fixture".into()),
                    revisions: Some([(TopicId::Agenda, 1)].into()),
                    error: None,
                    refusal: None,
                })
                .unwrap();
        });
        let mut snapshot = crate::AppSnapshot::empty(1, crate::DaemonConnection::Ready);
        snapshot.calendar.window.start_ms = 1_700_000_000_000;
        snapshot.calendar.window.end_ms = 1_700_086_400_000;
        let (snapshots, _) = watch::channel(snapshot);
        let mut cache = crate::topics::TopicCache::default();
        cache.observe_at(
            TopicFrame {
                epoch: "fixture".into(),
                revision: 1,
                data: TopicData::Agenda(Knowledge::Present {
                    value: orbit_core::topics::agenda::Value {
                        invalidation: 0,
                        calendars: None,
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
        tokio::time::timeout(
            IPC_FIXTURE_WAIT,
            start_agenda_window(&endpoint.client, &snapshots, &RwLock::new(cache)),
        )
        .await
        .unwrap();
        daemon.await.unwrap();
        assert_eq!(
            snapshots.borrow().calendar.reading.state,
            crate::CalendarReadingState::Unknown
        );
        assert_eq!(snapshots.borrow().calendar.reading.detail, None);
    }

    #[tokio::test]
    async fn engine_tools_read_hydrates_the_control_frame() {
        let mut cache = crate::topics::TopicCache::default();
        cache.observe_at(
            TopicFrame {
                epoch: "fixture".into(),
                revision: 1,
                data: TopicData::Control(Knowledge::Present {
                    value: orbit_core::topics::control::Value {
                        engine: Knowledge::Present {
                            value: orbit_core::topics::control::Engine::Ready { tools: 1 },
                            as_of_ms: None,
                        },
                        retry_at_ms: None,
                        action: None,
                        history: Knowledge::Present {
                            value: orbit_core::topics::control::History {
                                total: 0,
                                recent: Vec::new(),
                            },
                            as_of_ms: None,
                        },
                        folders: Box::new(Knowledge::Unknown {
                            why: orbit_core::topics::Pending::NotObservedYet,
                        }),
                    },
                    as_of_ms: None,
                }),
            },
            Some(1),
        );
        let topics = RwLock::new(cache);
        let channels: orbit_ipc::TopicChannels = TopicId::ALL
            .iter()
            .copied()
            .map(|id| {
                let (sender, receiver) = watch::channel(TopicFrame {
                    epoch: "fixture".into(),
                    revision: 1,
                    data: TopicData::not_observed(id),
                });
                drop(receiver);
                (id, sender)
            })
            .collect();
        let (calls, mut incoming) = mpsc::channel(2);
        let listener = client_fixture::channel(calls, channels);
        let actor = tokio::spawn(async move {
            if let Some(call) = incoming.recv().await {
                assert!(matches!(
                    call.request.body,
                    Command::Read {
                        read: orbit_core::reads::ReadRequest::EngineTools { cursor: None }
                    }
                ));
                let _ = call.reply.send(TopicResponse {
                    v: orbit_core::PROTOCOL_VERSION,
                    id: call.request.id,
                    ok: true,
                    revisions: [(TopicId::Control, 1)].into(),
                    read: Some(orbit_core::reads::ReadId::EngineTools),
                    answer: Some(Knowledge::Present {
                        value: orbit_core::topics::Page {
                            items: vec![serde_json::json!("list_apps")],
                            next: None,
                            total: 1,
                            revision: 0,
                            store_version: None,
                            handed_over_from: None,
                        },
                        as_of_ms: None,
                    }),
                    refusal: None,
                });
            }
        });
        tokio::time::timeout(
            Duration::from_secs(5),
            hydrate_control(&listener.client, &topics),
        )
        .await
        .unwrap();
        let snapshot = crate::AppSnapshot::empty(1, crate::DaemonConnection::Ready);
        let guard = topics.read().await;
        let view = crate::features::control::project(&crate::features::FeatureInput::new(
            &snapshot, &guard,
        ))
        .unwrap();
        assert_eq!(view.engine_tools, ["list_apps"]);
        actor.await.unwrap();
        listener.abort();
    }

    #[tokio::test]
    async fn failed_window_request_is_visible_to_the_agenda() {
        let mut snapshot = crate::AppSnapshot::empty(1, crate::DaemonConnection::Ready);
        snapshot.calendar.window.start_ms = 1_700_000_000_000;
        snapshot.calendar.window.end_ms = 1_700_086_400_000;
        let (snapshots, _) = watch::channel(snapshot);
        let mut cache = crate::topics::TopicCache::default();
        cache.observe_at(
            TopicFrame {
                epoch: "fixture".into(),
                revision: 1,
                data: TopicData::Agenda(Knowledge::Present {
                    value: orbit_core::topics::agenda::Value {
                        invalidation: 0,
                        calendars: None,
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
        let topics = RwLock::new(cache);
        start_agenda_window(&client_fixture::idle(), &snapshots, &topics).await;
        let snapshot = snapshots.borrow().clone();
        let cache = topics.read().await;
        let view = crate::features::agenda::project(&crate::features::FeatureInput::new(
            &snapshot, &cache,
        ));
        assert_eq!(view.reading.state, crate::CalendarReadingState::Unavailable);
        assert!(!view.stale);
        let detail = view.reading.detail.as_deref().unwrap_or("");
        assert!(detail.starts_with(crate::calendar::AGENDA_REQUEST_FAILED_PREFIX));
        assert!(detail.contains("orbit_closing"));
    }

    #[tokio::test]
    async fn topic_change_notifies_frame_consumer_without_a_snapshot_change() {
        let snapshot = crate::AppSnapshot::empty(7, crate::DaemonConnection::Ready);
        let (snapshots, snapshot_changes) = watch::channel(snapshot.clone());
        let topics = Arc::new(RwLock::new(crate::topics::TopicCache::default()));
        let frames = Arc::new(StdMutex::new(FramePublisher::new(snapshot)));
        let mut frame_changes = frames.lock().unwrap().watch_frames();
        let channels: orbit_ipc::TopicChannels = TopicId::ALL
            .iter()
            .copied()
            .map(|id| {
                let (sender, receiver) = watch::channel(TopicFrame {
                    epoch: "synthetic".into(),
                    revision: 1,
                    data: TopicData::not_observed(id),
                });
                drop(receiver);
                (id, sender)
            })
            .collect();
        let status = channels[&TopicId::Status].clone();
        let agenda = channels[&TopicId::Agenda].clone();
        let (calls, mut incoming) = mpsc::channel(2);
        let listener = client_fixture::channel(calls, channels);
        let actor = tokio::spawn(async move {
            while let Some(call) = incoming.recv().await {
                let _ = call.reply.send(TopicResponse {
                    v: orbit_core::PROTOCOL_VERSION,
                    id: call.request.id,
                    ok: true,
                    revisions: TopicId::ALL.iter().copied().map(|id| (id, 1)).collect(),
                    read: None,
                    answer: None,
                    refusal: None,
                });
            }
        });
        let worker = tokio::spawn(topic_subscription_loop(
            listener.client.clone(),
            snapshots.clone(),
            Arc::clone(&topics),
            Arc::clone(&frames),
        ));
        // Reuse the local IPC hang guard from the neighboring tests. Three
        // successful runs at one-minute load 39-41 finished within 0.04 s
        // inside the test; five seconds bounds a broken fixture, not delivery.
        let ipc_hang_guard = Duration::from_secs(5);
        tokio::time::timeout(ipc_hang_guard, async {
            while frames
                .lock()
                .unwrap()
                .last_topics
                .topic(TopicId::Notices)
                .is_none()
            {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        frame_changes.borrow_and_update();
        status.send_replace(TopicFrame {
            epoch: "synthetic".into(),
            revision: 2,
            data: TopicData::not_observed(TopicId::Status),
        });
        tokio::time::timeout(ipc_hang_guard, frame_changes.changed())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            frames
                .lock()
                .unwrap()
                .last_topics
                .topic(TopicId::Status)
                .map(|topic| topic.revision),
            Some(2)
        );
        assert_eq!(snapshots.borrow().revision, 7);
        assert!(!snapshot_changes.has_changed().unwrap());
        // A changed agenda value travels through the same IPC and
        // frame queue without any legacy StateChanged event.
        frames.lock().unwrap().drain();
        frame_changes.borrow_and_update();
        agenda.send_replace(TopicFrame {
            epoch: "synthetic".into(),
            revision: 2,
            data: TopicData::Agenda(orbit_core::topics::Knowledge::Absent {
                refusal: Refusal::PrivacyStopped {
                    since_ms: Some(1234),
                },
            }),
        });
        tokio::time::timeout(ipc_hang_guard, frame_changes.changed())
            .await
            .unwrap()
            .unwrap();
        let agenda_frame = frames
            .lock()
            .unwrap()
            .drain()
            .into_iter()
            .find(|frame| frame.feature() == crate::features::FeatureId::Agenda);
        assert!(matches!(
            agenda_frame,
            Some(crate::features::FeatureFrame::Agenda { view, .. })
                if view.reading.blocker == Some(crate::CalendarReadingBlocker::PrivacyStopped)
        ));
        assert!(!snapshot_changes.has_changed().unwrap());
        worker.abort();
        actor.abort();
        listener.abort();
    }

    #[tokio::test]
    async fn daemon_end_keeps_the_last_closing_notice_without_a_watcher_turn() {
        let snapshot = crate::AppSnapshot::empty(1, crate::DaemonConnection::Ready);
        let (snapshots, _) = watch::channel(snapshot.clone());
        let topics = Arc::new(RwLock::new(crate::topics::TopicCache::default()));
        let frames = Arc::new(StdMutex::new(FramePublisher::new(snapshot)));
        let notice = orbit_core::notice::Notice::new(orbit_core::notice::NoticeCode::OrbitClosing);
        let (notices, _) = watch::channel(TopicFrame {
            epoch: "last-turn".into(),
            revision: 7,
            data: TopicData::Notices(Knowledge::Present {
                value: orbit_core::topics::notices::Value {
                    notice: Some(notice.clone()),
                    notices: vec![notice],
                },
                as_of_ms: None,
            }),
        });
        let (calls, _incoming) = mpsc::channel(1);
        let endpoint = client_fixture::channel(calls, [(TopicId::Notices, notices)].into());
        // The producer has published its last value and ended before a watcher
        // can run. Its end must not discard the notice that explains cut reads.
        endpoint.abort();
        topic_subscription_loop(endpoint.client, snapshots, Arc::clone(&topics), frames).await;
        let cache = topics.read().await;
        assert!(matches!(
            cache.connection(),
            crate::topics::TopicConnection::Ended { .. }
        ));
        assert!(matches!(cache.topic(TopicId::Notices), Some(topic)
            if matches!(&topic.knowledge, TopicData::Notices(Knowledge::Present { value, .. })
                if value.notice.as_ref().is_some_and(|notice| notice.code == orbit_core::notice::NoticeCode::OrbitClosing))));
    }

    #[tokio::test]
    async fn ended_thread_is_stale_and_retains_its_cause() {
        let directory = tempfile::tempdir().unwrap();
        let paths = Paths::resolve(Some(directory.path().to_owned())).unwrap();
        let (calls, _incoming) = mpsc::channel(2);
        let endpoint = client_fixture::channel(calls, orbit_ipc::TopicChannels::new());
        let client = DaemonClient::connect(paths, endpoint.client.clone());
        let mut changes = client.watch_frames();
        endpoint.abort();
        tokio::time::timeout(IPC_FIXTURE_WAIT, async {
            loop {
                if matches!(client.inner.topics.read().await.connection(), crate::topics::TopicConnection::Ended { cause } if cause.contains("Fixture: actor terminado")) { break; }
                changes.changed().await.unwrap();
            }
        }).await.unwrap();
        assert_eq!(
            client.composed_snapshot().connection.daemon,
            crate::DaemonConnection::Lost
        );
    }
}
