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
        let started_at = *self.audio.lifecycle.started_at.get_or_insert(observed_at);
        self.audio
            .lifecycle
            .last_liveness
            .get_or_insert(observed_at);
        self.audio.lifecycle.deadline = Some(drain_deadline(observed_at, started_at));
        self.state.views.draining_since_ms.get_or_insert_with(now);
        if announce {
            self.say(NoticeCode::AudioDrainWaiting);
        }
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
            .audio
            .lifecycle
            .stops
            .values()
            .any(|pending| pending.stream == stream)
        {
            return Ok(());
        }
        let command = native::command("audio_stop", json!({"stream_id":stream}))?;
        let command_id = NativeCommandHeader::from_value(&command)?.id.to_owned();
        self.audio.lifecycle.stops.insert(
            command_id,
            PendingStop {
                stream,
                source,
                answered: false,
            },
        );
        self.begin_audio_drain(true);
        self.helper.record_pending(&command);
        match self
            .helper
            .send_within(command, Duration::from_millis(500))
            .await
        {
            Ok(Ok(())) => Ok(()),
            Ok(Err(_closed)) => {
                bail!("The Mac did not receive audio_stop: the Orbit.app channel is closed")
            }
            Err(_elapsed) => bail!(
                "The Mac did not receive audio_stop within 500 ms; the channel is still present but did not accept the command within its deadline"
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
            .audio
            .lifecycle
            .full_stop
            .as_ref()
            .filter(|(id, _)| id == command_id)
            .map(|(_, kind)| *kind);
        if (full_stop.is_some() || self.audio.lifecycle.stops.contains_key(command_id))
            && !decode_result_body::<DrainAcknowledgement>("audio_stop", result)?.drain_complete
        {
            bail!(
                "The helper did not confirm a complete drain; audio and the pending stop are retained"
            );
        }
        if let Some(kind) = full_stop {
            self.full_stop_drained(kind).await?;
            return Ok(true);
        }
        let Some(pending) = self.audio.lifecycle.stops.remove(command_id) else {
            return Ok(false);
        };
        if pending.source == "mic_local"
            && self.audio.mic_stream.as_deref() == Some(&pending.stream)
        {
            self.audio.mic_stream = None;
        }
        if pending.source == "remote_app"
            && self.audio.remote_stream.as_deref() == Some(&pending.stream)
        {
            self.audio.remote_stream = None;
        }
        let segments = self.capture.pause_source(pending.source)?;
        self.seal(segments)?;
        self.complete_audio_drain().await?;
        Ok(true)
    }

    /// The whole-helper stop's receipt drained every stream the helper held,
    /// including the selective stops in the bounded retired ledger. The
    /// stop's cause stays the line the user reads; the confirmation is added.
    async fn full_stop_drained(&mut self, kind: FullStop) -> Result<()> {
        self.flush_stopped_audio().await?;
        self.audio.retired_stops.clear();
        let confirmed = match kind {
            FullStop::Privacy => NoticeCode::PrivacyStopConfirmed,
            FullStop::Cut => NoticeCode::CaptureCutConfirmed,
        };
        let nothing_kept = self
            .state
            .durable
            .capture
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
        if self.audio.lifecycle.pending() {
            return Ok(());
        }
        self.audio.lifecycle.deadline = None;
        self.audio.lifecycle.started_at = None;
        self.audio.lifecycle.last_liveness = None;
        self.state.views.draining_since_ms = None;
        if !self.close_drained_recording().await? {
            return Ok(());
        }
        self.dispatch_drained_voice().await?;
        // Expiry can interrupt a recording while its stream still owes an
        // audio_stop receipt. Only now, after that receipt and the final seal,
        // can an empty capture release its recording identity.
        if let Some(interruption) = self.capture.interruption().cloned() {
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
        self.audio.lifecycle.full_stop_owed = true;
        self.audio.mark_call_stopping();
        self.audio.lifecycle.voice.submit = false;
        self.audio.lifecycle.voice.end = false;
        self.audio.lifecycle.meeting.recording_end = Some(true);
        self.audio.pending_starts.clear();
        if self.audio.lifecycle.full_stop.is_some() {
            return Ok(());
        }
        if !self.state.capabilities().native_helper() {
            self.flush_stopped_audio().await?;
            return Ok(());
        }
        let name = kind.command();
        let command = native::command(name, json!({}))?;
        self.audio.lifecycle.full_stop = Some((
            NativeCommandHeader::from_value(&command)?.id.to_owned(),
            kind,
        ));
        self.audio.lifecycle.full_stop_send_owed = false;
        self.helper.record_pending(&command);
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
        match self
            .helper
            .send_within(command, Duration::from_millis(500))
            .await
        {
            Ok(Ok(())) => Ok(()),
            Ok(Err(_closed)) => {
                self.state = decide(&self.state, Event::HelperStopUndelivered, now())?.state;
                bail!(
                    "The Mac did not receive command {name}: the Orbit.app channel is closed. Orbit no longer retains audio from that capture."
                )
            }
            Err(_elapsed) => {
                self.audio.lifecycle.full_stop_send_owed = true;
                bail!(
                    "The Mac did not receive command {name} within 500 ms. Orbit no longer retains audio from that capture; maintenance retries the command until the channel accepts it."
                )
            }
        }
    }

    /// Resend a whole-helper stop which timed out before it entered the helper's
    /// channel. Success means delivered to that channel, not microphone
    /// closed; `full_stop` remains until the matching drain response arrives.
    pub(in crate::daemon) async fn retry_full_stop_send(&mut self) -> Result<()> {
        if !self.audio.lifecycle.full_stop_send_owed || !self.state.capabilities().native_helper() {
            return Ok(());
        }
        let Some((command_id, kind)) = self.audio.lifecycle.full_stop.as_ref() else {
            self.audio.lifecycle.full_stop_send_owed = false;
            return Ok(());
        };
        let command_id = command_id.clone();
        let name = kind.command();
        let command = native::command(name, json!({"id":command_id}))?;
        self.helper.record_pending(&command);
        match self
            .helper
            .send_within(command, Duration::from_millis(500))
            .await
        {
            Ok(Ok(())) => {
                self.audio.lifecycle.full_stop_send_owed = false;
                // The drain window starts when the helper actually receives
                // this order, not when its full channel rejected it.
                self.audio.lifecycle.started_at = None;
                self.begin_audio_drain(false);
                Ok(())
            }
            Ok(Err(_closed)) => {
                self.state = decide(&self.state, Event::HelperStopUndelivered, now())?.state;
                bail!("The Mac did not receive command {name}: the Orbit.app channel is closed")
            }
            Err(_elapsed) => bail!(
                "The Mac did not receive command {name} within 500 ms; the channel is still present and the stop remains pending"
            ),
        }
    }

    pub(in crate::daemon) async fn flush_stopped_audio(&mut self) -> Result<()> {
        // All native tail frames must arrive before this local durability
        // boundary. On failure voice stays owned here or in pending_seals.
        self.audio.lifecycle.voice.submit = false;
        self.dispatch_voice_transcription().await?;
        self.audio.mic_stream = None;
        self.audio.remote_stream = None;
        self.audio.pending_starts.clear();
        self.audio.lifecycle = super::AudioLifecycle::default();
        self.state.views.draining_since_ms = None;
        let segments = self.capture.flush(true);
        self.seal(segments)?;
        if let Some(interruption) = self.capture.interruption().cloned() {
            Box::pin(self.close_empty_interrupted(interruption)).await?;
        }
        self.finish_call_drain().await?;
        Ok(())
    }

    /// Whether a whole-helper stop still owes the local flush of the audio it
    /// stopped: always under the user's stop, and after a cut or a closing until
    /// the flush succeeds.
    pub(in crate::daemon) fn stopped_audio_owed(&self) -> bool {
        self.state.privacy_stopped() || self.audio.lifecycle.full_stop_owed
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

impl crate::daemon::audio::AudioHandle {
    pub(in crate::daemon) fn observe_audio_helper_liveness(&mut self, event: &Value) {
        if !self.lifecycle.pending()
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
        let Some(started_at) = self.lifecycle.started_at else {
            return;
        };
        self.lifecycle.last_liveness = Some(observed_at);
        self.lifecycle.deadline = Some(drain_deadline(observed_at, started_at));
    }

    pub(in crate::daemon) fn audio_is_draining(&self) -> bool {
        self.lifecycle.pending()
    }

    pub(in crate::daemon) fn full_stop_send_owed(&self) -> bool {
        self.lifecycle.full_stop_send_owed
    }
}
