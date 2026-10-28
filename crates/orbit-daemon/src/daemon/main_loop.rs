//! The daemon's one loop: it routes each order, helper event, hub event, work
//! result, note answer and tick to the runtime, one turn at a time, and
//! closes the daemon in order when it ends.
use super::*;

mod orders;
mod workers;

/// How many turns each arm served, for the aggregate logged at closure.
#[derive(Default)]
struct Turns {
    signals: u64,
    commands: u64,
    native: u64,
    hub: u64,
    work: u64,
    notes: u64,
    topic_due: u64,
    tick: u64,
    capture_tick: u64,
}

/// What one arm of the loop asks of it after its turn.
enum After {
    Continue,
    Exit,
}

/// Everything the loop owns between its turns.
struct MainLoop<'a> {
    runtime: &'a mut Runtime,
    process: &'a mut run::Process,
    channels: run::Channels,
    closing: shutdown::Shutdown,
    turns: Turns,
    topics_started: std::time::Instant,
    tick: tokio::time::Interval,
    /// The capture's upkeep on the loop, on a clock of its own.
    capture_tick: tokio::time::Interval,
    last_tick: Duration,
    loop_lag: loop_lag::LoopLag,
}

/// Launches the helper, connects the hub, serves until the closure and then
/// closes everything in order.
pub(super) async fn serve(
    runtime: &mut Runtime,
    process: &mut run::Process,
    mut channels: run::Channels,
    helper_path: Option<std::path::PathBuf>,
) -> Result<()> {
    // Only now, with nothing left that can fail the startup: a helper launched
    // earlier could be left behind by a startup error.
    let mut helper = None;
    // The task runs with or without a helper: chunks a previous run sealed
    // still go up from it.
    let capture = spawn_capture(runtime, &mut channels);
    if helper_path.is_some() {
        let commands = channels.native_commands.take();
        let route = channels.native_route.take();
        if let (Some(commands), Some(route)) = (commands, route) {
            helper = Some(native::Supervisor::spawn(
                helper_path,
                commands,
                route,
                runtime.supervision.clone(),
            ));
        }
    }
    runtime.connect();
    let mut main = MainLoop {
        runtime,
        process,
        channels,
        closing: shutdown::Shutdown::default(),
        turns: Turns::default(),
        topics_started: std::time::Instant::now(),
        tick: tokio::time::interval(Duration::from_secs(1)),
        capture_tick: tokio::time::interval(Duration::from_secs(1)),
        last_tick: Duration::ZERO,
        loop_lag: loop_lag::LoopLag::default(),
    };
    let outcome = Box::pin(main.run()).await;
    main.record_turns();
    Box::pin(main.close(helper, capture)).await;
    outcome
}

/// The capture task, on its own thread, with the lane the runtime shares.
fn spawn_capture(
    runtime: &Runtime,
    channels: &mut run::Channels,
) -> Option<capture_lane::task::CaptureTask> {
    let parts = capture_lane::task::TaskParts {
        lane: runtime.capture.lane.clone(),
        audio: channels.audio_rx.take()?,
        port: runtime.store_actor.capture_port(),
        store: runtime.paths.store.clone(),
        reader: channels.capture_reader.take()?,
        gate: runtime.transfers.retry_gate.clone(),
        root: runtime.paths.root.clone(),
        tick: capture_lane::task::CAPTURE_TICK,
        supervision: runtime.supervision.clone(),
        diagnostics: runtime
            .diagnostics
            .as_ref()
            .map(|_| crate::diagnostics::DiagnosticLog::in_root(&runtime.paths.root)),
    };
    match capture_lane::task::CaptureTask::spawn(parts) {
        Ok(task) => Some(task),
        Err(error) => {
            runtime.log("capture_task_not_started", &[("error", &error.to_string())]);
            None
        }
    }
}

