//! How a starting daemon builds its runtime from the recovered state.
use super::*;

/// What the start hands the runtime it builds.
pub(super) struct Boot {
    pub(super) paths: Paths,
    pub(super) development: bool,
    pub(super) helper_override: Option<std::path::PathBuf>,
    pub(super) settings: Settings,
    pub(super) error_reports: crate::error_reports::ErrorReports,
    pub(super) key: [u8; 32],
}

/// The recovered pieces the runtime is built from.
pub(super) struct Recovered<'a> {
    pub(super) store_actor: store_actor::StoreActor,
    pub(super) checkpoint: &'a startup::RecoveryCheckpoint,
    pub(super) prepared: startup::PreparedEnvironment,
    pub(super) work_events: mpsc::Sender<WorkResult>,
    pub(super) work_rx: mpsc::Receiver<WorkResult>,
    pub(super) restart_recording: Option<&'a orbit_core::Recording>,
}

impl WorkRuntime {
    fn new(work_events: mpsc::Sender<WorkResult>, durable_snapshot: DurableSnapshot) -> Self {
        Self {
            worker: ticketed::Ticketed::logical(),
            retry_gate: upload_outcome::SharedGate::default(),
            finalize_cursor: None,
            work_events,
            durable_snapshot,
            force_snapshot_once: false,
            finalize_ready: BTreeMap::new(),
        }
    }
}

impl ConversationRuntime {
    fn new() -> Self {
        Self {
            bridge_request: None,
            interaction_response: None,
            interrupted_turns: Default::default(),
            voice_task: ticketed::Ticketed::logical(),
            voice_operation: None,
            speech_task: ticketed::Ticketed::logical(),
            speech_voices: BTreeMap::new(),
            speak_next: false,
            read_aloud: None,
            hub_attachments: None,
            handover_from: None,
            window_attachment: None,
            speech_notice_call_id: None,
            work: None,
        }
    }
}

impl CaptureRuntime {
    pub(super) fn new(recovery: Option<cut_recovery::CutRecovery>) -> Self {
        let lane = capture_lane::CaptureLane::default();
        Self {
            buffer: capture_lane::SplitBuffer::new(lane.clone()),
            pending_seals: capture_lane::SealQueue::new(lane.clone()),
            lane,
            served_control: 0,
            seal_cause: None,
            reserve_exhausted: false,
            seal_retry_at: None,
            audio: audio::AudioLifecycle::default(),
            call_runtime: None,
            mic_stream: None,
            remote_stream: None,
            pending_starts: Vec::new(),
            retired_call_starts: BTreeMap::new(),
            retired_stops: BTreeMap::new(),
            interruption: None,
            recovery,
        }
    }
}

impl FeatureRuntime {
    fn new(notes_events: mpsc::Sender<recording_notes::NoteEvent>) -> Self {
        Self {
            sources: sources::SourceRuntime::default(),
            calendar: calendar::CalendarRuntime::default(),
            computer: computer_control::ComputerRuntime::default(),
            meeting: meeting_runtime::MeetingRuntime::default(),
            call: call_detection::CallRuntime::default(),
            question: meeting_question::QuestionRuntime::default(),
            native_host: native_host::NativeHost::default(),
            notes: recording_notes::NotesRuntime::new(notes_events),
            files: assistant_files::FilesRuntime::default(),
        }
    }
}

