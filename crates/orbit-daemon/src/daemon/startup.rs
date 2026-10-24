//! Serve local IPC while the owner store completes its startup reconciliation.
use super::*;
use orbit_core::{RecoveryStatus, RecoveryStep};
use std::{collections::VecDeque, sync::Arc, time::Instant};
use tokio::sync::watch;

mod service;
mod stop_store;

use service::{RecoveryService, save_report_snapshot};
pub(super) use stop_store::StopStore;

#[derive(Debug, thiserror::Error)]
pub(super) enum StartupError {
    #[error("Recuperación sin paso: último paso {step}, {elapsed_ms} ms transcurridos")]
    MissingRecoveryStep { step: &'static str, elapsed_ms: u64 },
    #[error(
        "Arranque interrumpido por {signal} durante la recuperación: paso {step}, {elapsed_ms} ms transcurridos"
    )]
    Interrupted {
        signal: &'static str,
        step: &'static str,
        elapsed_ms: u64,
    },
    #[error(
        "El servidor IPC terminó durante la recuperación: paso {step}, {elapsed_ms} ms transcurridos"
    )]
    IpcEndedDuringRecovery { step: &'static str, elapsed_ms: u64 },
    #[error(
        "El servidor IPC falló durante la recuperación: paso {step}, {elapsed_ms} ms transcurridos; causa: {source:#}"
    )]
    IpcFailedDuringRecovery {
        step: &'static str,
        elapsed_ms: u64,
        source: anyhow::Error,
    },
    #[error(
        "El trabajador IPC terminó durante la recuperación: paso {step}, {elapsed_ms} ms transcurridos; causa: {source}"
    )]
    IpcTaskFailedDuringRecovery {
        step: &'static str,
        elapsed_ms: u64,
        source: tokio::task::JoinError,
    },
    #[error("El socket IPC no se abrió tras {elapsed_ms} ms: {source:#}")]
    IpcBind {
        elapsed_ms: u64,
        source: anyhow::Error,
    },
    #[error("El socket IPC no se abrió tras {elapsed_ms} ms: el servidor terminó antes de bind")]
    IpcEndedBeforeBind { elapsed_ms: u64 },
    #[error("El socket IPC no se abrió tras {elapsed_ms} ms: terminó el trabajador: {source}")]
    IpcTaskEndedBeforeBind {
        elapsed_ms: u64,
        source: tokio::task::JoinError,
    },
    #[error("El actor de almacenamiento se detuvo antes de admitir la parada de privacidad")]
    StopWriterStopped,
    #[error(
        "El candado de la cola de paradas de privacidad quedó envenenado; la parada no se admitió"
    )]
    StopWriterLockPoisoned,
}

pub(super) struct ListenerCleanup {
    abort: tokio::task::AbortHandle,
    socket: std::path::PathBuf,
    armed: bool,
    socket_owned: bool,
}

impl ListenerCleanup {
    pub(super) fn new(listener: &JoinHandle<Result<()>>, socket: std::path::PathBuf) -> Self {
        Self {
            abort: listener.abort_handle(),
            socket,
            armed: true,
            socket_owned: false,
        }
    }

    pub(super) fn socket_owned(&mut self) {
        self.socket_owned = true;
    }

    pub(super) fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for ListenerCleanup {
    fn drop(&mut self) {
        if self.armed {
            self.abort.abort();
            if self.socket_owned {
                let _ = std::fs::remove_file(&self.socket);
            }
        }
    }
}

pub(super) struct PreparedEnvironment {
    pub helper_path: Option<std::path::PathBuf>,
    pub token: Option<String>,
    pub diagnostics: crate::diagnostics::DiagnosticLog,
}

pub(super) struct RecoveryCheckpoint {
    pub state: State,
    pub started: Instant,
}

pub(super) struct RecoveryIo<'a> {
    pub log_root: std::path::PathBuf,
    pub calls: &'a mut mpsc::Receiver<Call>,
    pub signals: &'a mut shutdown::Signals,
    pub listener: &'a mut Option<JoinHandle<Result<()>>>,
}

impl RecoveryIo<'_> {
    pub(super) fn reborrow(&mut self) -> RecoveryIo<'_> {
        RecoveryIo {
            log_root: self.log_root.clone(),
            calls: self.calls,
            signals: self.signals,
            listener: self.listener,
        }
    }
}

