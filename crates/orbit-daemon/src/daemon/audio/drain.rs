//! How a stream any lifecycle holds is let go: its selective `audio_stop`,
//! or the whole-helper stop, and the helper's drain receipt that ends it.
//! Nothing here decides who may hold the microphone or what a lifecycle does
//! once its audio is in; each lifecycle's own module does.
use super::super::{Runtime, now};
use crate::native;
use anyhow::{Result, bail};
use orbit_core::notice::{Notice, NoticeCode, NoticeKey};
use orbit_core::wire::native::{NativeCommandHeader, NativeEventHeader, decode_result_body};
use orbit_core::{CapturePhase, Event, decide};
use serde::Deserialize;
use serde_json::{Value, json};
use std::time::Duration;
use tokio::time::Instant;

/// How long the helper may go without saying anything while a stop waits
/// for its drain, and how long the drain may take in all.
pub(super) const AUDIO_SILENCE_TIMEOUT: Duration = Duration::from_secs(3);
pub(super) const AUDIO_DRAIN_LIMIT: Duration = Duration::from_secs(15);

pub(in crate::daemon) struct PendingStop {
    pub(super) stream: String,
    pub(super) source: &'static str,
    pub(super) answered: bool,
}

#[derive(Deserialize)]
struct DrainAcknowledgement {
    drain_complete: bool,
}

/// Which whole-helper stop is waiting for its drain.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::daemon) enum FullStop {
    /// The user's stop: the helper writes its durable latch (`privacy_stop`).
    Privacy,
    /// A cut, a closing or a revocation: the helper stops every capture
    /// source and leaves its latch as it was (`capture_stop`).
    Cut,
}

impl FullStop {
    pub(super) fn command(self) -> &'static str {
        match self {
            Self::Privacy => "privacy_stop",
            Self::Cut => "capture_stop",
        }
    }
}

impl Runtime {
    pub(super) fn begin_audio_drain(&mut self, announce: bool) {
        let observed_at = Instant::now();
        let started_at = *self.capture.audio.started_at.get_or_insert(observed_at);
        self.capture.audio.last_liveness.get_or_insert(observed_at);
        self.capture.audio.deadline = Some(drain_deadline(observed_at, started_at));
        self.state.views.draining_since_ms.get_or_insert_with(now);
        if announce {
            self.say(NoticeCode::AudioDrainWaiting);
        }
    }

    pub(in crate::daemon) fn observe_audio_helper_liveness(&mut self, event: &Value) {
        if !self.capture.audio.pending()
            || !matches!(
                NativeEventHeader::from_value(event)
                    .ok()
                    .map(|header| header.event),
                Some("state" | "audio" | "response" | "local_rearm")
            )
        {
            return;
        }
        let observed_at = Instant::now();
        let Some(started_at) = self.capture.audio.started_at else {
            return;
        };
        self.capture.audio.last_liveness = Some(observed_at);
        self.capture.audio.deadline = Some(drain_deadline(observed_at, started_at));
    }

    /// Timeout/EOF is not a successful barrier. Preserve what arrived and
    /// declare uncertain missing tail samples before clearing source identity.
    pub(in crate::daemon) async fn abandon_audio_drain(&mut self) -> Result<()> {
        let mut gap_error = None;
        for source in ["mic_local", "remote_app"] {
            if let Err(error) = self.note_bound_audio_gap(source) {
                gap_error = Some(error);
            }
        }
        self.flush_stopped_audio().await?;
        if let Some(error) = gap_error {
            return Err(error);
        }
        Ok(())
    }

    pub(super) async fn request_stream_stop(
        &mut self,
        source: &'static str,
        stream: String,
    ) -> Result<()> {
        if self
            .capture
            .audio
            .stops
            .values()
            .any(|pending| pending.stream == stream)
        {
            return Ok(());
        }
        let command = native::command("audio_stop", json!({"stream_id":stream}))?;
        let command_id = NativeCommandHeader::from_value(&command)?.id.to_owned();
        self.capture.audio.stops.insert(
            command_id,
            PendingStop {
                stream,
                source,
                answered: false,
            },
        );
        self.begin_audio_drain(true);
        self.record_pending_native(&command);
        match tokio::time::timeout(Duration::from_millis(500), self.native.send(command)).await {
            Ok(Ok(())) => Ok(()),
            Ok(Err(_closed)) => {
                bail!("El Mac no recibió la orden audio_stop: el canal con Orbit.app está cerrado")
            }
            Err(_elapsed) => bail!(
                "El Mac no recibió la orden audio_stop en 500 ms; el canal sigue presente pero no aceptó la orden dentro de su plazo"
            ),
        }
    }

