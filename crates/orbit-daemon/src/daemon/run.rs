//! The daemon's life in order: its start and recovery, the loop, its closure.
use super::*;

pub async fn run(paths: Paths, development: bool) -> Result<()> {
    run_with_helper(paths, development, None).await
}

/// The whole daemon, until its closure ends. Orbit.app runs it on a thread of
/// its own (`apps/desktop/src-tauri/src/daemon_thread.rs`); the
/// development binary `orbit` runs it on its main thread.
pub async fn run_with_helper(
    paths: Paths,
    development: bool,
    helper_override: Option<std::path::PathBuf>,
) -> Result<()> {
    // The startup future measured 113,328 bytes on the caller's stack. Keep
    // that future on the heap so ordinary 2 MiB test threads can run it.
    Box::pin(run_with_helper_inner(
        paths,
        development,
        helper_override,
        None,
    ))
    .await
}

/// The embedded daemon exposes its existing call queue and topic watches.
/// The thread owner sends its final result after dropping the runtime; a
/// dropped end sender reports a panic instead of inventing a successful quit.
pub async fn run_embedded(
    paths: Paths,
    development: bool,
    helper_override: Option<std::path::PathBuf>,
    connected: tokio::sync::oneshot::Sender<ipc::LocalClient>,
    ended: watch::Receiver<Option<ipc::DaemonEnded>>,
) -> Result<()> {
    Box::pin(run_with_helper_inner(
        paths,
        development,
        helper_override,
        Some((connected, ended)),
    ))
    .await
}

type LocalEnds = (
    tokio::sync::oneshot::Sender<ipc::LocalClient>,
    watch::Receiver<Option<ipc::DaemonEnded>>,
);

/// What the process holds from its first line to its last: the signals, the
/// instance lock, the order queue, the topics and the IPC listener.
pub(super) struct Process {
    pub(super) signals: shutdown::Signals,
    _instance: std::fs::File,
    pub(super) calls: mpsc::Receiver<Call>,
    pub(super) topic_board: topics::TopicBoard,
    pub(super) listener: Option<JoinHandle<Result<()>>>,
    listener_cleanup: Option<startup::ListenerCleanup>,
}

impl Process {
    fn recovery_io(&mut self, log_root: std::path::PathBuf) -> startup::RecoveryIo<'_> {
        startup::RecoveryIo {
            log_root,
            calls: &mut self.calls,
            signals: &mut self.signals,
            listener: &mut self.listener,
        }
    }
}

/// The channels between the loop and everything that feeds it.
pub(super) struct Channels {
    pub(super) work_rx: mpsc::Receiver<WorkResult>,
    pub(super) native_commands: Option<mpsc::Receiver<Value>>,
    pub(super) native_route: Option<crate::native::NativeRoute>,
    /// The capture task's end of its channel, until the task takes it.
    pub(super) audio_rx: Option<mpsc::Receiver<crate::native::RoutedAudio>>,
    /// The capture task's own store reader, until the task takes it.
    pub(super) capture_reader: Option<orbit_store::ReadOnlyStore>,
    pub(super) native_events: mpsc::Receiver<Value>,
    pub(super) hub_rx: mpsc::Receiver<(u64, HubEvent)>,
    pub(super) notes_rx: mpsc::Receiver<recording_notes::NoteEvent>,
}

/// The saved state as the start found it, before recovery.
struct Saved {
    state: State,
    restart_interrupted_live_capture: bool,
    restart_recording: Option<orbit_core::Recording>,
    reported_sequence: u64,
}

