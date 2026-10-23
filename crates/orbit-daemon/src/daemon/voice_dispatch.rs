//! A finished voice turn, from the microphone's PCM to the hub: stored on
//! this Mac before it is sent, or, over Deimos's answer, heard first.
use super::audio::VoiceSource;
use super::{Runtime, WorkResult, id, now};
use crate::{hub, native};
use anyhow::{Context, Result};
use orbit_core::notice::NoticeCode;
use orbit_core::{Event, decide};
use orbit_store::{ChunkMetadata, Job, JobState};
use serde_json::json;

impl Runtime {
    pub(super) async fn dispatch_voice_transcription(&mut self) -> Result<()> {
        if self.capture.audio.submit_voice {
            self.prepend_joined_call_turn();
        }
        let Some(voice) = self.capture.buffer.voice.as_ref() else {
            return Ok(());
        };
        if voice.pcm.is_empty() {
            self.capture.buffer.voice = None;
            self.say(NoticeCode::VoiceNoteEmpty);
            return Ok(());
        }
        let cancelled = self.state.privacy_stopped() || !self.capture.audio.submit_voice;
        let duration_ms = voice_duration_ms(voice);
        // A call's turns have their own detector, which already asks for
        // this much speech.
        if !cancelled
            && self.state.durable.call.is_none()
            && let Some(short) = orbit_core::voice::short_voice_note(duration_ms)
        {
            self.capture.buffer.voice = None;
            let decision = decide(&self.state, Event::VoiceTooShort(short), now())?;
            self.store_actor.snapshot(&decision.state).await?;
            self.state = decision.state;
            return Ok(());
        }
        // A call turn begun while Deimos answered is heard before it
        // interrupts him: its words are asked for first, and until they come
        // nothing is stopped, stored or shown (`hear_over_answer`).
        if !cancelled
            && self.take_capture_over_answer()
            && self.state.connection() == orbit_core::Connection::Online
            && let Some(token) = self.token.clone()
            && let Some(voice) = self.capture.buffer.voice.take()
        {
            return self.hear_over_answer(voice, duration_ms, token).await;
        }
        let (source, metadata, wav) = voice_turn_parts(
            self.capture
                .buffer
                .voice
                .as_ref()
                .context("Fuente de voz perdida")?,
            id(),
            duration_ms,
            self.state.session_id().clone(),
        )?;
        if cancelled {
            return self.seal_cancelled_voice_turn(&source, metadata).await;
        }
        self.store_and_send_voice_turn(source, metadata, wav).await
    }

    /// A voice turn the user's stop or a closure cancelled: kept on this Mac
    /// as cancelled, never sent. Privacy/shutdown keeps the established retry
    /// queue: if the destination is unavailable, this PCM remains owned there
    /// until an authenticated WAV receipt exists.
    async fn seal_cancelled_voice_turn(
        &mut self,
        source: &VoiceSource,
        metadata: ChunkMetadata,
    ) -> Result<()> {
        self.enqueue_voice_manifest(source, &metadata, true).await?;
        let voice = self
            .capture
            .buffer
            .voice
            .take()
            .context("Fuente de voz perdida")?;
        self.seal(vec![crate::capture::Segment {
            metadata,
            pcm: voice.pcm,
        }])?;
        let job_id = format!("upload-{}", source.chunk_id);
        self.store_actor
            .run(move |store| Ok(store.update_job(&job_id, JobState::Cancelled)?))
            .await?;
        self.say(NoticeCode::VoiceTurnCancelled);
        Ok(())
    }

    /// A submitted voice turn: stored on this Mac, then the note of the open
    /// turn and sent, or kept waiting with why.
    async fn store_and_send_voice_turn(
        &mut self,
        source: VoiceSource,
        metadata: ChunkMetadata,
        wav: Vec<u8>,
    ) -> Result<()> {
        // Keep the original PCM owned by the capture buffer until the encrypted
        // WAV and its authenticated receipt are durable. The returned WAV is
        // the exact same bounded value later sent to STT.
        let wav = self.persist_voice_turn(&source, metadata, wav).await?;
        // The WAV and its receipt are durable: from here they are the note's
        // copy, and the PCM in memory is never sealed a second time (a later
        // stop once stored it again, under another id, as cancelled).
        let voice = self.capture.buffer.voice.take();
        let decision = decide(
            &self.state,
            Event::VoiceStored {
                id: source.message_id.clone(),
                duration_ms: source.duration_ms,
            },
            now(),
        )?;
        self.store_actor.snapshot(&decision.state).await?;
        self.state = decision.state;
        let in_turn = self
            .state
            .durable
            .turn
            .as_ref()
            .is_some_and(|turn| turn.user.id == source.message_id);
        if !in_turn {
            // Another turn was in flight: the note waits on the Mac and is
            // sent when that turn ends (`send_waiting_voice_note`).
            return Ok(());
        }
        if let Some(voice) = voice {
            self.keep_submitted_call_turn(&source.message_id, voice);
        }
        if self.state.connection() != orbit_core::Connection::Online && self.token.is_some() {
            // Spoken with the hub away: kept and shown, sent when it is back.
            return self
                .keep_voice_unsent(source.message_id, orbit_core::VOICE_HUB_UNREACHABLE)
                .await;
        }
        let Some(token) = self.token.clone() else {
            self.keep_voice_unsent(source.message_id, orbit_core::VOICE_PAIRING_REQUIRED)
                .await?;
            return Err(orbit_core::refusal::Refusal::NotPaired.into());
        };
        self.spawn_voice_transcription(source, wav, token, true)
            .await
    }