/// The runtime, the loop's channels and the helper to launch.
pub(super) fn build_runtime(
    boot: Boot,
    recovered: Recovered<'_>,
) -> Result<(Runtime, run::Channels, Option<std::path::PathBuf>)> {
    let Boot {
        paths,
        development,
        settings,
        error_reports,
        key,
        ..
    } = boot;
    let Recovered {
        mut store_actor,
        checkpoint,
        prepared,
        work_events,
        work_rx,
        restart_recording,
    } = recovered;
    let startup::PreparedEnvironment {
        helper_path,
        token,
        diagnostics,
    } = prepared;
    let (ends, mut channels) = loop_channels(work_rx);
    channels.capture_reader = Some(store_actor.runtime_reader(&paths.store, key)?);
    let mut state = checkpoint.state.clone();
    state.views.recovery = orbit_core::RecoveryStatus::Unknown;
    let durable_snapshot = DurableSnapshot::from(&state);
    #[cfg(test)]
    let fixture_store = test_store::open_existing(&paths.store, key)?;
    let turn_watch = turn_watch::TurnWatch::default();
    store_actor.report_waits_to(turn_watch.clone());
    let reader = timed_reader::TimedReader::new(
        store_actor.runtime_reader(&paths.store, key)?,
        turn_watch.clone(),
    );
    let voice_audio_reader = std::sync::Arc::new(std::sync::Mutex::new(
        store_actor.runtime_reader(&paths.store, key)?,
    ));
    let report_log = orbit_ipc::report::ReportLog::daemon(&paths.root);
    let recovery = cut_recovery::recovery_after_restart(restart_recording, helper_path.is_some());
    let runtime = Runtime {
        closing: false,
        privacy_retirement_owed: false,
        paths,
        settings,
        development,
        state,
        reader,
        turn_watch,
        voice_audio_reader,
        #[cfg(test)]
        fixture_store,
        store_actor,
        diagnostics: Some(diagnostics),
        report_log: Some(report_log),
        error_reports,
        turn: turn::Turn::fresh(orbit_core::notice::ReportSource::Daemon),
        native: ends.native,
        hub: hub_link::HubLink::default(),
        hub_protocol_version: None,
        unrecognized_hub_events: BTreeSet::new(),
        hub_storage_nearly_full: None,
        hub_task: None,
        snapshot_revision: None,
        hub_events: ends.hub_events,
        pending_native: BTreeMap::new(),
        helper_waits: BTreeMap::new(),
        token,
        work: WorkRuntime::new(work_events, durable_snapshot),
        conversation: ConversationRuntime::new(),
        capture: CaptureRuntime::new(recovery),
        features: FeatureRuntime::new(ends.notes_events),
        reads: InFlightReads::default(),
    };
    Ok((runtime, channels, helper_path))
}

/// The runtime's ends of the loop's channels.
struct RuntimeEnds {
    native: mpsc::Sender<Value>,
    hub_events: mpsc::Sender<(u64, HubEvent)>,
    notes_events: mpsc::Sender<recording_notes::NoteEvent>,
}

fn loop_channels(work_rx: mpsc::Receiver<WorkResult>) -> (RuntimeEnds, run::Channels) {
    let (native, native_commands) = mpsc::channel(64);
    // Sized for ten seconds of two-track audio (≈47 ev/s × 2 × 10 = 940,
    // rounded to 1000) so a stalled consumer pays in discarded frames, not
    // in the helper's life.
    let (native_events_tx, native_events) = mpsc::channel(crate::native::NATIVE_EVENTS_CAPACITY);
    // The capture task's own channel, of the same bound: frames never share
    // the loop's channel, and a full one makes the helper keep its backlog.
    let (audio_tx, audio_rx) = mpsc::channel(crate::native::NATIVE_EVENTS_CAPACITY);
    let (hub_events, hub_rx) = mpsc::channel(128);
    let (notes_events, notes_rx) = mpsc::channel(recording_notes::NOTE_EVENTS);
    let ends = RuntimeEnds {
        native,
        hub_events,
        notes_events,
    };
    let channels = run::Channels {
        work_rx,
        native_commands: Some(native_commands),
        native_route: Some(crate::native::NativeRoute::new(native_events_tx, audio_tx)),
        audio_rx: Some(audio_rx),
        capture_reader: None,
        native_events,
        hub_rx,
        notes_rx,
    };
    (ends, channels)
}
