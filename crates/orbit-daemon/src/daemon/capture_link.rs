//! The loop's side of the capture lane: when the capture task may serve the
//! recording alone, what the task hands back, and the order between the
//! helper events the loop serves and the frames the task handled.
use super::capture_lane::{ForwardedItem, Gate};
use super::*;
use orbit_core::privacy_stop::StopCause;

/// How long the loop waits for the capture task to reach a helper event's
/// place before serving it anyway. What it protects: the loop itself, from a
/// task that stopped. The task does nothing that waits, so it normally takes
/// microseconds; five seconds is the strict turn bound the capture tests hold
/// the loop to (`STRICT_TURN_BOUND`). Past it the order is no longer kept
/// for that event, and `capture_barrier_late` says so.
const BARRIER_BOUND: Duration = Duration::from_secs(5);

/// The recording (its id and phase), the privacy stop, a call and a voice
/// note: what the capture lane's arming depends on in the core state.
type GateInputs = (Option<(String, CapturePhase)>, bool, bool, bool);

/// A decision that changes any of the gate's inputs disarms the lane before
/// it takes effect.
fn gate_inputs(state: &State) -> GateInputs {
    (
        state
            .durable
            .capture
            .recording
            .as_ref()
            .map(|recording| (recording.id.clone(), recording.phase.clone())),
        state.privacy_stopped(),
        state.durable.voice.call.is_some(),
        state.durable.voice.voice_active,
    )
}

impl Runtime {
    /// A recording in its plain steady state: nothing but the recording
    /// consumes its frames, nothing is draining or owed, and the reserve and
    /// the writer are well. Only then may the task serve frames alone.
    fn capture_steady(&self) -> bool {
        !self.closing
            && self.recording_active()
            && !self.state.privacy_stopped()
            && self.state.durable.voice.call.is_none()
            && self.audio.call.is_none()
            && !self.state.durable.voice.voice_active
            && self.capture.buffer.voice.is_none()
            && !self.audio.audio_is_draining()
            && !self.stopped_audio_owed()
            && !self.audio.full_stop_send_owed()
            && !self.capture.reserve_exhausted
            && self.capture.seal_retry_at.is_none()
            && self.audio.mic_stream.is_some()
    }

    /// Before a decision takes effect: one that changes what the arming
    /// depends on stops the task's own handling until the turn's end, and
    /// one that retires the uploads (a privacy stop, a closure, another
    /// authority: a new generation) stops them now, the transfer in flight
    /// included, as the loop's own worker is cancelled.
    pub(super) fn disarm_capture_for(&self, next: &State) {
        if gate_inputs(&self.state) != gate_inputs(next) {
            self.capture.lane.disarm();
        }
        let mut core = self.capture.lane.lock();
        core.upload.privacy_stopped = next.privacy_stopped();
        core.upload.online = core.upload.online && next.connection() == Connection::Online;
        if next.generation() != core.upload.generation {
            core.upload.generation = next.generation();
            if let Some(transfer) = core.upload_abort.take() {
                transfer.abort();
            }
        }
    }

    /// What the uploads may do now, as the loop's state says.
    pub(super) fn upload_view(&self) -> capture_lane::UploadView {
        capture_lane::UploadView {
            online: self.state.connection() == Connection::Online,
            privacy_stopped: self.state.privacy_stopped(),
            generation: self.state.generation(),
            token: self.link.token.clone(),
            hub_url: self.settings.hub_url.clone(),
            development: self.development,
            discarded: self.notes.outbox.discarded(),
        }
    }

    /// What the task's chunk transfers left for the loop: the closures they
    /// make ready, and what to say about the ones that failed.
    async fn absorb_uploads(&mut self) {
        let reports = std::mem::take(&mut self.capture.lane.lock().outbox.uploads);
        let mut delivered = false;
        for report in reports {
            match report {
                capture_lane::UploadReport::Settled { recording, failure } => {
                    if let Some(recording) = recording {
                        self.transfers.finalize_ready.remove(&recording);
                    }
                    match failure {
                        Some((error, notice)) => {
                            self.log("upload_failed", &[("error", &error)]);
                            self.say_upload_failure(notice);
                        }
                        None => delivered = true,
                    }
                }
                capture_lane::UploadReport::NotSaved(error) => {
                    self.say_failure(orbit_core::notice::NoticeCode::WorkFailed, &error);
                }
                capture_lane::UploadReport::Problem(error) => {
                    let text = format!(
                        "Failed to save Orbit state or upload queue: {error}. Nothing is cut."
                    );
                    self.contain(StopCause::VerticalMaintenanceFailed, &text)
                        .await;
                }
            }
        }
        // A delivered chunk may be the last its closure waited for.
        if delivered
            && !self.state.privacy_stopped()
            && let Err(error) = Box::pin(self.schedule_upload()).await
        {
            self.log("upload_schedule_failed", &[("error", &error.to_string())]);
        }
    }