    /// The wait for the Mac's drain ends with this notice instead.
    pub(in crate::daemon) fn replace_drain_wait(&mut self, notice: Notice) {
        self.say(notice);
    }

    pub(in crate::daemon) async fn audio_response(
        &mut self,
        command_id: &str,
        result: &Value,
    ) -> Result<bool> {
        let full_stop = self
            .capture
            .audio
            .full_stop
            .as_ref()
            .filter(|(id, _)| id == command_id)
            .map(|(_, kind)| *kind);
        if (full_stop.is_some() || self.capture.audio.stops.contains_key(command_id))
            && !decode_result_body::<DrainAcknowledgement>("audio_stop", result)?.drain_complete
        {
            bail!(
                "El helper no confirmó el drenaje completo; se conserva el audio y la parada pendiente"
            );
        }
        if let Some(kind) = full_stop {
            self.full_stop_drained(kind).await?;
            return Ok(true);
        }
        let Some(pending) = self.capture.audio.stops.remove(command_id) else {
            return Ok(false);
        };
        if pending.source == "mic_local"
            && self.capture.mic_stream.as_deref() == Some(&pending.stream)
        {
            self.capture.mic_stream = None;
        }
        if pending.source == "remote_app"
            && self.capture.remote_stream.as_deref() == Some(&pending.stream)
        {
            self.capture.remote_stream = None;
        }
        let segments = self.capture.buffer.pause_source(pending.source)?;
        self.seal(segments)?;
        self.complete_audio_drain().await?;
        Ok(true)
    }

    /// The whole-helper stop's receipt drained every stream the helper held,
    /// including the selective stops in the bounded retired ledger. The
    /// stop's cause stays the line the user reads; the confirmation is added.
    async fn full_stop_drained(&mut self, kind: FullStop) -> Result<()> {
        self.flush_stopped_audio().await?;
        self.capture.retired_stops.clear();
        let confirmed = match kind {
            FullStop::Privacy => NoticeCode::PrivacyStopConfirmed,
            FullStop::Cut => NoticeCode::CaptureCutConfirmed,
        };
        let nothing_kept = self
            .state
            .durable
            .recording
            .as_ref()
            .is_some_and(|recording| {
                recording.phase == CapturePhase::Interrupted
                    && recording.ended_at_ms.is_some()
                    && recording.sealed_chunks == 0
            });
        if nothing_kept {
            self.say(confirmed);
        } else {
            self.say(Notice::new(confirmed).with(NoticeKey::Fragments, true));
        }
        self.store_actor.snapshot(&self.state).await?;
        Ok(())
    }

    pub(in crate::daemon) async fn complete_audio_drain(&mut self) -> Result<()> {
        if self.capture.audio.pending() {
            return Ok(());
        }
        self.capture.audio.deadline = None;
        self.capture.audio.started_at = None;
        self.capture.audio.last_liveness = None;
        self.state.views.draining_since_ms = None;
        if !self.close_drained_recording().await? {
            return Ok(());
        }
        self.dispatch_drained_voice().await?;
        // Expiry can interrupt a recording while its stream still owes an
        // audio_stop receipt. Only now, after that receipt and the final seal,
        // can an empty capture release its recording identity.
        if let Some(interruption) = self.capture.interruption.clone() {
            Box::pin(self.close_empty_interrupted(interruption)).await?;
        }
        self.finish_call_drain().await?;
        if self.state.notice_code() == Some(NoticeCode::AudioDrainWaiting) {
            self.say(NoticeCode::AudioDrainConfirmed);
        }
        self.store_actor.snapshot(&self.state).await?;
        Ok(())
    }

