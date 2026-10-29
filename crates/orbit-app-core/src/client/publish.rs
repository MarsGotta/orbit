use super::*;
use crate::{
    calendar::LocalClock,
    features::{FeatureFrame, FeatureInput, FeatureProjection, FrameRevision},
    topics::TopicCache,
};

static NEXT_EPOCH: AtomicU64 = AtomicU64::new(1);

#[derive(PartialEq, Eq)]
struct ClockKey {
    timezone_name: Result<String, String>,
    offset_ms: i64,
    day: i64,
}

impl From<&LocalClock> for ClockKey {
    fn from(clock: &LocalClock) -> Self {
        Self {
            timezone_name: clock.timezone_name.clone(),
            offset_ms: clock.zone.offset_ms_at(clock.now_ms),
            day: clock.zone.day(clock.now_ms),
        }
    }
}

pub(super) struct FramePublisher {
    revision: FrameRevision,
    last: AppSnapshot,
    projected: FeatureProjection,
    pub(super) last_topics: TopicCache,
    pending: Vec<FeatureFrame>,
    frame_changes: watch::Sender<()>,
    clock_key: Option<ClockKey>,
    clock: LocalClock,
    pub(super) copy: crate::PresentationCopy,
}

impl FramePublisher {
    pub(super) fn new(initial: AppSnapshot) -> Self {
        let (frame_changes, receiver) = watch::channel(());
        drop(receiver);
        let clock = LocalClock::from_snapshot(&initial);
        let revision = FrameRevision {
            epoch: NEXT_EPOCH.fetch_add(1, Ordering::Relaxed),
            counter: 0,
        };
        let last_topics = TopicCache::default();
        let projected = FeatureProjection::new(
            &FeatureInput::with_clock(&initial, &last_topics, clock.clone()),
            revision,
        );
        Self {
            revision,
            projected,
            last: initial,
            copy: crate::PresentationCopy::default(),
            last_topics,
            pending: Vec::new(),
            frame_changes,
            clock_key: None,
            clock,
        }
    }

    pub(super) fn with_clock(initial: AppSnapshot, clock: &LocalClock) -> Self {
        let mut publisher = Self::new(initial);
        publisher.clock_key = Some(clock.into());
        publisher.clock = clock.clone();
        publisher.projected = FeatureProjection::new(
            &FeatureInput::with_clock(&publisher.last, &publisher.last_topics, clock.clone()),
            publisher.revision,
        );
        publisher
    }

    pub(super) fn observe_clock(
        &mut self,
        snapshots: &watch::Sender<AppSnapshot>,
        clock: &LocalClock,
    ) {
        let key = ClockKey::from(clock);
        if self.clock_key.as_ref() == Some(&key) {
            return;
        }
        let mut snapshot = self.last.clone();
        snapshots.send_modify(|current| {
            match &clock.timezone_name {
                Ok(name) => {
                    current.system_timezone.clone_from(name);
                    current.system_timezone_error = None;
                }
                Err(cause) => {
                    current.system_timezone.clear();
                    current.system_timezone_error = Some(cause.clone());
                }
            }
            snapshot.clone_from(current);
        });
        let mut revision = self.revision;
        let mut changed = self.projected.update(
            &FeatureInput::with_clock(&snapshot, &self.last_topics, clock.clone())
                .with_copy(self.copy),
            || {
                revision.counter = revision.counter.saturating_add(1);
                revision
            },
        );
        macro_rules! ensure_clock_frame {
            ($id:ident) => {
                if !changed
                    .iter()
                    .any(|frame| frame.feature() == crate::features::FeatureId::$id)
                    && let Some(frame) = self
                        .projected
                        .frames()
                        .iter()
                        .find(|frame| frame.feature() == crate::features::FeatureId::$id)
                {
                    revision.counter = revision.counter.saturating_add(1);
                    let mut frame = frame.clone();
                    frame.set_revision(revision);
                    changed.push(frame);
                }
            };
        }
        ensure_clock_frame!(Status);
        ensure_clock_frame!(Agenda);
        ensure_clock_frame!(Notes);
        ensure_clock_frame!(Briefing);
        self.queue(changed);
        self.revision = revision;
        self.last = snapshot;
        self.clock = clock.clone();
        self.clock_key = Some(key);
    }

