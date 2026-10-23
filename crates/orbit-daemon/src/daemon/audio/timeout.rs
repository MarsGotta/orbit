//! A stop the Mac never confirmed: what is live is cut with its cause, the
//! audio that arrived is kept and the missing tail is declared as a gap.
use super::super::Runtime;
use super::drain::{AUDIO_DRAIN_LIMIT, AUDIO_SILENCE_TIMEOUT};
use orbit_core::notice::NoticeCode;
use tokio::time::Instant;

impl Runtime {
    fn audio_timed_out(&self) -> bool {
        self.capture
            .audio
            .deadline
            .is_some_and(|deadline| Instant::now() >= deadline)
    }

    /// The Mac did not confirm an audio stop in its window, or its helper was
    /// lost while an unsent stop had no deadline left. What is live is cut
    /// with its cause; the audio that arrived is kept and the missing tail is
    /// declared as a gap. The user's stop stays in force.
    pub(in crate::daemon) async fn handle_audio_timeout(&mut self) {
        let helper_lost_with_unsent_stop =
            self.full_stop_send_owed() && !self.state.capabilities().native_helper();
        if !self.audio_timed_out() && !helper_lost_with_unsent_stop {
            return;
        }
        if helper_lost_with_unsent_stop {
            self.abandon_unsent_stop().await;
            return;
        }
        if self.full_stop_send_owed() && self.state.capabilities().native_helper() {
            // The order never entered the helper. A drain deadline cannot
            // prove its microphone closed or discard the retry debt while the
            // channel exists. Once the helper is gone, retry is impossible:
            // abandon the drain below and seal the audio already received.
            // The existing 500 ms send budget governs each maintenance
            // attempt; the drain window restarts only after delivery.
            self.capture.audio.deadline = None;
            self.say(NoticeCode::StopRetrying);
            let _ = self.store_actor.snapshot(&self.state).await;
            return;
        }
        if self.live_call_reached_drain_limit() && self.abandon_audio_drain().await.is_ok() {
            self.say(NoticeCode::CallDrainUnconfirmed);
            let _ = self.store_actor.snapshot(&self.state).await;
            return;
        }
        let waited_ms = self
            .capture
            .audio
            .started_at
            .map_or(0, |started| started.elapsed().as_millis());
        let cause = format!(
            "El Mac no confirmó la parada de audio en {waited_ms} ms (plazo: {} ms sin respuesta del Mac, {} ms en total). Se conserva el audio recibido; el final puede faltar.",
            AUDIO_SILENCE_TIMEOUT.as_millis(),
            AUDIO_DRAIN_LIMIT.as_millis()
        );
        self.contain(
            orbit_core::privacy_stop::StopCause::AudioStopUnconfirmed,
            &cause,
        )
        .await;
        let _ = self.abandon_audio_drain().await;
        let _ = self.store_actor.snapshot(&self.state).await;
    }

    /// The earlier timeout deliberately removed the drain deadline while the
    /// channel existed. Losing that channel later makes the retry impossible,
    /// even though no deadline can fire again.
    async fn abandon_unsent_stop(&mut self) {
        let name = match self.capture.audio.full_stop.as_ref() {
            Some((_, kind)) => kind.command(),
            None => "orden de parada",
        };
        let cause = format!(
            "El helper de Orbit.app se desconectó con {name} sin entregar tras 500 ms. Orbit deja de esperar al helper y envía el audio recibido a sellado local; el final puede faltar."
        );
        self.contain(
            orbit_core::privacy_stop::StopCause::AudioStopUnconfirmed,
            &cause,
        )
        .await;
        if self.abandon_audio_drain().await.is_ok() {
            // `halt` saw the still-owned audio before it could be sealed
            // and said a storage warning. Once the local handoff
            // succeeds, that warning and the old retry promise are false.
            self.say_failure(NoticeCode::AudioStopUnconfirmed, &cause);
        }
        let _ = self.store_actor.snapshot(&self.state).await;
    }
}