    /// Stops every capture source in the helper and waits for its drain: the
    /// user's latching stop, or a cut that leaves the latch as it was.
    pub(in crate::daemon) async fn request_full_audio_stop(
        &mut self,
        kind: FullStop,
    ) -> Result<()> {
        self.capture.audio.full_stop_owed = true;
        self.mark_call_stopping();
        self.capture.audio.voice.submit = false;
        self.capture.audio.voice.end = false;
        self.capture.audio.meeting.recording_end = Some(true);
        self.capture.pending_starts.clear();
        if self.capture.audio.full_stop.is_some() {
            return Ok(());
        }
        if !self.state.capabilities().native_helper() {
            self.flush_stopped_audio().await?;
            return Ok(());
        }
        let name = kind.command();
        let command = native::command(name, json!({}))?;
        self.capture.audio.full_stop = Some((
            NativeCommandHeader::from_value(&command)?.id.to_owned(),
            kind,
        ));
        self.capture.audio.full_stop_send_owed = false;
        self.record_pending_native(&command);
        self.begin_audio_drain(false);
        // The caller already wrote why the stop happened, and that cause is the
        // only line telling a person what to do about it. Overwriting it made
        // every stop read identically — a changed audio device looked exactly
        // like a deliberate brake — so the wait is appended, never substituted.
        if self.state.durable.notice.is_none() {
            self.say(Notice::new(NoticeCode::FullStopRequested).with_token(NoticeKey::Code, name));
        }
        // Every helper send carries the 500 ms deadline. This bound
        // protects the actor from a full helper channel; it is the measured
        // native command deadline, not evidence that Orbit.app died and not a
        // deadline after which the microphone is known to be closed.
        match tokio::time::timeout(Duration::from_millis(500), self.native.send(command)).await {
            Ok(Ok(())) => Ok(()),
            Ok(Err(_closed)) => {
                self.state = decide(&self.state, Event::HelperStopUndelivered, now())?.state;
                bail!(
                    "El Mac no recibió la orden {name}: el canal con Orbit.app está cerrado. Orbit ya no guarda el audio de esa captura."
                )
            }
            Err(_elapsed) => {
                self.capture.audio.full_stop_send_owed = true;
                bail!(
                    "El Mac no recibió la orden {name} en 500 ms. Orbit ya no guarda el audio de esa captura; la orden se reintenta desde el mantenimiento hasta que el canal la acepte."
                )
            }
        }
    }

    /// Resend a whole-helper stop which timed out before it entered the helper's
    /// channel. Success means delivered to that channel, not microphone
    /// closed; `full_stop` remains until the matching drain response arrives.
    pub(in crate::daemon) async fn retry_full_stop_send(&mut self) -> Result<()> {
        if !self.capture.audio.full_stop_send_owed || !self.state.capabilities().native_helper() {
            return Ok(());
        }
        let Some((command_id, kind)) = self.capture.audio.full_stop.as_ref() else {
            self.capture.audio.full_stop_send_owed = false;
            return Ok(());
        };
        let command_id = command_id.clone();
        let name = kind.command();
        let command = native::command(name, json!({"id":command_id}))?;
        self.record_pending_native(&command);
        match tokio::time::timeout(Duration::from_millis(500), self.native.send(command)).await {
            Ok(Ok(())) => {
                self.capture.audio.full_stop_send_owed = false;
                // The drain window starts when the helper actually receives
                // this order, not when its full channel rejected it.
                self.capture.audio.started_at = None;
                self.begin_audio_drain(false);
                Ok(())
            }
            Ok(Err(_closed)) => {
                self.state = decide(&self.state, Event::HelperStopUndelivered, now())?.state;
                bail!("El Mac no recibió la orden {name}: el canal con Orbit.app está cerrado")
            }
            Err(_elapsed) => bail!(
                "El Mac no recibió la orden {name} en 500 ms; el canal sigue presente y la parada continúa pendiente"
            ),
        }
    }

    pub(in crate::daemon) async fn flush_stopped_audio(&mut self) -> Result<()> {
        // All native tail frames must arrive before this local durability
        // boundary. On failure voice stays owned here or in pending_seals.
        self.capture.audio.voice.submit = false;
        self.dispatch_voice_transcription().await?;
        self.capture.mic_stream = None;
        self.capture.remote_stream = None;
        self.capture.pending_starts.clear();
        self.capture.audio = super::AudioLifecycle::default();
        self.state.views.draining_since_ms = None;
        let segments = self.capture.buffer.flush(true);
        self.seal(segments)?;
        if let Some(interruption) = self.capture.interruption.clone() {
            Box::pin(self.close_empty_interrupted(interruption)).await?;
        }
        self.finish_call_drain().await?;
        Ok(())
    }

    pub(in crate::daemon) fn audio_is_draining(&self) -> bool {
        self.capture.audio.pending()
    }

    /// Whether a whole-helper stop still owes the local flush of the audio it
    /// stopped: always under the user's stop, and after a cut or a closing until
    /// the flush succeeds.
    pub(in crate::daemon) fn stopped_audio_owed(&self) -> bool {
        self.state.privacy_stopped() || self.capture.audio.full_stop_owed
    }

    pub(in crate::daemon) fn full_stop_send_owed(&self) -> bool {
        self.capture.audio.full_stop_send_owed
    }
}

/// The drain's deadline seen at `observed_at`: three seconds of the helper's
/// silence, never past fifteen from its start.
fn drain_deadline(observed_at: Instant, started_at: Instant) -> Instant {
    observed_at
        .checked_add(AUDIO_SILENCE_TIMEOUT)
        .unwrap_or(observed_at)
        .min(
            started_at
                .checked_add(AUDIO_DRAIN_LIMIT)
                .unwrap_or(started_at),
        )
}