impl MainLoop<'_> {
    async fn run(&mut self) -> Result<()> {
        loop {
            let listener_ended = self
                .process
                .listener
                .as_ref()
                .is_some_and(JoinHandle::is_finished);
            self.closing
                .observe_listener(listener_ended, self.runtime)
                .await;
            if self.closing.ready(self.runtime) {
                break;
            }
            // The bounded budget is checked on every iteration, not only on
            // the tick, so a closure that has been waiting on a stuck vertical
            // still leaves on time.
            if self.closing.exit_now(std::time::Instant::now()) {
                let _ = self.runtime.declare_bounded_exit().await;
                break;
            }
            if let After::Exit = Box::pin(self.turn()).await {
                break;
            }
            Box::pin(self.end_turn()).await;
            // An actor past its budget ends the daemon: the app relaunches it
            // whole, from the store, as after any fall.
            if let Some(spent) = self.runtime.supervision.exhausted() {
                bail!("{}", spent.describe());
            }
            if self.closing.exit_now(std::time::Instant::now()) {
                let _ = self.runtime.declare_bounded_exit().await;
                break;
            }
            if self.closing.ready(self.runtime) {
                break;
            }
        }
        if self.closing.listener_failed {
            bail!(
                "The IPC server stopped; pending audio is already saved. Check the data path and other instances"
            );
        }
        Ok(())
    }

    /// Waits for the next source and serves it.
    async fn turn(&mut self) -> After {
        let topic_due = self.process.topic_board.next_due_ms().and_then(|due_ms| {
            self.topics_started
                .checked_add(Duration::from_millis(due_ms))
                .map(tokio::time::Instant::from_std)
        });
        let lane = self.runtime.capture.lane.clone();
        let served = self.runtime.capture.served_control;
        tokio::select! {
            name = self.process.signals.next() => return Box::pin(self.on_signal(name)).await,
            Some(call) = self.process.calls.recv() => Box::pin(self.on_call(call)).await,
            Some(event) = self.channels.native_events.recv() => Box::pin(self.on_native(event)).await,
            () = lane.forwarded_waiting(served) => Box::pin(self.on_forwarded()).await,
            Some((generation, event)) = self.channels.hub_rx.recv() => {
                Box::pin(self.on_hub(generation, event)).await;
            }
            Some(event) = self.channels.work_rx.recv() => Box::pin(self.on_work(event)).await,
            Some(event) = self.channels.notes_rx.recv() => Box::pin(self.on_note(event)).await,
            _ = async {
                if let Some(due) = topic_due {
                    tokio::time::sleep_until(due).await;
                }
            }, if topic_due.is_some() => {
                self.turns.topic_due += 1;
                self.runtime.turn_watch.begin(turn_watch::TurnSource::topics());
            }
            _ = self.tick.tick() => Box::pin(self.on_tick()).await,
            _ = self.capture_tick.tick() => Box::pin(self.on_capture_tick()).await,
        }
        After::Continue
    }

    fn now_ms(&self) -> u64 {
        u64::try_from(self.topics_started.elapsed().as_millis()).unwrap_or(u64::MAX)
    }

    /// Notices, topics and the turn's own timing, after every turn.
    async fn end_turn(&mut self) {
        self.runtime.reads.tasks.retain(|task| !task.is_finished());
        let watch = self.runtime.turn_watch.clone();
        watch
            .time("capture_lane", Box::pin(self.runtime.settle_capture_lane()))
            .await;
        watch.time_sync("report_notices", || self.runtime.report_notices());
        let now_ms = self.now_ms();
        watch.time_sync("publish", || {
            let source = topics::TopicSource {
                state: &self.runtime.state,
                runtime: Some(&*self.runtime),
                now_ms,
            };
            self.process.topic_board.publish(&source);
        });
        if let Some(slow) = watch.end() {
            // Codes and numbers only: which arm, what it served, its
            // slowest step and wait, and whether a meeting was recording.
            let fields = slow.fields(self.runtime.state.durable.capture.recording.is_some());
            let fields: Vec<(&str, &str)> = fields
                .iter()
                .map(|(name, value)| (*name, value.as_str()))
                .collect();
            self.runtime.log("daemon_turn_slow", &fields);
        }
    }

    /// SIGTERM (launchd at logout, `kill`) and SIGINT (a terminal) are one
    /// request: the complete closure the IPC order runs. A second signal
    /// only means "now".
    async fn on_signal(&mut self, name: &'static str) -> After {
        self.turns.signals += 1;
        self.runtime
            .turn_watch
            .begin(turn_watch::TurnSource::signal());
        self.runtime
            .begin_fresh_turn(orbit_core::notice::ReportSource::Daemon);
        let first = !self.closing.requested;
        self.closing.signal();
        self.runtime.log("shutdown_signal", &[("signal", name)]);
        if self.closing.signals > 1 {
            let cause = shutdown::BoundedExitCause::SecondSignal(name);
            let _ = self.runtime.declare_bounded_exit_for(cause).await;
            return After::Exit;
        }
        if !first {
            return After::Continue;
        }
        let next = self.process.signals.next();
        match self.closing.close_until_exit(self.runtime, next).await {
            shutdown::CloseWait::Completed => After::Continue,
            shutdown::CloseWait::BudgetExpired => {
                let _ = self.runtime.declare_bounded_exit().await;
                After::Exit
            }
            shutdown::CloseWait::SecondSignal(signal) => {
                self.closing.signal();
                self.runtime.log("shutdown_signal", &[("signal", signal)]);
                let cause = shutdown::BoundedExitCause::SecondSignal(signal);
                let _ = self.runtime.declare_bounded_exit_for(cause).await;
                After::Exit
            }
        }
    }

    async fn on_native(&mut self, mut event: Value) {
        self.turns.native += 1;
        let seq = event
            .as_object_mut()
            .and_then(|object| object.remove(native::SEQ_FIELD))
            .and_then(|seq| seq.as_u64());
        self.runtime
            .turn_watch
            .begin(turn_watch::TurnSource::native(&event));
        self.runtime
            .begin_fresh_turn(orbit_core::notice::ReportSource::Daemon);
        // Every frame the helper sent before this event, first.
        self.runtime.capture_barrier(seq).await;
        if self.closing.allows_native_event(&event) {
            self.process.topic_board.state_changed();
            self.runtime.handle_native_event(event).await;
        }
        if let Some(seq) = seq {
            self.runtime.capture.served_control = seq;
            self.runtime.serve_forwarded(None).await;
        }
    }

    /// One item the capture task handed over: a helper event as the loop
    /// served it before, in the helper's order.
    async fn on_forwarded(&mut self) {
        self.turns.native += 1;
        self.runtime
            .turn_watch
            .begin(turn_watch::TurnSource::forwarded());
        self.runtime
            .begin_fresh_turn(orbit_core::notice::ReportSource::Daemon);
        self.process.topic_board.state_changed();
        // Only `audio` and `gap` are handed over, which a closure still takes
        // (`Shutdown::allows_native_event`).
        self.runtime.serve_forwarded(None).await;
    }

    async fn on_hub(&mut self, generation: u64, event: HubEvent) {
        self.turns.hub += 1;
        self.runtime
            .turn_watch
            .begin(turn_watch::TurnSource::hub(&event));
        let op = self.runtime.link.hub_event_op(&event);
        self.runtime
            .begin_turn(op, orbit_core::notice::ReportSource::Hub);
        if generation == self.runtime.state.generation() {
            self.process.topic_board.state_changed();
            if let Err(error) = self.runtime.hub_event(event).await {
                self.runtime
                    .say_failure(orbit_core::notice::NoticeCode::HubEventFailed, &error);
            }
        }
    }

    async fn on_work(&mut self, event: WorkResult) {
        self.turns.work += 1;
        self.runtime
            .turn_watch
            .begin(turn_watch::TurnSource::work(event.kind()));
        let op = event.op();
        self.runtime
            .begin_turn(op, orbit_core::notice::ReportSource::Daemon);
        self.process.topic_board.state_changed();
        if let Err(error) = self.runtime.work_event(event).await {
            self.runtime
                .say_failure(orbit_core::notice::NoticeCode::WorkFailed, &error);
        }
    }

    async fn on_note(&mut self, event: recording_notes::NoteEvent) {
        self.turns.notes += 1;
        self.runtime
            .turn_watch
            .begin(turn_watch::TurnSource::note_answer());
        self.runtime
            .begin_fresh_turn(orbit_core::notice::ReportSource::Daemon);
        self.process.topic_board.state_changed();
        if let Err(error) = self.runtime.note_answer(event).await {
            self.runtime
                .say_failure(orbit_core::notice::NoticeCode::WorkFailed, &error);
        }
    }

    async fn on_tick(&mut self) {
        self.turns.tick += 1;
        let tick_started = std::time::Instant::now();
        // Before this turn begins: the slowest turn since the last tick is
        // what held a late one.
        let slowest = self.runtime.turn_watch.take_slowest_since_tick();
        self.runtime
            .turn_watch
            .begin(turn_watch::TurnSource::tick());
        self.runtime
            .begin_fresh_turn(orbit_core::notice::ReportSource::Daemon);
        if let Some(late) = self.loop_lag.observe(tick_started, self.tick.period()) {
            self.log_stall(late, slowest.as_ref());
        }
        self.process.topic_board.state_changed();
        #[cfg(test)]
        self.runtime.inject_test_faults();
        Box::pin(self.runtime.supervise_workers()).await;
        Box::pin(self.runtime.run_vertical_maintenance()).await;
        Box::pin(self.runtime.run_state_maintenance()).await;
        let watch = self.runtime.turn_watch.clone();
        watch.time_sync("error_reports", || {
            self.runtime
                .error_reports
                .send_due(std::time::Instant::now());
        });
        self.last_tick = tick_started.elapsed();
    }

    /// The capture's own clock on the loop: a drain's deadline, the core's
    /// tick, a cut recording's recovery, a call's health and a closure,
    /// never behind the other verticals' upkeep.
    async fn on_capture_tick(&mut self) {
        self.turns.capture_tick += 1;
        self.runtime
            .turn_watch
            .begin(turn_watch::TurnSource::capture_tick());
        self.runtime
            .begin_fresh_turn(orbit_core::notice::ReportSource::Daemon);
        // The call's health is judged with what the loop has not read yet:
        // after a held loop, frames waiting here are audio that arrived, not
        // silence.
        let queued = self
            .runtime
            .capture
            .unserved_native_events(self.channels.native_events.len());
        self.runtime.audio.note_loop_tick(queued, self.last_tick);
        self.process.topic_board.state_changed();
        let watch = self.runtime.turn_watch.clone();
        watch
            .time("audio_timeout", self.runtime.handle_audio_timeout())
            .await;
        Box::pin(self.runtime.run_capture_maintenance()).await;
    }

    /// Codes and numbers only: how long, how much the helper had sent
    /// meanwhile that is still unread, and the slowest turn of the period
    /// with its step and wait.
    fn log_stall(&self, late: Duration, slowest: Option<&turn_watch::SlowTurn>) {
        let recording = self.runtime.state.durable.capture.recording.is_some();
        let mut fields = vec![
            ("stalled_ms", late.as_millis().to_string()),
            (
                "queued_native_events",
                self.runtime
                    .capture
                    .unserved_native_events(self.channels.native_events.len())
                    .to_string(),
            ),
            ("recording", recording.to_string()),
        ];
        fields.extend(
            slowest
                .into_iter()
                .flat_map(turn_watch::SlowTurn::stall_fields),
        );
        let fields: Vec<(&str, &str)> = fields
            .iter()
            .map(|(name, value)| (*name, value.as_str()))
            .collect();
        self.runtime.log("daemon_loop_stalled", &fields);
    }

    /// One aggregate at closure: no event payload, command, op or conversation.
    fn record_turns(&self) {
        let Some(diagnostics) = &self.runtime.diagnostics else {
            return;
        };
        let turns = &self.turns;
        diagnostics.record(
            "loop_turns",
            &[
                ("signals", &turns.signals.to_string()),
                ("commands", &turns.commands.to_string()),
                ("native", &turns.native.to_string()),
                ("hub", &turns.hub.to_string()),
                ("work", &turns.work.to_string()),
                ("notes", &turns.notes.to_string()),
                ("topic_due", &turns.topic_due.to_string()),
                ("tick", &turns.tick.to_string()),
                ("capture_tick", &turns.capture_tick.to_string()),
                (
                    "elapsed_ms",
                    &self.topics_started.elapsed().as_millis().to_string(),
                ),
            ],
        );
    }

    /// Every exit has already stopped producers and crossed the same PCM
    /// durability boundary. Do not start a second unobserved drain here.
    async fn close(
        &mut self,
        helper: Option<native::Supervisor>,
        capture: Option<capture_lane::task::CaptureTask>,
    ) {
        cancel_remote_reads(&mut self.runtime.reads.tasks);
        if !self.runtime.store_actor.bounded_exit_declared() {
            let _ = self.runtime.store_actor.snapshot(&self.runtime.state).await;
        }
        if let Some(task) = self.runtime.link.task.take() {
            task.abort();
        }
        self.runtime.transfers.worker.cancel();
        // The loop is gone: the task reads and drops the helper's last
        // frames, so a full channel cannot hold the helper's own closure.
        self.runtime.capture.lane.lock().discard = true;
        self.runtime.capture.lane.wake_task();
        if let Some(helper) = helper {
            // `shutdown`, its input closed, its grace, and only then a kill.
            let end = helper.stop(&mut self.channels.native_events).await;
            self.runtime.log_helper_end(end);
        }
        if let Some(mut capture) = capture {
            capture.stop();
        }
        if let Some(listener) = self.process.listener.take() {
            listener.abort();
            let _ = std::fs::remove_file(&self.runtime.paths.socket);
        }
        self.runtime
            .error_reports
            .flush(crate::error_reports::EXIT_FLUSH);
    }
}
