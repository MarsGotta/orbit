//! Voice notes kept on this Mac: their audio for playback and «Reintentar»,
//! the ones waiting for the hub, «Descartar», and the ones in which nothing
//! was heard, which leave without a trace.
use super::audio::VoiceSource;
use super::{Runtime, now};
use anyhow::{Result, bail};
use base64::{Engine, engine::general_purpose::STANDARD};
use orbit_core::{Event, MAX_VOICE_NOTE_BYTES, decide, protocol::VoiceNoteAudio};

impl Runtime {
    /// A voice note of any conversation, by its message's id, which
    /// is its recording's; the conversation itself is read from the hub.
    pub(super) fn voice_note_audio(&self, message_id: &str) -> Result<VoiceNoteAudio> {
        let Some((_, wav)) = self.stored_voice_note(message_id)? else {
            bail!("voice_note_missing");
        };
        Ok(VoiceNoteAudio {
            message_id: message_id.to_owned(),
            format: "wav".into(),
            bytes_base64: STANDARD.encode(wav),
        })
    }

    /// The chunk id and WAV of the voice note `message_id`, while this Mac
    /// still holds them; what playback and «Reintentar transcripción» read.
    fn stored_voice_note(&self, message_id: &str) -> Result<Option<(String, Vec<u8>)>> {
        let Ok(chunks) = self.reader.list_chunks(message_id, None, 2) else {
            return Ok(None);
        };
        let Some(chunk) = voice_note_chunk(chunks) else {
            return Ok(None);
        };
        if chunk.bytes > MAX_VOICE_NOTE_BYTES as u64 {
            bail!("voice_note_too_large");
        }
        let wav = self.reader.read_chunk(&chunk.metadata.id)?;
        if wav.len() > MAX_VOICE_NOTE_BYTES {
            bail!("voice_note_too_large");
        }
        Ok(Some((chunk.metadata.id, wav)))
    }

    /// «Reintentar transcripción»: the open turn's voice note whose
    /// transcription failed goes again to the hub, from the WAV this Mac
    /// kept. Without that WAV there is nothing to send, and the refusal
    /// says it is the audio that is missing.
    pub(super) async fn retry_voice_transcription(&mut self, message_id: String) -> Result<()> {
        if self.assistant.voice_task.active().is_some() || self.capture.buffer.voice.is_some() {
            return Err(orbit_core::refusal::Refusal::Busy {
                operation: orbit_core::refusal::Operation::Interaction,
                retry_after_ms: None,
            }
            .into());
        }
        let retry = Event::VoiceRetry {
            id: message_id.clone(),
        };
        // The pure decision first, so a note that cannot be retried is
        // refused for that before its audio is looked for.
        decide(&self.state, retry.clone(), now())?;
        let Some(token) = self.link.token.clone() else {
            return Err(orbit_core::refusal::Refusal::NotPaired.into());
        };
        let Some((chunk_id, wav)) = self.stored_voice_note(&message_id)? else {
            return Err(orbit_core::refusal::Refusal::NotFound {
                resource: orbit_core::refusal::Resource::Audio,
            }
            .into());
        };
        let duration_ms = self
            .state
            .durable
            .assistant
            .turn
            .iter()
            .map(|turn| &turn.user)
            .chain(self.state.durable.open_conversation_pending_voice())
            .find(|message| message.id == message_id)
            .and_then(|message| message.voice.as_ref())
            .map_or(0, |voice| voice.duration_ms);
        self.apply(retry).await?;
        let source = VoiceSource {
            recording_id: message_id.clone(),
            chunk_id,
            session_id: self.state.session_id().clone(),
            message_id,
            duration_ms,
        };
        self.spawn_voice_transcription(source, wav, token, true)
            .await
    }

    /// «Descartar», confirmed: the failed voice note `message_id` leaves the
    /// conversation, and its audio this Mac. The decision comes first, so a
    /// note still being transcribed is refused before anything is deleted;
    /// the audio goes before the note, so a failed deletion leaves the note
    /// there to discard again rather than audio nothing names.
    pub(super) async fn discard_voice_note(&mut self, message_id: String) -> Result<()> {
        let discard = Event::VoiceDiscard {
            id: message_id.clone(),
        };
        decide(&self.state, discard.clone(), now())?;
        self.forget_voice_audio(&message_id).await?;
        self.apply(discard).await?;
        // Messages are outside the snapshot's fingerprint.
        self.store_actor.snapshot(&self.state).await?;
        Ok(())
    }