pub(super) async fn run_with_helper_inner(
    paths: Paths,
    development: bool,
    helper_override: Option<std::path::PathBuf>,
    local: Option<LocalEnds>,
) -> Result<()> {
    if helper_override
        .as_ref()
        .is_some_and(|path| !path.is_absolute())
    {
        bail!("El helper debe tener una ruta absoluta");
    }
    // Before anything starts, so SIGTERM is never the default kill.
    // Process-wide: inside the app they are the app's too, and the app
    // installs none of its own, so a SIGTERM to Orbit closes the daemon in
    // order and the app follows it.
    let signals = shutdown::Signals::install()?;
    paths.initialize()?;
    let instance = ipc::lock_instance(&paths)?;
    let socket_enabled = local.is_none() || orbit_core::orbit_bundle::LOCAL_SOCKET;
    if !socket_enabled {
        ipc::remove_owned_socket(&paths)?;
    }
    if socket_enabled && tokio::net::UnixStream::connect(&paths.socket).await.is_ok() {
        bail!("Ya hay un servicio Orbit activo");
    }
    let settings = paths.load_settings()?;
    // Before anything else runs, so a panic from here on is reported if the
    // user allowed it.
    let error_reports = crate::error_reports::ErrorReports::start(
        crate::error_reports::DSN,
        settings.error_reports,
    );
    error_reports.report_panics();
    let key = storage_key(&paths, development)?;
    let store = store_actor::StoreActor::open_bootstrap(&paths.store, key)?;
    let saved = load_saved(&store, &settings)?;
    let mut process = open_process(&paths, signals, instance, socket_enabled, local).await?;
    let boot = runtime_build::Boot {
        paths,
        development,
        helper_override,
        settings,
        error_reports,
        key,
    };
    let (mut runtime, channels, helper_path) =
        Box::pin(start_runtime(boot, store, saved, &mut process)).await?;
    if let Some(cleanup) = &mut process.listener_cleanup {
        cleanup.disarm();
    }
    Box::pin(main_loop::serve(
        &mut runtime,
        &mut process,
        channels,
        helper_path,
    ))
    .await
}

fn load_saved(store: &Store, settings: &Settings) -> Result<Saved> {
    let saved_state = store.load_snapshot::<DurableState>()?;
    let restart_interrupted_live_capture = saved_state
        .as_ref()
        .and_then(|saved| saved.capture.recording.as_ref())
        .is_some_and(|recording| recording.phase.interrupted_on_recovery());
    let restart_recording = saved_state
        .as_ref()
        .and_then(|saved| saved.capture.recording.clone());
    let startup_op = orbit_ipc::new_op();
    let reported_sequence = saved_state
        .as_ref()
        .map(|state| state.notice_log.last_sequence())
        .unwrap_or(0);
    let mut state = saved_state
        .map(|mut state| {
            state.notice_log.begin_turn(startup_op.clone());
            state.recovered()
        })
        .map(State::from)
        .unwrap_or_default();
    state.durable.notice_log.begin_turn(startup_op);
    state.views.account = Some(account_summary(settings));
    Ok(Saved {
        state,
        restart_interrupted_live_capture,
        restart_recording,
        reported_sequence,
    })
}

/// The order queue, the topics and the IPC listener, bound before recovery
/// so a closing order or a signal can reach a start that is still recovering.
async fn open_process(
    paths: &Paths,
    signals: shutdown::Signals,
    instance: std::fs::File,
    socket_enabled: bool,
    local: Option<LocalEnds>,
) -> Result<Process> {
    let (calls_tx, calls) = mpsc::channel::<Call>(32);
    let topic_board = topics::TopicBoard::new(uuid::Uuid::new_v4().to_string());
    let topic_channels = topic_board.channels();
    let mut listener = None;
    let mut listener_cleanup = None;
    if socket_enabled {
        let (task, cleanup) =
            bind_listener(paths, calls_tx.clone(), topic_channels.clone()).await?;
        listener_cleanup = Some(cleanup);
        listener = Some(task);
    }
    if let Some((connected, ended)) = local {
        let _ = connected.send(ipc::LocalClient::new(calls_tx, &topic_channels, ended));
    }
    Ok(Process {
        signals,
        _instance: instance,
        calls,
        topic_board,
        listener,
        listener_cleanup,
    })
}