pub(super) fn needs_runtime_stop(checkpoint: &State, runtime: &State) -> bool {
    checkpoint.privacy_stopped()
        && checkpoint.privacy_stop_by() == Some(orbit_core::privacy_stop::StopRequester::User)
        && (!runtime.privacy_stopped()
            || runtime.privacy_stop_by() != Some(orbit_core::privacy_stop::StopRequester::User))
}

pub(super) fn prepare_environment(
    paths: Paths,
    development: bool,
    helper_override: Option<std::path::PathBuf>,
) -> Result<PreparedEnvironment> {
    let helper_path = helper_override;
    let token = get_secret(&paths, "device", development)?;
    // This diagnostic is durable before the first connection attempt.
    let diagnostics = crate::diagnostics::DiagnosticLog::in_root(&paths.root);
    diagnostics.record(
        "daemon_starting",
        &[(
            "native",
            if helper_path.is_some() {
                "true"
            } else {
                "false"
            },
        )],
    );
    Ok(PreparedEnvironment {
        helper_path,
        token,
        diagnostics,
    })
}

pub(super) async fn recover(
    store: Store,
    mut state: State,
    stop_store: StopStore,
    snapshot_gate: Arc<tokio::sync::Mutex<()>>,
    io: RecoveryIo<'_>,
) -> Result<(Store, State, Instant, DurableState)> {
    let started = Instant::now();
    state.views.recovery = RecoveryStatus::Recovering {
        step: RecoveryStep::Chunks,
        elapsed_ms: 0,
    };
    let initial_snapshot = state.durable();
    let (progress_tx, progress) = watch::channel(RecoveryStep::Chunks);
    let mut service = RecoveryService::new(
        RecoveryCheckpoint { state, started },
        stop_store,
        io,
        progress,
    );
    let worker_done = match &service.stop_store {
        StopStore::Direct { worker_done, .. } => Some(worker_done.clone()),
        StopStore::Actor(_) => None,
    };
    let worker = spawn_store_recovery(store, progress_tx.clone(), worker_done);
    let (store, report) = service
        .serve_future(async {
            worker
                .await
                .context("Terminó el trabajador de recuperación")?
        })
        .await?;
    say_store_recovery(&mut service.state, &report);
    let (store, written) = save_report_snapshot(
        &mut service,
        store,
        initial_snapshot,
        snapshot_gate,
        |mut store, snapshot| async move {
            tokio::task::spawn_blocking(move || -> Result<Store> {
                store.save_snapshot(&snapshot)?;
                Ok(store)
            })
            .await
            .context("Terminó el guardado final")?
        },
    )
    .await?;
    Ok((store, service.state, started, written))
}

/// The store's own recovery, off the loop: its chunks, then its upload
/// jobs, each step said on `progress`. `worker_done` tells an early stop's
/// direct write that the store is free again.
fn spawn_store_recovery(
    store: Store,
    progress: watch::Sender<RecoveryStep>,
    worker_done: Option<watch::Sender<bool>>,
) -> JoinHandle<Result<(Store, orbit_store::RecoveryReport)>> {
    let runtime = tokio::runtime::Handle::current();
    tokio::task::spawn_blocking(move || -> Result<_> {
        let mut store = store;
        let result = (|| {
            let report = runtime.block_on(recover_store(&mut store))?;
            progress.send_replace(RecoveryStep::UploadJobs);
            store.recover_upload_jobs()?;
            progress.send_replace(RecoveryStep::Snapshot);
            Ok((store, report))
        })();
        if let Some(done) = worker_done {
            done.send_replace(true);
        }
        result
    })
}

/// What the store's recovery found, said when it found anything: files
/// recovered, or lost.
fn say_store_recovery(state: &mut State, report: &orbit_store::RecoveryReport) {
    if report.recovered > 0
        || report.invalid > 0
        || report.missing > 0
        || report.incomplete_temporary > 0
    {
        use orbit_core::notice::{Notice, NoticeCode, NoticeKey};
        let lost = report.invalid + report.missing + report.incomplete_temporary > 0;
        state.set_notice(
            Notice::new(if lost {
                NoticeCode::StoreRecoveryLostFiles
            } else {
                NoticeCode::StoreRecovered
            })
            .with(NoticeKey::Recovered, report.recovered)
            .with(NoticeKey::Invalid, report.invalid)
            .with(NoticeKey::Missing, report.missing)
            .with(NoticeKey::Incomplete, report.incomplete_temporary),
        );
    }
}

