use crate::{
    AppCommand, AppError, AppSnapshot, LoginItemState, UpdateSnapshot, UpdateState,
    calendar::{CalendarClock, CalendarCoordinator, LocalClock, SystemCalendarClock},
    projection::project_update,
};
use orbit_core::{calendar::CalendarWindow, calendar_control::CalendarCommand, protocol::Command};
use orbit_ipc::{LocalClient, Paths};
use std::{
    fs,
    sync::{
        Arc, Mutex as StdMutex,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
};
use tokio::{
    sync::{Mutex, RwLock, watch},
    task::AbortHandle,
    time::Duration,
};

const PROFILE_NAME_FILE: &str = "app-profile-name";
/// How long a command that reads the visible week waits for the daemon to say
/// the read finished or why it did not.
///
/// Protects: the person who pressed «refrescar» or changed week from a button
/// that never comes back. Why ten seconds: a week is one window, which EventKit
/// reads within its five-second read lifetime plus paging, and ten is the wait
/// this client already uses for every other daemon confirmation. It is not a
/// limit on the read: the daemon keeps reading, and the week appears as soon
/// as it is there.
const READ_WAIT: Duration = Duration::from_secs(10);
const MAX_PROFILE_NAME_CHARS: usize = 80;
const MAX_PROFILE_NAME_BYTES: usize = MAX_PROFILE_NAME_CHARS * 4;

/// Where "Lo que Orbit hizo hoy" and "Escrito por Orbit" start reading: local
/// midnight or twenty-four hours back, whichever is earlier. A local day is
/// 23, 24 or 25 hours long, so a fixed day back missed the start of the day on
/// which the clocks go back.
#[derive(Clone)]
pub struct DaemonClient {
    inner: Arc<ClientInner>,
}

struct ClientInner {
    paths: Paths,
    daemon: LocalClient,
    snapshots: watch::Sender<AppSnapshot>,
    calendar: Arc<Mutex<CalendarCoordinator>>,
    calendar_io: Arc<Mutex<()>>,
    /// One note read at a time, and the turns of the last note whose
    /// transcript was complete (`note_read::SettledTurns`).
    note_io: Mutex<Option<note_read::SettledTurns>>,
    /// One list read at a time, holding the notes it saw being written
    /// (`projection::notes::follow_writing`), and how many pages of it are
    /// shown.
    list_io: Mutex<std::collections::BTreeSet<String>>,
    notes_pages: AtomicUsize,
    /// One library read at a time.
    library_io: Mutex<()>,
    note_target: Mutex<(String, u64)>,
    frames: Arc<StdMutex<FramePublisher>>,
    subscription_abort: AbortHandle,
    maintenance_abort: AbortHandle,
    note_refresh_abort: StdMutex<Option<AbortHandle>>,
    /// The task that reads the agenda again after local midnight.
    calendar_day_abort: StdMutex<Option<AbortHandle>>,
    topics: Arc<RwLock<crate::topics::TopicCache>>,
}

struct Channels {
    snapshots: watch::Sender<AppSnapshot>,
    clock: Arc<dyn CalendarClock>,
    topics: Arc<RwLock<crate::topics::TopicCache>>,
    frames: Arc<StdMutex<FramePublisher>>,
}

impl Drop for ClientInner {
    fn drop(&mut self) {
        self.subscription_abort.abort();
        self.maintenance_abort.abort();
        if let Ok(mut task) = self.calendar_day_abort.lock()
            && let Some(task) = task.take()
        {
            task.abort();
        }
        if let Ok(mut task) = self.note_refresh_abort.lock()
            && let Some(task) = task.take()
        {
            task.abort();
        }
    }
}

mod publish;
mod send;
use publish::*;
mod subscription;

mod agenda;
mod agents;
mod library;
mod note_read;
mod notes;
mod series;
mod sessions;
use agenda::*;
mod local;
use local::*;

impl DaemonClient {
    pub fn connect(paths: Paths, daemon: LocalClient) -> Self {
        Self::connect_with_clock(paths, Arc::new(SystemCalendarClock::default()), daemon)
    }

    pub fn connect_with_clock(
        paths: Paths,
        clock: Arc<dyn CalendarClock>,
        daemon: LocalClient,
    ) -> Self {
        let initial_clock = LocalClock::read(clock.as_ref());
        let coordinator = CalendarCoordinator::new(Arc::clone(&clock));
        let now_ms = initial_clock.now_ms;
        let (start_ms, end_ms) = coordinator.read_window();
        let mut initial = AppSnapshot {
            briefing_clock_ms: now_ms,
            ..AppSnapshot::default()
        };
        initial.calendar.window.start_ms = start_ms;
        initial.calendar.window.end_ms = end_ms;
        initial.calendar.range = coordinator.range();
        match &initial_clock.timezone_name {
            Ok(name) => {
                initial.system_timezone.clone_from(name);
                initial.system_timezone_error = None;
            }
            Err(cause) => {
                initial.system_timezone.clear();
                initial.system_timezone_error = Some(cause.clone());
            }
        }
        apply_profile_name(&mut initial, load_profile_name(&paths));
        let frames = Arc::new(StdMutex::new(FramePublisher::with_clock(
            initial.clone(),
            &initial_clock,
        )));
        let (snapshots, _) = watch::channel(initial);
        let calendar = Arc::new(Mutex::new(coordinator));
        let calendar_io = Arc::new(Mutex::new(()));
        let topics = Arc::new(RwLock::new(crate::topics::TopicCache::default()));
        let channels = Channels {
            snapshots: snapshots.clone(),
            clock: Arc::clone(&clock),
            topics: Arc::clone(&topics),
            frames: Arc::clone(&frames),
        };
        let (subscription_abort, maintenance_abort) =
            subscription::spawn_all(daemon.clone(), &channels);
        let client = Self {
            inner: Arc::new(ClientInner {
                paths,
                daemon,
                snapshots,
                calendar,
                calendar_io,
                note_io: Mutex::new(None),
                list_io: Mutex::new(std::collections::BTreeSet::new()),
                notes_pages: AtomicUsize::new(1),
                library_io: Mutex::new(()),
                note_target: Mutex::new((String::new(), 0)),
                frames,
                subscription_abort,
                maintenance_abort,
                note_refresh_abort: StdMutex::new(None),
                calendar_day_abort: StdMutex::new(None),
                topics,
            }),
        };
        let weak = Arc::downgrade(&client.inner);
        let mut changes = client.watch_frames();
        let task = tokio::spawn(async move {
            // This client belongs to one daemon lifetime. Read the list and
            // library again only when their published heads change.
            let mut seen: Option<(u64, Option<String>)> = None;
            let mut seen_library = None;
            let mut seen_series: Option<(Option<series::SeriesTarget>, u64)> = None;
            while changes.changed().await.is_ok() {
                changes.borrow_and_update();
                let Some(inner) = weak.upgrade() else { break };
                let head = {
                    let topics = inner.topics.read().await;
                    match topics
                        .topic(orbit_core::topics::TopicId::Notes)
                        .map(|cell| (&cell.knowledge, &cell.freshness))
                    {
                        Some((
                            orbit_core::topics::TopicData::Notes(
                                orbit_core::topics::Knowledge::Present { value, .. },
                            ),
                            crate::topics::TopicFreshness::Current,
                        )) => Some(value.clone()),
                        _ => None,
                    }
                };
                let Some(head) = head else { continue };
                let client = DaemonClient { inner };
                let list = (head.revision, head.recording.clone());
                if seen.as_ref() != Some(&list) {
                    seen = Some(list);
                    // The read stores its cause in `notes.index`.
                    if let Err(error) = client.refresh_notes().await {
                        drop(error);
                    }
                }
                if seen_library != Some(head.library_revision) {
                    seen_library = Some(head.library_revision);
                    client.read_library().await;
                }
                // Hoy's next meeting: its series is read again when the
                // meeting changes or any recording does (a task marked done).
                let target = client.next_series().await;
                let series = (target.clone(), head.revision);
                if seen_series.as_ref() != Some(&series) {
                    seen_series = Some(series);
                    client.read_series_previous(target).await;
                }
                // An open note is read again when the hub says it
                // changed (`recording.changed`).
                let open = client.local_snapshot().notes.and_then(|notes| {
                    let read = notes.open_read?;
                    (notes.open.is_some() && read.state == crate::NoteReadState::Available)
                        .then_some((read.recording_id, read.revision))
                });
                if let Some((recording_id, revision)) = open
                    && head.changed_at(&recording_id) > revision
                {
                    // The read stores its cause in `open_read`.
                    if let Err(error) = client.reread_open_note(recording_id).await {
                        drop(error);
                    }
                }
            }
        });
        if let Ok(mut slot) = client.inner.note_refresh_abort.lock() {
            *slot = Some(task.abort_handle());
        }
        let day_task = tokio::spawn(subscription::calendar_day_loop(
            Arc::downgrade(&client.inner),
            Arc::clone(&clock),
        ));
        if let Ok(mut slot) = client.inner.calendar_day_abort.lock() {
            *slot = Some(day_task.abort_handle());
        }
        client
    }

    fn local_snapshot(&self) -> AppSnapshot {
        self.inner.snapshots.borrow().clone()
    }

    pub fn update_version(&self) -> Option<String> {
        self.inner
            .snapshots
            .borrow()
            .settings
            .update
            .version
            .clone()
    }

    pub fn watch_frames(&self) -> watch::Receiver<()> {
        let frames = match self.inner.frames.lock() {
            Ok(frames) => frames,
            Err(poisoned) => poisoned.into_inner(),
        };
        frames.watch_frames()
    }

    pub fn bootstrap(&self) -> crate::AppBootstrap {
        let mut frames = match self.inner.frames.lock() {
            Ok(frames) => frames,
            Err(poisoned) => poisoned.into_inner(),
        };
        frames.observe(&self.inner.snapshots.borrow());
        let projected_frames = frames.bootstrap_frames();
        crate::AppBootstrap {
            revision: frames.revision(),
            frames: projected_frames,
        }
    }

    /// A local compatibility view for callers still using the in-process
    /// snapshot model. The IPC bootstrap contains only function frames.
    pub fn composed_snapshot(&self) -> AppSnapshot {
        let bootstrap = self.bootstrap();
        let mut snapshot = AppSnapshot::default();
        for frame in &bootstrap.frames {
            frame.apply(&mut snapshot);
        }
        snapshot.revision = bootstrap.revision.counter;
        snapshot
    }

    pub fn take_frames(&self) -> Vec<crate::features::FeatureFrame> {
        let mut frames = match self.inner.frames.lock() {
            Ok(frames) => frames,
            Err(poisoned) => poisoned.into_inner(),
        };
        frames.observe(&self.inner.snapshots.borrow());
        frames.drain()
    }
}

#[cfg(test)]
mod bootstrap_tests {
    use super::*;
    use crate::features::FeatureId;

    #[tokio::test]
    async fn bootstrap_wire_contains_only_revision_and_function_frames()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let paths = Paths::resolve(Some(directory.path().to_path_buf()))?;
        let client = DaemonClient::connect(paths, crate::client_fixture::idle());
        let bootstrap = client.bootstrap();
        let wire = serde_json::to_value(&bootstrap)?;
        assert!(wire.get("snapshot").is_none());
        assert!(wire.get("revision").is_some());
        assert_eq!(
            wire["frames"].as_array().map(Vec::len),
            Some(FeatureId::ALL.len())
        );
        let mut composed = AppSnapshot::default();
        for frame in &bootstrap.frames {
            frame.apply(&mut composed);
        }
        assert_eq!(client.composed_snapshot(), composed);
        Ok(())
    }

    #[tokio::test]
    async fn local_write_notifies_frame_watchers_without_snapshot_subscription()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let paths = Paths::resolve(Some(directory.path().to_path_buf()))?;
        let client = DaemonClient::connect(paths, crate::client_fixture::idle());
        client.take_frames();
        let mut changes = client.watch_frames();
        client.set_login_item(LoginItemState::On);
        tokio::time::timeout(Duration::from_secs(1), changes.changed()).await??;
        assert!(client.take_frames().iter().any(
            |frame| matches!(frame, crate::features::FeatureFrame::Local { view, .. }
                if view.login_item == LoginItemState::On)
        ));
        Ok(())
    }
}
