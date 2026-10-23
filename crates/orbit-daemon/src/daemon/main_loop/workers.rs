//! The loop's workers under supervision, once a second. A worker that ended
//! in a panic delivers nothing, so its owner learns it here and starts again
//! from what is durable, as after any failure of that worker. Each fall is
//! counted (`actor_panicked`); a spent budget ends the daemon from the loop.
use super::*;
use crate::supervision::Actor;

#[cfg(test)]
mod daemon_tests;
#[cfg(test)]
mod tests;

/// The failure a voice turn lost to a panic is said with: a code, as the
/// worker's own local failures are.
const VOICE_PANIC_CAUSE: &str = "voice_worker_panicked";

impl Runtime {
    pub(in crate::daemon) async fn supervise_workers(&mut self) {
        self.relaunch_hub().await;
        if self.work.worker.panicked().is_some() {
            // Which transfer it was is in the store: the next schedule lists
            // the closure or the cleanup again, `running` ones included.
            self.work.worker.cancel();
            self.supervision.panicked(Actor::TransferWorker);
        }
        self.reap_voice_turn().await;
        if self.conversation.speech_task.panicked().is_some() {
            // Nothing waits on a reading but the slot itself.
            self.conversation.speech_task.cancel();
            self.supervision.panicked(Actor::Playback);
        }
        if let Some(ticket) = self.features.computer.work.panicked() {
            self.supervision.panicked(Actor::ComputerControl);
            let failure = anyhow::anyhow!(
                "El trabajo del motor de control del Mac falló; se puede volver a pedir"
            );
            if let Err(error) = self.computer_event(&ticket, Err(failure)).await {
                self.say_failure(orbit_core::notice::NoticeCode::WorkFailed, &error);
            }
        }
        for actor in self.features.notes.reap_panicked() {
            self.supervision.panicked(actor);
        }
    }

    /// The hub's connection fell: the link is said lost, as after any lost
    /// socket, and connected again from the saved credential.
    async fn relaunch_hub(&mut self) {
        if !self.supervision.take_down(Actor::HubConnection) {
            return;
        }
        // Another path may have connected it again already.
        if self
            .hub_task
            .as_ref()
            .is_some_and(|task| !task.is_finished())
        {
            return;
        }
        self.hub_task = None;
        if let Err(error) = self.apply(Event::HubDisconnected).await {
            self.say_failure(orbit_core::notice::NoticeCode::HubEventFailed, &error);
        }
        self.connect();
    }

    /// A voice turn lost to a panic fails as a turn the hub could not
    /// transcribe: its note says why and its audio stays on this Mac.
    async fn reap_voice_turn(&mut self) {
        let Some(ticket) = self.conversation.voice_task.panicked() else {
            return;
        };
        self.supervision.panicked(Actor::VoiceTurn);
        let Some(operation_id) = self.conversation.voice_operation.clone() else {
            self.conversation.voice_task.cancel();
            return;
        };
        let event = WorkResult::Capture(CaptureWork::VoiceProblem {
            ticket,
            operation_id,
            failure: orbit_core::VoiceTranscriptionFailure::local(VOICE_PANIC_CAUSE),
        });
        if let Err(error) = self.work_event(event).await {
            self.say_failure(orbit_core::notice::NoticeCode::WorkFailed, &error);
        }
    }
}

/// Faults a test asks of a running daemon, each by a file it creates in the
/// daemon's `test-faults/` directory and that is removed as it fires:
/// `store_writer` hands the writer one more panicking operation than its
/// budget, `loop` panics the loop itself. Nothing in a build without tests
/// looks for them.
#[cfg(test)]
impl Runtime {
    #[allow(clippy::panic, reason = "a test injects these panics on purpose")]
    pub(in crate::daemon) fn inject_test_faults(&mut self) {
        let faults = self.paths.root.join("test-faults");
        if std::fs::remove_file(faults.join("store_writer")).is_ok() {
            for _ in 0..=crate::supervision::RESTART_BUDGET {
                let _abandoned = self
                    .store_actor
                    .begin_run(|_| -> Result<()> { panic!("injected store writer panic") });
            }
        }
        if std::fs::remove_file(faults.join("loop")).is_ok() {
            panic!("injected loop panic");
        }
    }
}
