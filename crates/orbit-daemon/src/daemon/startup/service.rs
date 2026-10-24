//! The local IPC served while the store recovers: the status, every other
//! order refused as recovering, and the user's stop admitted and written
//! before its reply.
use super::*;
use orbit_core::{
    privacy_stop::UserStop,
    refusal::{Refusal, Resource, recovery_step_name as step_name},
};

// At most one IPC channel's 32 calls may wait for disk after admission. This
// protects memory if the disk stalls; it is not a deadline for a privacy stop.
const PENDING_STOP_CAPACITY: usize = 32;

pub(super) struct RecoveryService<'a> {
    pub(super) diagnostics: crate::diagnostics::DiagnosticLog,
    pub(super) state: State,
    pub(super) calls: &'a mut mpsc::Receiver<Call>,
    pub(super) signals: &'a mut shutdown::Signals,
    pub(super) listener: &'a mut Option<JoinHandle<Result<()>>>,
    pub(super) progress: watch::Receiver<RecoveryStep>,
    pub(super) started: Instant,
    pub(super) interrupted_by: Option<&'static str>,
    pub(super) stop_store: StopStore,
    pub(super) pending_stops: VecDeque<Call>,
    pub(super) stop_running: bool,
    pub(super) stop_result_tx: mpsc::Sender<(Call, Result<()>)>,
    pub(super) stop_result_rx: mpsc::Receiver<(Call, Result<()>)>,
}

impl<'a> RecoveryService<'a> {
    pub(super) fn new(
        checkpoint: RecoveryCheckpoint,
        stop_store: StopStore,
        io: RecoveryIo<'a>,
        progress: watch::Receiver<RecoveryStep>,
    ) -> Self {
        let RecoveryCheckpoint { mut state, started } = checkpoint;
        state.views.recovery = RecoveryStatus::Recovering {
            step: *progress.borrow(),
            elapsed_ms: 0,
        };
        // One write at a time: its receipt is consumed before the next write.
        // This bounds receipts, never the time allowed for a privacy stop.
        let (stop_result_tx, stop_result_rx) = mpsc::channel(1);
        Self {
            diagnostics: crate::diagnostics::DiagnosticLog::in_root(&io.log_root),
            state,
            calls: io.calls,
            signals: io.signals,
            listener: io.listener,
            progress,
            started,
            interrupted_by: None,
            stop_store,
            pending_stops: VecDeque::new(),
            stop_running: false,
            stop_result_tx,
            stop_result_rx,
        }
    }

    fn elapsed_ms(&self) -> u64 {
        let millis = self.started.elapsed().as_millis();
        millis.min(u128::from(u64::MAX)) as u64
    }

    fn status(&self) -> DaemonStatus {
        let mut status = DaemonStatus::from(&self.state);
        if let RecoveryStatus::Recovering { step, elapsed_ms } = &mut status.recovery {
            *step = *self.progress.borrow();
            *elapsed_ms = self.elapsed_ms();
        }
        status
    }

    fn reply(&mut self, call: Call) -> Result<()> {
        let command = &call.request.body;
        let status = matches!(command, Command::Status);
        if matches!(command, Command::PrivacyStop) {
            if self.pending_stops.len() == PENDING_STOP_CAPACITY {
                let refusal = Refusal::TooMany {
                    resource: Resource::IpcRequest,
                    count: self.pending_stops.len() as u64,
                    max: PENDING_STOP_CAPACITY as u64,
                };
                super::record_ipc_reply_loss(
                    call.reply.send(super::command_response(
                        call.request.id,
                        Some(self.status()),
                        Some(refusal),
                        None,
                        BTreeMap::new(),
                    )),
                    command,
                    Some(&self.diagnostics),
                );
                return Ok(());
            }
            let mut decision = decide(&self.state, Event::PrivacyStop, now())?;
            decision.state.set_notice(UserStop::Order.notice());
            self.state = decision.state;

            self.pending_stops.push_back(call);
            self.start_next_stop();
            return Ok(());
        }
        let accepted = status;
        let step = *self.progress.borrow();
        if !matches!(
            &self.state.views.recovery,
            RecoveryStatus::Recovering { .. }
        ) {
            return Err(StartupError::MissingRecoveryStep {
                step: step_name(step),
                elapsed_ms: self.elapsed_ms(),
            }
            .into());
        }
        let refusal = (!accepted).then(|| Refusal::Recovering {
            step,
            elapsed_ms: self.elapsed_ms(),
        });
        if matches!(command, Command::VoiceNoteAudio { .. }) {
            super::record_ipc_reply_loss(
                call.reply.send(VoiceNoteAudioResponse {
                    v: 1,
                    id: call.request.id,
                    ok: false,
                    error: refusal.as_ref().map(ToString::to_string),
                    audio: None,
                    refusal,
                }),
                command,
                Some(&self.diagnostics),
            );
        } else {
            super::record_ipc_reply_loss(
                call.reply.send(super::command_response(
                    call.request.id,
                    Some(self.status()),
                    refusal,
                    None,
                    BTreeMap::new(),
                )),
                command,
                Some(&self.diagnostics),
            );
        }
        Ok(())
    }