    /// The note `message_id` stays on this Mac unsent, saying why (`code`).
    async fn keep_voice_unsent(&mut self, message_id: String, code: &str) -> Result<()> {
        let decision = decide(
            &self.state,
            Event::VoiceFailed {
                id: message_id,
                failure: orbit_core::VoiceTranscriptionFailure::local(code),
            },
            now(),
        )?;
        self.store_actor.snapshot(&decision.state).await?;
        self.work.durable_snapshot = crate::daemon::DurableSnapshot::from(&decision.state);
        self.state = decision.state;
        Ok(())
    }

    /// The voice turn's manifest, before its first blob exists: orphan
    /// recovery may reconstruct its upload job, but must never override this
    /// quarantine. Until it succeeds the PCM stays owned in RAM.
    async fn enqueue_voice_manifest(
        &mut self,
        source: &VoiceSource,
        metadata: &ChunkMetadata,
        cancelled: bool,
    ) -> Result<()> {
        let job = Job {
            id: format!("recording-{}", source.recording_id),
            kind: orbit_store::QueueKind::RecordingManifest.into(),
            payload: json!({"recording_id":source.recording_id,
                "recording_kind":"voice_turn", "session_id":source.session_id,
                "call_id":self.state.durable.call.as_ref().map(|call| &call.id),
                "message_id":source.message_id, "source_chunk_id":source.chunk_id,
                // A voice note goes to `/v1/audio/transcribe` and
                // stays on this Mac for playback; nothing on the hub reads
                // its audio, which it listed as a meeting never closed.
                "source_metadata":metadata, "upload_allowed":false,
                "voice_state":if cancelled {"cancelled"} else {"submitted"},
                "capture_complete":!cancelled,
                "local_audio_receipt_required":true,
                "ended_at_ms":now()}),
            // A durable manifest is not proof of audio durability or STT success:
            // the authenticated chunk receipt is the separate audio boundary.
            state: JobState::Succeeded,
            created_at: now(),
        };
        self.store_actor
            .run(move |store| Ok(store.enqueue_job(&job)?))
            .await
    }

    /// A submitted voice turn's manifest and encrypted WAV, durable before
    /// its transcription is asked for; the WAV is returned to be sent.
    async fn persist_voice_turn(
        &mut self,
        source: &VoiceSource,
        metadata: ChunkMetadata,
        wav: Vec<u8>,
    ) -> Result<Vec<u8>> {
        self.enqueue_voice_manifest(source, &metadata, false)
            .await?;
        let upload = format!("upload-{}", source.chunk_id);
        self.store_actor
            .run(move |store| {
                store.seal_chunk(&metadata, &wav)?;
                store.update_job(&upload, JobState::Cancelled)?;
                Ok(wav)
            })
            .await
    }

    /// A call turn spoken over Deimos's answer: its words are asked for from
    /// the audio in memory while he goes on. Nothing is interrupted, stored
    /// or shown yet: noise, a cough or his own voice coming back would
    /// otherwise cut him off and leave a note that never transcribes
    /// (2026-10-23, Orbit 0.11.0: three such clips of 1–2 s, each refused by
    /// the provider, stayed as failed notes). The words, when they come,
    /// interrupt him and become the turn (`accept_over_answer`); no words
    /// leave nothing. A crash before they come loses only this turn, whose
    /// audio was never the user's message.
    async fn hear_over_answer(
        &mut self,
        voice: crate::capture::VoiceBuffer,
        duration_ms: u64,
        token: String,
    ) -> Result<()> {
        let operation_id = id();
        let wav = native::wav(&voice.pcm, voice.sample_rate, voice.channels)?;
        let source = VoiceSource {
            chunk_id: format!("{operation_id}-mic_local-0"),
            recording_id: operation_id.clone(),
            session_id: self.state.session_id().clone(),
            message_id: operation_id.clone(),
            duration_ms,
        };
        self.keep_over_answer_turn(&operation_id, duration_ms, voice);
        // Said nothing while it is heard: it may be noise.
        self.spawn_voice_transcription(source, wav, token, false)
            .await
    }