    pub(super) fn observe(&mut self, snapshot: &AppSnapshot) {
        let mut revision = self.revision;
        let changed = self.projected.update(
            &FeatureInput::with_clock(snapshot, &self.last_topics, self.clock.clone())
                .with_copy(self.copy),
            || {
                revision.counter = revision.counter.saturating_add(1);
                revision
            },
        );
        self.queue(changed);
        self.revision = revision;
        if self.last != *snapshot {
            self.last.clone_from(snapshot);
        }
    }

    pub(super) fn observe_topics(&mut self, snapshot: &AppSnapshot, topics: &TopicCache) {
        let mut revision = self.revision;
        let changed = self.projected.update(
            &FeatureInput::with_clock(snapshot, topics, self.clock.clone()).with_copy(self.copy),
            || {
                revision.counter = revision.counter.saturating_add(1);
                revision
            },
        );
        self.queue(changed);
        self.revision = revision;
        if self.last != *snapshot {
            self.last.clone_from(snapshot);
        }
        // Topic payloads and keyed reads are shared; retain just their heads
        // for the next dependency comparison and command receipt.
        self.last_topics.clone_from(topics);
        self.frame_changes.send_replace(());
    }

    fn queue(&mut self, changed: Vec<FeatureFrame>) {
        if changed.is_empty() {
            return;
        }
        for frame in changed {
            // The queue protects the Tauri reader when bootstrap polls without
            // draining. It holds one latest frame per registered feature, not
            // a history or a limit on state changes.
            if let Some(previous) = self
                .pending
                .iter_mut()
                .find(|old| old.feature() == frame.feature())
            {
                *previous = frame;
            } else {
                self.pending.push(frame);
            }
        }
        self.frame_changes.send_replace(());
    }

    pub(super) fn set_copy(&mut self, copy: crate::PresentationCopy) {
        self.copy = copy;
        self.revision.counter = self.revision.counter.saturating_add(1);
        self.projected = FeatureProjection::new(
            &FeatureInput::with_clock(&self.last, &self.last_topics, self.clock.clone())
                .with_copy(copy),
            self.revision,
        );
        let frames = self.bootstrap_frames();
        self.queue(frames);
    }

    pub(super) fn watch_frames(&self) -> watch::Receiver<()> {
        self.frame_changes.subscribe()
    }

    pub(super) fn bootstrap_frames(&self) -> Vec<FeatureFrame> {
        self.projected
            .frames()
            .iter()
            .cloned()
            .map(|mut frame| {
                frame.set_revision(self.revision);
                frame
            })
            .collect()
    }

    pub(super) fn revision(&self) -> FrameRevision {
        self.revision
    }

    pub(super) fn drain(&mut self) -> Vec<FeatureFrame> {
        std::mem::take(&mut self.pending)
    }
}

pub(super) fn publish_clock_tick(
    snapshots: &watch::Sender<AppSnapshot>,
    frames: &StdMutex<FramePublisher>,
    clock: &dyn CalendarClock,
) {
    let sample = LocalClock::read(clock);
    let mut publisher = match frames.lock() {
        Ok(publisher) => publisher,
        Err(poisoned) => poisoned.into_inner(),
    };
    publisher.observe_clock(snapshots, &sample);
}

#[cfg(test)]
mod frame_tests {
    use super::*;

    #[test]
    fn review_clock_tick_preserves_a_concurrent_local_write() {
        let mut initial = crate::AppSnapshot::empty(1, crate::DaemonConnection::Ready);
        initial.settings.profile = Some(crate::ProfileSettingsSnapshot {
            name: Some("M".repeat(2 * 1024 * 1024)),
        });
        for attempt in 0..8 {
            let first = sample_clock("UTC", 0, 1_789_511_400_000 + attempt * 60_000);
            let next = sample_clock("Europe/Madrid", 3_600_000, first.now_ms);
            let (snapshots, _) = watch::channel(initial.clone());
            let writer = snapshots.clone();
            let barrier = std::sync::Barrier::new(2);
            std::thread::scope(|scope| {
                let mut publisher = FramePublisher::with_clock(initial.clone(), &first);
                let barrier_ref = &barrier;
                let snapshots_ref = &snapshots;
                let tick = scope.spawn(move || {
                    barrier_ref.wait();
                    publisher.observe_clock(snapshots_ref, &next);
                });
                barrier.wait();
                std::thread::sleep(Duration::from_micros(50));
                writer.send_modify(|snapshot| snapshot.settings.onboarding_done = true);
                assert!(tick.join().is_ok());
            });
            assert!(
                snapshots.borrow().settings.onboarding_done,
                "attempt {attempt} lost a local write"
            );
            assert_eq!(snapshots.borrow().system_timezone, "Europe/Madrid");
        }
    }