    fn start_next_stop(&mut self) {
        if self.stop_running {
            return;
        }
        let Some(call) = self.pending_stops.pop_front() else {
            return;
        };
        let writer = self.stop_store.clone();
        let state = self.state.clone();
        let results = self.stop_result_tx.clone();
        self.stop_running = true;
        tokio::spawn(async move {
            let result = writer.persist(state).await;
            if results.send((call, result)).await.is_err() {
                eprintln!("stop_result_receiver_closed");
            }
        });
    }

    fn finish_stop(&mut self, call: Call, result: Result<()>) {
        let refusal = result.err().map(|cause| Refusal::StorageFailed {
            operation: orbit_core::refusal::Operation::PrivacyStop,
            cause: Some(orbit_core::refusal::StorageCause::PrivacyStop {
                error: cause.to_string(),
                step: *self.progress.borrow(),
                elapsed_ms: self.elapsed_ms(),
            }),
        });
        super::record_ipc_reply_loss(
            call.reply.send(super::command_response(
                call.request.id,
                Some(self.status()),
                refusal,
                None,
                BTreeMap::new(),
            )),
            &call.request.body,
            Some(&self.diagnostics),
        );
        self.stop_running = false;
        self.start_next_stop();
    }

    pub(super) async fn serve_future<T, F>(&mut self, future: F) -> Result<T>
    where
        F: std::future::Future<Output = Result<T>>,
    {
        tokio::pin!(future);
        let mut finished = None;
        loop {
            if !self.stop_running
                && self.pending_stops.is_empty()
                && let Some(output) = finished.take()
            {
                let value = output?;
                if let Some(signal) = self.interrupted_by {
                    return Err(StartupError::Interrupted {
                        signal,
                        step: step_name(*self.progress.borrow()),
                        elapsed_ms: self.elapsed_ms(),
                    }
                    .into());
                }
                return Ok(value);
            }
            tokio::select! {
                output = &mut future, if finished.is_none() => finished = Some(output),
                Some(call) = self.calls.recv() => self.reply(call)?,
                Some((call, result)) = self.stop_result_rx.recv(), if self.stop_running => {
                    self.finish_stop(call, result);
                }
                changed = self.progress.changed() => {
                    if changed.is_ok() {
                        let _ = self.progress.borrow_and_update();

                    }
                }
                signal = self.signals.next(), if self.interrupted_by.is_none() => {
                    self.interrupted_by = Some(signal);
                }
                result = async {
                    match self.listener.as_mut() {
                        Some(listener) => listener.await,
                        None => std::future::pending().await,
                    }
                } => {
                    let step = step_name(*self.progress.borrow());
                    let elapsed_ms = self.elapsed_ms();
                    return Err(match result {
                        Ok(Ok(())) => StartupError::IpcEndedDuringRecovery { step, elapsed_ms },
                        Ok(Err(source)) => StartupError::IpcFailedDuringRecovery { step, elapsed_ms, source },
                        Err(source) => StartupError::IpcTaskFailedDuringRecovery { step, elapsed_ms, source },
                    }.into());
                }
            }
        }
    }
}

pub(super) async fn save_report_snapshot<T, Save, Saved>(
    service: &mut RecoveryService<'_>,
    mut owner: T,
    mut written: DurableState,
    snapshot_gate: Arc<tokio::sync::Mutex<()>>,
    mut save: Save,
) -> Result<(T, DurableState)>
where
    Save: FnMut(T, DurableState) -> Saved,
    Saved: std::future::Future<Output = Result<T>>,
{
    let mut initial_write_owed = true;
    while initial_write_owed || service.state.durable() != written {
        // The gate is held only by the actual write. An IPC stop waits for
        // this older snapshot to finish, then saves its newer state before
        // receiving an affirmative reply.
        let guard = loop {
            if let Ok(guard) = snapshot_gate.clone().try_lock_owned() {
                break guard;
            }
            // The wait must release its guard before `serve_future` sees a
            // completed future: it may still be serving a stop that needs the
            // same gate. Retry against the current state after that stop.
            service
                .serve_future(async {
                    drop(snapshot_gate.clone().lock_owned().await);
                    Ok(())
                })
                .await?;
        };
        if let RecoveryStatus::Recovering { step, .. } = &mut service.state.views.recovery {
            *step = RecoveryStep::Snapshot;
        }

        let snapshot = service.state.durable();
        let write = save(owner, snapshot.clone());
        owner = service
            .serve_future(async move {
                let result = write.await;
                drop(guard);
                result
            })
            .await?;
        written = snapshot;
        initial_write_owed = false;
    }
    Ok((owner, written))
}
