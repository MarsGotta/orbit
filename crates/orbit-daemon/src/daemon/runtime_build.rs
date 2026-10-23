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
    pub(super) supervision: crate::supervision::Supervision,
    pub(super) checkpoint: &'a startup::RecoveryCheckpoint,
    pub(super) prepared: startup::PreparedEnvironment,
    pub(super) work_events: mpsc::Sender<WorkResult>,
    pub(super) work_rx: mpsc::Receiver<WorkResult>,
    pub(super) restart_recording: Option<&'a orbit_core::Recording>,
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
        supervision,
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
        work_events,
        persisted: kernel::Persisted::new(&state),
        transfers: transfers::TransfersHandle::default(),
        state,
        reader,
        turn_watch,
        voice_audio_reader,
        #[cfg(test)]
        fixture_store,
        store_actor,
        supervision,
        diagnostics: Some(diagnostics),
        report_log: Some(report_log),
        error_reports,
        turn: turn::Turn::fresh(orbit_core::notice::ReportSource::Daemon),
        helper: helper::HelperHandle::new(ends.native),
        link: hub_link::LinkHandle::new(ends.hub_events, token),
        assistant: assistant::AssistantHandle::new(),
        capture: capture_handle::CaptureHandle::new(recovery),
        audio: audio::AudioHandle::default(),
        sources: sources::SourcesHandle::default(),
        calendar: calendar::CalendarHandle::default(),
        computer: computer_control::ComputerHandle::default(),
        notes: recording_notes::NotesHandle::new(ends.notes_events),
        calls: call_detection::CallDetectionHandle::default(),
        question: meeting_question::QuestionHandle::default(),
        files: assistant_files::FilesHandle::default(),
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