    #[test]
    fn topic_projection_cannot_erase_a_completed_note_read() {
        let mut local = crate::AppSnapshot::default();
        let completed = crate::NoteReadSnapshot {
            recording_id: "11111111-1111-4111-8111-111111111111".into(),
            revision: 7,
            state: crate::NoteReadState::Available,
            detail: None,
            read_at_ms: Some(1_000),
        };
        if let Some(notes) = local.notes.as_mut() {
            notes.open_read = Some(completed.clone());
        }
        let mut publisher = FramePublisher::new(local.clone());
        let mut topics = TopicCache::default();
        topics.observe_at(status_topic(2, None), Some(2));
        publisher.observe_topics(&local, &topics);
        for frame in publisher.drain() {
            frame.apply(&mut local);
        }
        assert_eq!(
            local.notes.and_then(|notes| notes.open_read),
            Some(completed)
        );
    }

    #[test]
    fn recording_topic_change_drains_a_real_feature_frame() -> Result<(), Box<dyn std::error::Error>>
    {
        let initial = crate::AppSnapshot::empty(1, crate::DaemonConnection::Ready);
        let mut publisher = FramePublisher::new(initial.clone());
        // The subscription is already ready, so this recording frame is not
        // also the Connecting→Ready flip that every daemon-owned view reads.
        let mut baseline = TopicCache::default();
        baseline.ready_without_topics();
        publisher.observe_topics(&initial, &baseline);
        publisher.drain();
        let frame: orbit_core::topics::TopicFrame = serde_json::from_value(serde_json::json!({
            "epoch": "synthetic", "revision": 1,
            "data": {"topic": "recording", "knowledge": {
                "state": "present", "as_of_ms": null,
                "value": {
                    "recording": {
                        "id": "recording-1", "title": "Prueba",
                        "started_at_ms": 10, "ended_at_ms": null,
                        "expires_at_ms": 60_010, "application_pid": null,
                        "audio": "system", "phase": "starting",
                        "sealed_chunks": 0, "gaps": 0, "tracks": [],
                        "finalizing_since_ms": null
                    },
                    "recording_signal": null,
                    "pending_jobs": 0,
                    "offline": false,
                    "applications": {"state": "unknown", "why": "not_observed_yet"}
                }
            }}
        }))?;
        let mut topics = TopicCache::default();
        topics.observe_at(frame, Some(50));
        let wake = publisher.watch_frames();
        publisher.observe_topics(&initial, &topics);
        assert!(wake.has_changed()?);
        let frames = publisher.drain();
        assert_eq!(frames.len(), 3);
        assert_eq!(frames[0].feature().as_str(), "recording");
        assert_eq!(frames[1].feature().as_str(), "briefing");
        let crate::features::FeatureFrame::Local { view: local, .. } = &frames[2] else {
            return Err(std::io::Error::other("missing update installation state").into());
        };
        assert!(local.update.install_blocked.is_some());
        let crate::features::FeatureFrame::Recording { view, .. } = &frames[0] else {
            return Err(std::io::Error::other("wrong feature frame").into());
        };
        assert_eq!(view.meeting.phase, crate::MeetingPhase::Starting);
        let mut composed = initial;
        frames[0].apply(&mut composed);
        assert_eq!(composed.meeting.phase, crate::MeetingPhase::Starting);
        Ok(())
    }