    /// At a turn's end: what the task observed is counted, and the lane is
    /// armed when the recording is steady and nothing the task handed over
    /// is still waiting, or disarmed.
    pub(super) async fn settle_capture_lane(&mut self) {
        Box::pin(self.absorb_uploads()).await;
        if let Err(error) = self.collect_ready_seals(sealing::OPPORTUNISTIC_RECEIPT_WAIT, false) {
            self.log("capture_receipts_failed", &[("error", &error.to_string())]);
        }
        let signals = std::mem::take(&mut self.capture.lane.lock().outbox.signals);
        for (signal, at) in signals {
            self.observe_recording_signal(Some(signal), at);
        }
        if !self.recording_active() {
            self.finish_audio_failures();
        }
        let steady = self.capture_steady();
        let view = self.upload_view();
        {
            let mut core = self.capture.lane.lock();
            if core.upload.generation != view.generation
                && let Some(transfer) = core.upload_abort.take()
            {
                transfer.abort();
            }
            core.upload = view;
            if steady && core.forwarded.is_empty() {
                core.gate = Gate {
                    armed: true,
                    microphone: self.audio.mic_stream.clone(),
                    remote: self.audio.remote_stream.clone(),
                };
            } else {
                core.gate.armed = false;
            }
        }
        self.capture.lane.wake_task();
    }

    /// Serves, in order, what the task handed over and may be served now.
    /// Returns how many items it served.
    pub(super) async fn serve_forwarded(&mut self, before: Option<u64>) -> usize {
        let mut served = 0;
        while let Some(forwarded) = self.capture.take_forwarded(before) {
            self.capture.lane.wake_task();
            Box::pin(self.serve_forwarded_item(forwarded.item)).await;
            served += 1;
        }
        served
    }

    async fn serve_forwarded_item(&mut self, item: ForwardedItem) {
        match item {
            ForwardedItem::Event(event) => self.handle_native_event(event).await,
            ForwardedItem::AudioHole { source } => {
                if let Err(error) = self.note_audio_hole(source, "audio_discontinuity") {
                    self.drop_audio_event(Some(source), &error.to_string());
                }
            }
            ForwardedItem::FrameFailed { frame, error } => {
                let source = frame["source"].as_str();
                self.drop_audio_event(source, &error);
            }
            ForwardedItem::Segments { segments } => {
                if let Err(error) = self.seal(segments) {
                    self.drop_audio_event(None, &error.to_string());
                }
            }
            ForwardedItem::SealRefused { error } => {
                self.drop_audio_event(None, &error);
            }
        }
    }

    /// Before the loop serves the helper event at `seq`: the task has handled
    /// every frame the helper sent before it, and the loop has served those
    /// the task handed over. An event without a place came to the loop
    /// because the task's channel is closed (the task is down): what the
    /// task handed over before it went down came first, and is served first.
    pub(super) async fn capture_barrier(&mut self, seq: Option<u64>) {
        let deadline = std::time::Instant::now() + BARRIER_BOUND;
        loop {
            let (reached, running) = {
                let core = self.capture.lane.lock();
                (
                    seq.is_some_and(|seq| core.consumed_through >= seq),
                    core.handing_over || (seq.is_some() && core.task_running),
                )
            };
            // Read progress first: every item up to that position is already
            // queued, so the drain below cannot miss one racing the barrier.
            self.serve_forwarded(seq).await;
            if reached || !running {
                return;
            }
            if std::time::Instant::now() >= deadline {
                self.log(
                    "capture_barrier_late",
                    &[("waited_ms", &BARRIER_BOUND.as_millis().to_string())],
                );
                return;
            }
            tokio::select! {
                () = self.capture.lane.progressed() => {}
                // What the task handed over may be what holds it.
                () = self.capture.lane.loop_woken() => {}
                () = tokio::time::sleep(Duration::from_millis(5)) => {}
            }
        }
    }
}

impl crate::daemon::capture_handle::CaptureHandle {
    /// Helper events the loop has not served yet: those in its channel plus
    /// those the task handed over or still has to read.
    pub(super) fn unserved_native_events(&self, in_channel: usize) -> usize {
        let core = self.lane.lock();
        in_channel
            .saturating_add(core.forwarded.len())
            .saturating_add(core.backlog)
    }

    /// The oldest item the task handed over, if it may be served now and
    /// comes before the helper event at `before`.
    fn take_forwarded(&mut self, before: Option<u64>) -> Option<capture_lane::Forwarded> {
        let mut core = self.lane.lock();
        let front = core.forwarded.front()?;
        if front.after_control > self.served_control
            || before.is_some_and(|before| front.seq >= before)
        {
            return None;
        }
        core.forwarded.pop_front()
    }
}