async fn bind_listener(
    paths: &Paths,
    calls: mpsc::Sender<Call>,
    topics: orbit_ipc::TopicChannels,
) -> Result<(JoinHandle<Result<()>>, startup::ListenerCleanup)> {
    let listener_paths = paths.clone();
    let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
    let listener_started = std::time::Instant::now();
    let task =
        tokio::spawn(
            async move { ipc::listen(&listener_paths, calls, topics, Some(ready_tx)).await },
        );
    let mut cleanup = startup::ListenerCleanup::new(&task, paths.socket.clone());
    if ready_rx.await.is_err() {
        let elapsed_ms = listener_started
            .elapsed()
            .as_millis()
            .min(u128::from(u64::MAX)) as u64;
        return Err(match task.await {
            Ok(Err(source)) => startup::StartupError::IpcBind { elapsed_ms, source },
            Ok(Ok(())) => startup::StartupError::IpcEndedBeforeBind { elapsed_ms },
            Err(source) => startup::StartupError::IpcTaskEndedBeforeBind { elapsed_ms, source },
        }
        .into());
    }
    cleanup.socket_owned();
    Ok((task, cleanup))
}

/// The store recovered, the writer started, the environment prepared and the
/// runtime built and recovered: everything before the helper is launched.
async fn start_runtime(
    mut boot: runtime_build::Boot,
    store: Store,
    saved: Saved,
    process: &mut Process,
) -> Result<(Runtime, Channels, Option<std::path::PathBuf>)> {
    let snapshot_gate = std::sync::Arc::new(tokio::sync::Mutex::new(()));
    let (worker_done, _) = tokio::sync::watch::channel(false);
    let direct_stop = startup::StopStore::Direct {
        path: boot.paths.store.clone(),
        key: boot.key,
        snapshot_gate: snapshot_gate.clone(),
        worker_done,
    };
    let Saved {
        state,
        restart_interrupted_live_capture,
        restart_recording,
        reported_sequence,
    } = saved;
    let mut io = process.recovery_io(boot.paths.root.clone());
    let (store, state, started, persisted_state) = startup::recover(
        store,
        state,
        direct_stop,
        snapshot_gate.clone(),
        io.reborrow(),
    )
    .await?;
    let (work_events, work_rx) = mpsc::channel(WORK_EVENTS_CAPACITY);
    let supervision = crate::supervision::Supervision::new(Some(
        crate::diagnostics::DiagnosticLog::in_root(&boot.paths.root),
    ));
    let store_actor = store_actor::StoreActor::spawn_with_diagnostics(
        store,
        work_events.clone(),
        crate::diagnostics::DiagnosticLog::in_root(&boot.paths.root),
        supervision.clone(),
    )?;
    let stop_store = startup::StopStore::Actor(store_actor.startup_stop_writer()?);
    let written = Written {
        store_actor: &store_actor,
        persisted_state,
        snapshot_gate,
    };
    let checkpoint = startup::RecoveryCheckpoint { state, started };
    let (checkpoint, prepared) = Box::pin(prepare(
        &mut boot,
        checkpoint,
        &stop_store,
        &mut io,
        written,
    ))
    .await?;
    let recovered = runtime_build::Recovered {
        store_actor,
        supervision,
        checkpoint: &checkpoint,
        prepared,
        work_events,
        work_rx,
        restart_recording: restart_recording.as_ref(),
    };
    let (mut runtime, channels, helper_path) = runtime_build::build_runtime(boot, recovered)?;
    // Restored records were already reported; recovery notices are new entries.
    runtime.turn.reported_sequence = reported_sequence;
    runtime.begin_fresh_turn(orbit_core::notice::ReportSource::Daemon);
    let live = restart_interrupted_live_capture;
    Box::pin(recover_runtime(
        &mut runtime,
        checkpoint,
        stop_store,
        &mut io,
        live,
    ))
    .await?;
    Ok((runtime, channels, helper_path))
}