    #[test]
    fn topic_receipt_reaches_feature_input_without_changing_today_views() {
        let initial = crate::AppSnapshot::empty(1, crate::DaemonConnection::Ready);
        let mut publisher = FramePublisher::new(initial.clone());
        let mut ready = TopicCache::default();
        ready.ready_without_topics();
        publisher.observe_topics(&initial, &ready);
        publisher.drain();
        let mut topics = ready;
        topics.observe_at(
            orbit_core::topics::TopicFrame {
                epoch: "synthetic".into(),
                revision: 1,
                data: orbit_core::topics::TopicData::not_observed(
                    orbit_core::topics::TopicId::Status,
                ),
            },
            Some(1_000),
        );
        publisher.observe_topics(&initial, &topics);
        assert_eq!(
            publisher
                .last_topics
                .topic(orbit_core::topics::TopicId::Status)
                .map(|topic| topic.revision),
            Some(1)
        );
        let frames = publisher.drain();
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].feature(), crate::features::FeatureId::Status);
        let crate::features::FeatureFrame::Status { view, .. } = &frames[0] else {
            panic!("status receipt did not project status");
        };
        assert!(matches!(
            view.availability,
            crate::features::FeatureAvailability::Unknown { .. }
        ));
        publisher.observe_topics(&initial, &topics);
        assert!(publisher.drain().is_empty());
    }

    #[test]
    fn quiet_minute_creates_no_revision() {
        let initial = crate::AppSnapshot::empty(1, crate::DaemonConnection::Ready);
        let mut clock = LocalClock::from_snapshot(&initial);
        let mut publisher = FramePublisher::with_clock(initial.clone(), &clock);
        let (snapshots, _) = watch::channel(initial);
        clock.now_ms += 60_000;
        publisher.observe_clock(&snapshots, &clock);
        assert_eq!(publisher.revision().counter, 0);
        assert!(publisher.drain().is_empty());
    }

    #[test]
    fn local_update_emits_only_local_frame_with_monotonic_revision() {
        let initial = crate::AppSnapshot::empty(1, crate::DaemonConnection::Ready);
        let mut publisher = FramePublisher::new(initial.clone());
        let mut updated = initial.clone();
        updated.settings.profile = Some(crate::ProfileSettingsSnapshot {
            name: Some("Alina".into()),
        });
        publisher.observe(&updated);
        let frames = publisher.drain();
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].feature(), crate::features::FeatureId::Briefing);
        assert_eq!(frames[1].feature(), crate::features::FeatureId::Local);
        assert_eq!(frames[0].revision().counter, 1);
        assert_eq!(frames[1].revision().counter, 2);
        let mut composed = initial;
        for frame in &frames {
            frame.apply(&mut composed);
        }
        assert_eq!(composed.settings.profile, updated.settings.profile);
        assert_eq!(
            composed
                .briefing
                .as_ref()
                .and_then(|briefing| briefing.greeting.name.as_deref()),
            Some("Alina")
        );
        assert!(publisher.drain().is_empty());
    }

    #[test]
    fn status_frame_carries_settings_hub_change() {
        let initial = crate::AppSnapshot::empty(1, crate::DaemonConnection::Ready);
        let mut publisher = FramePublisher::new(initial.clone());
        let mut unpaired = TopicCache::default();
        unpaired.observe_at(status_topic(1, None), Some(1));
        publisher.observe_topics(&initial, &unpaired);
        publisher.drain();
        let mut paired = unpaired.clone();
        paired.observe_at(status_topic(2, Some("abcd")), Some(2));
        publisher.observe_topics(&initial, &paired);
        let frames = publisher.drain();
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].feature(), crate::features::FeatureId::Status);
        let mut composed = initial;
        frames[0].apply(&mut composed);
        assert!(composed.settings.hub.paired);
        assert_eq!(
            composed.settings.hub.device_id_short.as_deref(),
            Some("abcd")
        );
    }

    fn status_topic(
        revision: u64,
        device_id_short: Option<&str>,
    ) -> orbit_core::topics::TopicFrame {
        let account = device_id_short.map(|id| {
            serde_json::json!({
                "device_id_short": id
            })
        });
        orbit_core::topics::TopicFrame {
            epoch: "synthetic".into(),
            revision,
            data: serde_json::from_value(serde_json::json!({
                "topic": "status",
                "knowledge": {
                    "state": "present",
                    "as_of_ms": null,
                    "value": {
                        "hub": "online",
                        "account": account,
                        "privacy_stopped": false,
                        "helper_connected": false
                    }
                }
            }))
            .unwrap_or_else(|error| panic!("synthetic status topic: {error}")),
        }
    }

    #[test]
    fn pending_frames_keep_only_latest_view_per_function() {
        let initial = crate::AppSnapshot::empty(1, crate::DaemonConnection::Ready);
        let mut publisher = FramePublisher::new(initial.clone());
        let mut updated = initial.clone();
        for value in 1..=100 {
            updated.settings.profile = Some(crate::ProfileSettingsSnapshot {
                name: Some(value.to_string()),
            });
            publisher.observe(&updated);
        }
        let frames = publisher.drain();
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].feature(), crate::features::FeatureId::Briefing);
        assert_eq!(frames[1].feature(), crate::features::FeatureId::Local);
        assert_eq!(frames[1].revision().counter, 200);
        let mut composed = initial;
        for frame in &frames {
            frame.apply(&mut composed);
        }
        assert_eq!(composed.settings.profile, updated.settings.profile);
        assert_eq!(
            composed
                .briefing
                .as_ref()
                .and_then(|briefing| briefing.greeting.name.as_deref()),
            Some("100")
        );
    }

    fn sample_clock(name: &str, offset_ms: i64, now_ms: i64) -> LocalClock {
        LocalClock {
            now_ms,
            zone: crate::local_time::LocalZone::fixed(offset_ms),
            timezone_name: Ok(name.into()),
        }
    }

    #[test]
    fn quiet_tick_publishes_nothing_and_offset_or_day_publishes_status() {
        let initial = crate::AppSnapshot::empty(1, crate::DaemonConnection::Ready);
        let now_ms = 1_789_511_400_000;
        let first = sample_clock("Europe/Madrid", 2 * 3_600_000, now_ms);
        let mut publisher = FramePublisher::with_clock(initial.clone(), &first);
        let (snapshots, _) = watch::channel(initial);
        publisher.observe_clock(&snapshots, &first);
        assert!(
            publisher.drain().is_empty(),
            "rereading the same zone, offset and local day is not a new frame"
        );
        publisher.observe_clock(
            &snapshots,
            &sample_clock("America/New_York", -4 * 3_600_000, now_ms),
        );
        let offset_frames = publisher.drain();
        assert_eq!(offset_frames.len(), 4);
        for feature in [
            crate::features::FeatureId::Status,
            crate::features::FeatureId::Agenda,
            crate::features::FeatureId::Notes,
            crate::features::FeatureId::Briefing,
        ] {
            assert!(offset_frames.iter().any(|frame| frame.feature() == feature));
        }
        assert!(offset_frames.iter().any(|frame| matches!(frame,
            FeatureFrame::Status { view, .. } if view.system_timezone == "America/New_York")));
        publisher.observe_clock(
            &snapshots,
            &sample_clock(
                "America/New_York",
                -4 * 3_600_000,
                now_ms + 24 * 60 * 60 * 1_000,
            ),
        );
        let day_frames = publisher.drain();
        assert_eq!(day_frames.len(), 4);
        for feature in [
            crate::features::FeatureId::Status,
            crate::features::FeatureId::Agenda,
            crate::features::FeatureId::Notes,
            crate::features::FeatureId::Briefing,
        ] {
            assert!(day_frames.iter().any(|frame| frame.feature() == feature));
        }
    }

    #[test]
    fn daylight_saving_offset_change_keeps_zone_name_and_republishes_four_views() {
        let initial = crate::AppSnapshot::empty(1, crate::DaemonConnection::Ready);
        let now_ms = 1_789_511_400_000;
        let first = sample_clock("Europe/Madrid", 0, now_ms);
        let mut publisher = FramePublisher::with_clock(initial.clone(), &first);
        let (snapshots, _) = watch::channel(initial);
        publisher.observe_clock(
            &snapshots,
            &sample_clock("Europe/Madrid", 3_600_000, now_ms),
        );
        let changed = publisher.drain();
        assert_eq!(changed.len(), 4);
        for feature in [
            crate::features::FeatureId::Status,
            crate::features::FeatureId::Agenda,
            crate::features::FeatureId::Notes,
            crate::features::FeatureId::Briefing,
        ] {
            assert!(changed.iter().any(|frame| frame.feature() == feature));
        }
        assert!(changed.iter().any(|frame| matches!(frame,
            FeatureFrame::Status { view, .. } if view.system_timezone == "Europe/Madrid")));
    }
}