    /// A voice turn left without a trace, counted in the log with why:
    /// `no_words` (the hub heard none) or `joined_next_turn`. Never its words.
    pub(super) fn note_dropped_voice(&mut self, reason: &str) {
        self.assistant.dropped_voice_turns = self.assistant.dropped_voice_turns.saturating_add(1);
        let count = self.assistant.dropped_voice_turns.to_string();
        self.log(
            "voice_turn_dropped",
            &[("reason", reason), ("count", &count)],
        );
    }

    /// The local audio of the voice note `message_id` leaves this Mac.
    pub(super) async fn forget_voice_audio(&mut self, message_id: &str) -> Result<()> {
        let recording = message_id.to_owned();
        self.store_actor
            .run(move |store| Ok(store.discard_voice_note(&recording)?))
            .await?;
        Ok(())
    }

    /// Nothing was heard in the voice note `message_id`: noise, silence, or
    /// what the user said next carries it. It leaves the conversation and
    /// its audio this Mac without a word to the user; the log keeps how
    /// many and why (`reason`), never what.
    pub(super) async fn drop_unheard_voice_note(
        &mut self,
        message_id: &str,
        reason: &str,
    ) -> Result<()> {
        self.note_dropped_voice(reason);
        self.apply(Event::VoiceUnheard {
            id: message_id.to_owned(),
        })
        .await?;
        self.store_actor.snapshot(&self.state).await?;
        if let Err(error) = self.forget_voice_audio(message_id).await {
            self.log(
                "voice_note_audio_discard_failed",
                &[("error", &error.to_string())],
            );
        }
        Ok(())
    }

    /// The hub is back and nothing else is in flight: the oldest voice note
    /// that never reached it (spoken offline, unpaired, or behind another
    /// turn) is sent, one per tick. A note the hub did try waits for
    /// «Reintentar transcripción»: sending it again by itself could loop.
    pub(super) async fn send_waiting_voice_note(&mut self) -> Result<()> {
        let durable = &self.state.durable;
        if self.state.privacy_stopped()
            || self.state.connection() != orbit_core::Connection::Online
            || self.link.token.is_none()
            || durable.assistant.active_request.is_some()
            || durable.voice.voice_active
            || durable.voice.call.is_some()
            || durable.assistant.pending_approval.is_some()
            || self.assistant.voice_task.active().is_some()
            || self.capture.buffer.voice.is_some()
        {
            return Ok(());
        }
        let waits = |message: &&orbit_core::Message| {
            message.voice.as_ref().is_some_and(|voice| {
                voice.transcript.is_none()
                    && voice
                        .failure
                        .as_ref()
                        .is_some_and(|failure| orbit_core::voice_note_waits_for_hub(&failure.code))
            })
        };
        let Some(message_id) = durable
            .assistant
            .turn
            .iter()
            .map(|turn| &turn.user)
            .filter(waits)
            // Only the open conversation's: a note waiting in another is
            // sent when that one is open, never into this one.
            .chain(durable.open_conversation_pending_voice().filter(waits))
            .map(|message| message.id.clone())
            .next()
        else {
            return Ok(());
        };
        self.log("waiting_voice_note_sent", &[]);
        if let Err(error) = self.retry_voice_transcription(message_id.clone()).await {
            // Not sent again by itself: its WAV is gone or the hub refused
            // the retry. It stays listed, with «Reintentar».
            self.log(
                "waiting_voice_note_failed",
                &[("error", &error.to_string())],
            );
            self.apply(Event::VoiceFailed {
                id: message_id,
                failure: orbit_core::VoiceTranscriptionFailure::local("voice_resend_failed"),
            })
            .await?;
            self.store_actor.snapshot(&self.state).await?;
        }
        Ok(())
    }
}

/// The one mono chunk a voice note's recording holds, while this Mac still
/// has it: what `voice_note_audio` plays and what a conversation's page says
/// with `audio_available`.
pub(super) fn voice_note_chunk(
    chunks: Vec<orbit_store::ChunkReceipt>,
) -> Option<orbit_store::ChunkReceipt> {
    match <[orbit_store::ChunkReceipt; 1]>::try_from(chunks) {
        Ok([chunk]) if chunk.metadata.channels == 1 => Some(chunk),
        _ => None,
    }
}