/// What the writer already holds of the recovery, for its runtime snapshot.
struct Written<'a> {
    store_actor: &'a store_actor::StoreActor,
    persisted_state: DurableState,
    snapshot_gate: std::sync::Arc<tokio::sync::Mutex<()>>,
}

/// The environment (helper, credentials, diagnostics) and the runtime's own
/// recovery snapshot, each a served step a closing order can still reach.
async fn prepare(
    boot: &mut runtime_build::Boot,
    checkpoint: startup::RecoveryCheckpoint,
    stop_store: &startup::StopStore,
    io: &mut startup::RecoveryIo<'_>,
    written: Written<'_>,
) -> Result<(startup::RecoveryCheckpoint, startup::PreparedEnvironment)> {
    let preparation = tokio::task::spawn_blocking({
        let paths = boot.paths.clone();
        let development = boot.development;
        let helper_override = boot.helper_override.take();
        move || startup::prepare_environment(paths, development, helper_override)
    });
    let (checkpoint, prepared) = startup::serve_step(
        checkpoint,
        stop_store.clone(),
        io.reborrow(),
        orbit_core::RecoveryStep::Environment,
        async {
            preparation
                .await
                .context("Terminó la preparación del servicio")?
        },
    )
    .await?;
    let checkpoint = startup::persist_runtime_recovery(
        checkpoint,
        stop_store.clone(),
        io.reborrow(),
        written.store_actor,
        written.persisted_state,
        written.snapshot_gate,
    )
    .await?;
    Ok((checkpoint, prepared))
}

/// The recordings, calls, notes, call preference and folders as the saved
/// state left them; a privacy stop the person asked for, again; and the
/// state durable before the helper or the hub hear of it.
async fn recover_runtime(
    runtime: &mut Runtime,
    checkpoint: startup::RecoveryCheckpoint,
    stop_store: startup::StopStore,
    io: &mut startup::RecoveryIo<'_>,
    restart_interrupted_live_capture: bool,
) -> Result<()> {
    let (runtime_progress_tx, runtime_progress) =
        tokio::sync::watch::channel(orbit_core::RecoveryStep::Operations);
    let _progress_sender = runtime_progress_tx.clone();
    let runtime_recovery = async {
        runtime
            .recover_recording_at_start(orbit_core::RecordingInterruption {
                cause: if restart_interrupted_live_capture {
                    orbit_core::RecordingInterruptionCause::Restart
                } else {
                    orbit_core::RecordingInterruptionCause::CauseNotSaved
                },
                detail: None,
                audio_available: false,
            })
            .await?;
        runtime.recover_call_closures(now()).await?;
        runtime.recover_recording_notes()?;
        runtime_progress_tx.send_replace(orbit_core::RecoveryStep::CallPreference);
        runtime.recover_call_preference();
        runtime.recover_assistant_folders();
        Ok(())
    };
    let (checkpoint, ()) = startup::serve_runtime_recovery(
        checkpoint,
        stop_store.clone(),
        io.reborrow(),
        runtime_progress,
        runtime_recovery,
    )
    .await?;
    if startup::needs_runtime_stop(&checkpoint.state, &runtime.state) {
        let (_after_stop, ()) = startup::serve_step(
            checkpoint,
            stop_store,
            io.reborrow(),
            orbit_core::RecoveryStep::Snapshot,
            async {
                runtime
                    .privacy_stop(orbit_core::privacy_stop::UserStop::Order)
                    .await;
                Ok(())
            },
        )
        .await?;
    }
    runtime
        .store_actor
        .critical_snapshot(&runtime.state)
        .await?;
    runtime.store_actor.finish_startup().await?;
    runtime.state.views.recovery = orbit_core::RecoveryStatus::Ready;
    Ok(())
}