pub(super) async fn serve_runtime_recovery<T, F>(
    checkpoint: RecoveryCheckpoint,
    stop_store: StopStore,
    io: RecoveryIo<'_>,
    progress: watch::Receiver<RecoveryStep>,
    recovery: F,
) -> Result<(RecoveryCheckpoint, T)>
where
    F: std::future::Future<Output = Result<T>>,
{
    let started = checkpoint.started;
    let mut service = RecoveryService::new(checkpoint, stop_store, io, progress);

    let result = service.serve_future(recovery).await?;
    Ok((
        RecoveryCheckpoint {
            state: service.state,
            started,
        },
        result,
    ))
}

/// Fixed phases keep their progress sender alive until the work and stop
/// receipts finish; the operations phase publishes its own changing steps.
pub(super) async fn serve_step<T>(
    checkpoint: RecoveryCheckpoint,
    stop_store: StopStore,
    io: RecoveryIo<'_>,
    step: RecoveryStep,
    recovery: impl std::future::Future<Output = Result<T>>,
) -> Result<(RecoveryCheckpoint, T)> {
    let (_progress_sender, progress) = watch::channel(step);
    serve_runtime_recovery(checkpoint, stop_store, io, progress, recovery).await
}

/// A stop during environment preparation may have changed the durable state.
/// Use the same save-until-current loop as the store recovery, while serving
/// IPC and waiting for every admitted stop's receipt.
pub(super) async fn persist_runtime_recovery(
    checkpoint: RecoveryCheckpoint,
    stop_store: StopStore,
    io: RecoveryIo<'_>,
    actor: &store_actor::StoreActor,
    written: DurableState,
    snapshot_gate: Arc<tokio::sync::Mutex<()>>,
) -> Result<RecoveryCheckpoint> {
    if checkpoint.state.durable() == written {
        return Ok(checkpoint);
    }
    let started = checkpoint.started;
    let (_progress_sender, progress) = watch::channel(RecoveryStep::Snapshot);
    let mut service = RecoveryService::new(checkpoint, stop_store, io, progress);
    save_report_snapshot(
        &mut service,
        (),
        written,
        snapshot_gate,
        |(), snapshot| async move { actor.critical_snapshot(&State::from(snapshot)).await },
    )
    .await?;
    Ok(RecoveryCheckpoint {
        state: service.state,
        started,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use orbit_core::privacy_stop::StopRequester;
    use std::io::{Read, Write};

    #[test]
    fn a_user_stop_during_runtime_recovery_reattributes_an_old_stop() {
        let mut checkpoint = State::default();
        crate::daemon::authority_fixture::set(
            &mut checkpoint,
            "privacy_stopped",
            serde_json::json!(true),
        );
        crate::daemon::authority_fixture::set(
            &mut checkpoint,
            "privacy_stop_by",
            serde_json::json!(StopRequester::User),
        );
        let mut runtime = State::default();
        crate::daemon::authority_fixture::set(
            &mut runtime,
            "privacy_stopped",
            serde_json::json!(true),
        );
        assert!(needs_runtime_stop(&checkpoint, &runtime));
    }

    // Forwarding the stop before notifying makes the following Status a FIFO
    // barrier: it must observe that stop, without repeatedly polling the socket.
    fn observe_stop(
        calls: mpsc::Sender<Call>,
    ) -> (mpsc::Sender<Call>, tokio::sync::oneshot::Receiver<()>) {
        let (sender, mut receiver) = mpsc::channel::<Call>(32);
        let (seen, event) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            let mut seen = Some(seen);
            while let Some(call) = receiver.recv().await {
                let stop = matches!(call.request.body, Command::PrivacyStop);
                if calls.send(call).await.is_err() {
                    break;
                }
                if stop && let Some(seen) = seen.take() {
                    let _ = seen.send(());
                }
            }
        });
        (sender, event)
    }

    #[tokio::test(flavor = "current_thread")]
    async fn stop_during_final_snapshot_write_is_on_disk_before_acknowledgement() -> Result<()> {
        tokio::task::LocalSet::new()
            .run_until(async {
                let directory = tempfile::tempdir()?;
                let paths = Paths::resolve(Some(directory.path().join("orbit")))?;
                paths.initialize()?;
                let key = [0x42; 32];
                let mut store = Store::open(&paths.store, key)?;
                let written = State::default().durable();
                store.save_snapshot(&written)?;
                let mut state = State::default();
                state.views.recovery = RecoveryStatus::Recovering {
                    step: RecoveryStep::Snapshot,
                    elapsed_ms: 0,
                };
                state.set_notice(orbit_core::notice::NoticeCode::StoreRecoveryLostFiles);
                let (calls_tx, mut calls) = mpsc::channel(32);
                let (calls_tx, stop_forwarded) = observe_stop(calls_tx);
                let listener_paths = paths.clone();
                let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
                let listener = tokio::spawn(async move {
                    ipc::listen(
                        &listener_paths,
                        calls_tx,
                        ipc::TopicChannels::new(),
                        Some(ready_tx),
                    )
                    .await
                });
                ready_rx.await.context("fixture socket did not bind")?;
                let mut signals = shutdown::Signals::install()?;
                let gate = paths.root.join("final-save-gate");
                let status = std::process::Command::new("mkfifo")
                    .arg("-m")
                    .arg("600")
                    .arg(&gate)
                    .status()?;
                if !status.success() {
                    bail!("final save gate failed: {status}");
                }
                let (started_tx, started_rx) = tokio::sync::oneshot::channel();
                let (progress_tx, progress) = watch::channel(RecoveryStep::Snapshot);
                let _progress_tx = progress_tx;
                let writer_gate = gate.clone();
                let snapshot_gate = Arc::new(tokio::sync::Mutex::new(()));
                let (worker_done, _) = watch::channel(true);
                let stop_store = StopStore::Direct {
                    path: paths.store.clone(),
                    key,
                    snapshot_gate: snapshot_gate.clone(),
                    worker_done,
                };
                let diagnostic_root = paths.root.clone();
                let mut listener = Some(listener);
                let save = tokio::task::spawn_local(async move {
                    let (stop_result_tx, stop_result_rx) = mpsc::channel(1);
                    let mut service = RecoveryService {
                        diagnostics: crate::diagnostics::DiagnosticLog::in_root(&diagnostic_root),
                        state,
                        calls: &mut calls,
                        signals: &mut signals,
                        listener: &mut listener,
                        progress,
                        started: Instant::now(),
                        interrupted_by: None,
                        stop_store,
                        pending_stops: VecDeque::new(),
                        stop_running: false,
                        stop_result_tx,
                        stop_result_rx,
                    };

                    let mut started_tx = Some(started_tx);
                    let result = save_report_snapshot(
                        &mut service,
                        store,
                        written,
                        snapshot_gate,
                        |mut store, snapshot| {
                            let gate = writer_gate.clone();
                            let first = started_tx.take();
                            async move {
                                tokio::task::spawn_blocking(move || -> Result<Store> {
                                    if let Some(started_tx) = first {
                                        let _ = started_tx.send(());
                                        let mut fifo =
                                            std::fs::OpenOptions::new().read(true).open(gate)?;
                                        let mut byte = [0];
                                        fifo.read_exact(&mut byte)?;
                                    }
                                    store.save_snapshot(&snapshot)?;
                                    Ok(store)
                                })
                                .await
                                .context("fixture final save worker ended")?
                            }
                        },
                    )
                    .await;
                    drop(service);
                    if let Some(listener) = listener {
                        listener.abort();
                        let _ = listener.await;
                    }
                    result
                });
                started_rx.await.context("final save never began")?;
                let stop_paths = paths.clone();
                let stop = tokio::task::spawn_local(async move {
                    ipc::request(&stop_paths, Command::PrivacyStop).await
                });
                // The existing jam guard bounds a lost event, not convergence.
                tokio::time::timeout(Duration::from_secs(12), stop_forwarded).await??;
                let response = ipc::request(&paths, Command::Status).await?;
                assert!(
                    response
                        .status
                        .context("status missing startup facts")?
                        .privacy_stopped
                );
                let mut writer = std::fs::OpenOptions::new().write(true).open(&gate)?;
                writer.write_all(b"go")?;
                let response = stop.await??;
                let (store, _) = save.await??;
                drop(store);
                let saved = Store::open_existing(&paths.store, key)?
                    .load_snapshot::<DurableState>()?
                    .context("snapshot absent after final save")?;
                assert!(
                    response.ok,
                    "stop was not acknowledged: {:?}",
                    response.error
                );
                assert!(
                    saved.recovered().privacy_stopped(),
                    "acknowledged stop disappeared from the final snapshot"
                );
                Ok(())
            })
            .await
    }

    #[tokio::test(flavor = "current_thread")]
    async fn queued_stop_cannot_hold_the_final_snapshot_gate() -> Result<()> {
        tokio::task::LocalSet::new()
            .run_until(async {
                for iteration in 0..8 {
                    let directory = tempfile::tempdir()?;
                    let paths = Paths::resolve(Some(directory.path().join("orbit")))?;
                    paths.initialize()?;
                    let key = [0x42; 32];
                    Store::open(&paths.store, key)?;
                    let mut state = State::default();
                    state.views.recovery = RecoveryStatus::Recovering {
                            step: RecoveryStep::Snapshot,
                            elapsed_ms: 0,
                        };
                    let (calls_tx, mut calls) = mpsc::channel(32);
                    let (calls_tx, stop_forwarded) = observe_stop(calls_tx);
                    let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
                    let listener_paths = paths.clone();
                        let listener = tokio::spawn(async move {
                        ipc::listen(
                            &listener_paths, calls_tx, ipc::TopicChannels::new(), Some(ready_tx),
                        )
                        .await
                    });
                    let mut cleanup = ListenerCleanup::new(&listener, paths.socket.clone());
                    ready_rx.await.context("fixture socket did not bind")?;
                    cleanup.socket_owned();
                    let gate = Arc::new(tokio::sync::Mutex::new(()));
                    let blocker = gate.clone().lock_owned().await;
                    let (worker_done, _) = watch::channel(true);
                    let stop_store = StopStore::Direct {
                        path: paths.store.clone(),
                        key,
                        snapshot_gate: gate.clone(),
                        worker_done,
                    };
                    let store_path = paths.store.clone();
                    let diagnostic_root = paths.root.clone();
                    let mut listener = Some(listener);
                    let mut save = tokio::task::spawn_local(async move {
                        let mut signals = shutdown::Signals::install()?;
                        let (progress_tx, progress) = watch::channel(RecoveryStep::Snapshot);
                        let _progress_tx = progress_tx;
                        let (stop_result_tx, stop_result_rx) = mpsc::channel(1);
                        let mut service = RecoveryService {
                            diagnostics: crate::diagnostics::DiagnosticLog::in_root(&diagnostic_root),
                            state,
                            calls: &mut calls,
                            signals: &mut signals,
                            listener: &mut listener,
                            progress,
                            started: Instant::now(),
                                                interrupted_by: None,
                            stop_store,
                            pending_stops: VecDeque::new(),
                            stop_running: false,
                            stop_result_tx,
                            stop_result_rx,
                        };

                        let result = save_report_snapshot(
                            &mut service,
                            (),
                            DurableState::default(),
                            gate,
                            move |(), snapshot| {
                                let path = store_path.clone();
                                async move {
                                    tokio::task::spawn_blocking(move || -> Result<()> {
                                        Store::open_existing(&path, key)?.save_snapshot(&snapshot)?;
                                        Ok(())
                                    })
                                    .await
                                    .context("fixture snapshot writer ended")?
                                }
                            },
                        )
                        .await;
                        drop(service);
                        if let Some(listener) = listener {
                            listener.abort();
                            let _ = listener.await;
                        }
                        result
                    });
                    // The current-thread executor polls the save into the
                    // blocked gate before it polls the later IPC stop.
                    tokio::task::yield_now().await;
                    let stop_paths = paths.clone();
                    let mut stop = tokio::task::spawn_local(async move {
                        ipc::request(&stop_paths, Command::PrivacyStop).await
                    });
                    tokio::time::timeout(Duration::from_secs(12), stop_forwarded).await??;
                    let response = ipc::request(&paths, Command::Status).await?;
                    assert!(response.status.context("status missing startup facts")?.privacy_stopped);
                    drop(blocker);
                    // A 12 s jam guard exceeds IPC's existing 10 s receipt
                    // budget and the observed 4.3 s disk worst. It protects
                    // this test from a deadlock, not startup latency.
                    let outcome = tokio::time::timeout(Duration::from_secs(12), async {
                        let reply = (&mut stop).await??;
                        let (_, written) = (&mut save).await??;
                        Ok::<_, anyhow::Error>((reply, written))
                    })
                    .await;
                    let (reply, written) = match outcome {
                        Ok(Ok(value)) => value,
                        Ok(Err(error)) => {
                            save.abort();
                            stop.abort();
                            let _ = save.await;
                            return Err(error).context(format!(
                                "iteration {iteration}: queued stop waited for the final snapshot gate"
                            ));
                        }
                        Err(_) => {
                            save.abort();
                            stop.abort();
                            let _ = save.await;
                            bail!(
                                "iteration {iteration}: queued stop and final snapshot gate waited 12 s"
                            );
                        }
                    };
                    assert!(reply.ok, "iteration {iteration}: stop was refused");
                    assert!(written.privacy_stopped(), "iteration {iteration}: stop was lost");
                    cleanup.disarm();
                    std::fs::remove_file(&paths.socket)?;
                }
                Ok(())
            })
            .await
    }
}