    /// The words of a turn spoken over Deimos's answer came: now it stops
    /// him, and is stored and sent as a turn of its own. While the user is
    /// already saying more, it goes first in that turn instead.
    pub(super) async fn accept_over_answer(
        &mut self,
        turn: super::call::OverAnswerTurn,
        failure: Option<orbit_core::VoiceTranscriptionFailure>,
    ) -> Result<bool> {
        if failure.is_none() {
            self.interrupt_for_accepted_turn().await?;
            if self.capture.buffer.voice.is_some() {
                self.carry_into_next_turn(super::call::call_turn_audio(
                    turn.operation_id,
                    turn.voice,
                ));
                return Ok(false);
            }
        }
        let (source, metadata, wav) = voice_turn_parts(
            &turn.voice,
            turn.operation_id,
            turn.duration_ms,
            self.state.session_id().clone(),
        )?;
        self.persist_voice_turn(&source, metadata, wav).await?;
        if failure.is_none() {
            self.apply(Event::CallTurnStarted).await?;
        }
        self.apply(Event::VoiceStored {
            id: source.message_id.clone(),
            duration_ms: source.duration_ms,
        })
        .await?;
        if let Some(failure) = failure {
            // Not heard, and not lost: kept as a note that says why.
            self.apply(Event::VoiceFailed {
                id: source.message_id,
                failure,
            })
            .await?;
            self.store_actor.snapshot(&self.state).await?;
        }
        Ok(true)
    }

    /// Sends one voice note's WAV to the hub on the single voice worker; its
    /// result returns as a work event for that note.
    pub(super) async fn spawn_voice_transcription(
        &mut self,
        source: VoiceSource,
        wav: Vec<u8>,
        token: String,
        announce: bool,
    ) -> Result<()> {
        let settings = self.settings.clone();
        let development = self.development;
        let generation = self.state.generation();
        let events = self.work.work_events.clone();
        if announce {
            self.say(NoticeCode::VoiceTranscribing);
        }
        let operation_id = source.message_id.clone();
        self.conversation.voice_operation = Some(operation_id.clone());
        let worker_operation_id = operation_id.clone();
        // One voice turn at a time; the guard above keeps a second turn out.
        // It is not a limit on stored voice messages.
        let started = self.conversation.voice_task.start_logical(
            generation,
            self.turn.op.clone(),
            (),
            move |ticket| {
                tokio::spawn(async move {
                    let event =
                        match hub::transcribe(&settings, &token, development, wav, ticket.op())
                            .await
                        {
                            Ok(text) => {
                                WorkResult::Capture(crate::daemon::CaptureWork::Transcribed {
                                    ticket,
                                    source,
                                    text,
                                })
                            }
                            Err(failure) => {
                                WorkResult::Capture(crate::daemon::CaptureWork::VoiceProblem {
                                    ticket,
                                    operation_id: worker_operation_id,
                                    failure,
                                })
                            }
                        };
                    crate::daemon::deliver_work(&events, event).await;
                })
            },
        );
        if started.is_none() {
            self.conversation.voice_operation = None;
            let decision = decide(
                &self.state,
                Event::VoiceFailed {
                    id: operation_id,
                    failure: orbit_core::VoiceTranscriptionFailure::local(
                        "voice_transcription_slot_unavailable",
                    ),
                },
                now(),
            )?;
            self.store_actor.snapshot(&decision.state).await?;
            self.work.durable_snapshot = crate::daemon::DurableSnapshot::from(&decision.state);
            self.state = decision.state;
            // One slot protects the active voice transcription worker: a second
            // note cannot replace its receipt. This is not an audio size limit.
            return Err(orbit_core::refusal::Refusal::WorkInFlight {
                in_flight: 1,
                max: 1,
            }
            .into());
        }
        Ok(())
    }
}

/// A voice buffer's length in ms, from its PCM16 bytes.
pub(super) fn voice_duration_ms(voice: &crate::capture::VoiceBuffer) -> u64 {
    (voice.pcm.len() as u64)
        .saturating_div(u64::from(voice.channels).saturating_mul(2))
        .saturating_mul(1_000)
        .saturating_div(u64::from(voice.sample_rate))
}

/// The identity, chunk receipt facts and WAV of a voice turn `message_id`:
/// its recording is its message's id, and it is one chunk.
fn voice_turn_parts(
    voice: &crate::capture::VoiceBuffer,
    message_id: String,
    duration_ms: u64,
    session_id: Option<String>,
) -> Result<(VoiceSource, ChunkMetadata, Vec<u8>)> {
    let wav = native::wav(&voice.pcm, voice.sample_rate, voice.channels)?;
    let recording_id = message_id.clone();
    let source = VoiceSource {
        chunk_id: format!("{recording_id}-mic_local-0"),
        recording_id,
        session_id,
        message_id,
        duration_ms,
    };
    let metadata = ChunkMetadata {
        capture_epoch_id: voice.capture_epoch_id.clone(),
        id: source.chunk_id.clone(),
        recording_id: source.recording_id.clone(),
        track: "mic_local".into(),
        sequence: 0,
        sample_rate: voice.sample_rate,
        channels: voice.channels,
        frames: voice.pcm.len() as u64 / (u64::from(voice.channels) * 2),
        monotonic_ns: voice.monotonic_ns,
        first_sample: 0,
        started_at_ms: voice.started_at_ms,
    };
    Ok((source, metadata, wav))
}
