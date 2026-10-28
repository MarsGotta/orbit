use super::{Runtime, WorkResult, id};
use anyhow::{Result, bail};
use orbit_core::refusal::{Expected, Parameter, Refusal};
use serde_json::{Value, json};

/// The answer read aloud, at the user's request or by itself after a voice
/// note, and the playback that carries it: the helper names that playback
/// when it ends.
#[derive(Debug, Clone)]
pub(super) struct ReadAloud {
    pub(super) message_id: String,
    pub(super) playback_id: String,
    remaining: std::collections::VecDeque<String>,
    clip_done: bool,
    current_text: String,
}

impl Runtime {
    /// The helper says how a playback goes; one that ended frees its
    /// reading.
    pub(super) fn on_playback_progress(&mut self, event: Value) -> Result<()> {
        let decoded: orbit_core::wire::native::NativeEvent = serde_json::from_value(event)?;
        let orbit_core::wire::native::NativeEvent::PlaybackProgress {
            playback_id,
            state,
            playback_active,
            ..
        } = decoded
        else {
            bail!("Evento de progreso nativo incoherente");
        };
        let state = match state.as_str() {
            "started" => "started",
            "played" => "played",
            "cancelled" => "cancelled",
            "paused" => "paused",
            "resumed" => "resumed",
            _ => "unrecognized_state",
        };
        if matches!(state, "played" | "cancelled") {
            if state == "played"
                && let Some(read) = self.assistant.read_aloud.as_mut()
                && read.playback_id == playback_id
            {
                read.clip_done = true;
                self.advance_reading();
            } else {
                self.assistant.playback_ended(&playback_id);
            }
        }
        self.log(
            "native_playback_progress",
            &[
                ("state", state),
                ("active", if playback_active { "true" } else { "false" }),
            ],
        );
        Ok(())
    }

    /// Nothing plays any more, and nothing waits to be read next.
    pub(super) async fn stop_playback(&mut self) -> Result<()> {
        self.assistant.speech_task.cancel();
        self.assistant.speak_next = false;
        self.assistant.speech_paused = false;
        self.assistant.read_aloud = None;
        self.native_send("playback_stop", json!({})).await
    }

    /// «Escuchar»: the hub's voice reads `text`, the answer `message_id`.
    /// A new one replaces the one being read (the helper stops a playback
    /// when another one starts). Each refusal names its cause.
    pub(super) fn speak(&mut self, message_id: String, text: String) -> Result<()> {
        if self.state.privacy_stopped() {
            return Err(Refusal::PrivacyStopped { since_ms: None }.into());
        }
        if !self.state.capabilities().native_helper() {
            return Err(Refusal::HelperNotRunning.into());
        }
        // The same bound as a chat message, and the one `speech::play` keeps.
        if text.trim().is_empty() || text.len() > orbit_core::MAX_TEXT_BYTES {
            return Err(Refusal::InvalidParameter {
                parameter: Parameter::Text,
                expected: Expected::TextLength {
                    bytes: text.len() as u64,
                    max_bytes: orbit_core::MAX_TEXT_BYTES as u64,
                },
            }
            .into());
        }
        if self.recording_active() {
            bail!("Orbit does not read responses aloud while recording a meeting");
        }
        if self.state.durable.voice.call.is_some() {
            bail!("Orbit does not read other responses aloud during a call");
        }
        if self.link.token.is_none() {
            return Err(Refusal::NotPaired.into());
        }
        self.begin_reading(message_id, text);
        Ok(())
    }

    /// «Parar»: what is being read stops, in the hub fetch or in the helper.
    pub(super) async fn stop_speaking(&mut self) -> Result<()> {
        if self.assistant.read_aloud.take().is_none() {
            return Ok(());
        }
        self.assistant.speech_task.cancel();
        self.assistant.speech_paused = false;
        self.native_send("playback_stop", json!({})).await
    }

    /// Replay the last assistant turn locally. A replay replaces the
    /// playback in flight; the replaced turn's answer arrives foreign and is
    /// discarded. It is not a queue of turns to read.
    pub(super) fn start_speech(&mut self) {
        let requested =
            std::mem::take(&mut self.assistant.speak_next) || self.settings.speak_responses;
        // During meetings the default response stays private in the terminal.
        if !requested
            || self.recording_active()
            || self.state.privacy_stopped()
            || !self.state.capabilities().native_helper()
        {
            return;
        }
        let Some((message_id, text)) = self
            .state
            .durable
            .assistant
            .turn
            .as_ref()
            .and_then(|turn| turn.answer.as_ref())
            .map(|answer| (answer.id.clone(), answer.text.clone()))
        else {
            return;
        };
        // The turn's own voice replaces an answer the user was hearing, and
        // is named like one: «Escuchar» says «Parar» while it sounds.
        self.begin_reading(message_id, text);
    }

    pub(super) fn ask_call_presence(&mut self) {
        let voice = self.reading_voice();
        let available = self.settings.hub_agents.as_ref().is_none_or(|roster| {
            roster
                .agent(&voice)
                .is_none_or(|agent| agent.voice != Some(false))
        });
        if available
            && self.link.token.is_some()
            && self.state.connection() == orbit_core::Connection::Online
        {
            self.begin_reading(
                "call-presence".into(),
                "¿Sigues ahí? Si no dices nada, cuelgo en 15 segundos".into(),
            );
        }
    }

    fn begin_reading(&mut self, message_id: String, text: String) {
        let mut remaining = orbit_core::speech::parts(&text);
        let Some(first) = remaining.pop_front() else {
            return;
        };
        let playback_id = id();
        self.assistant.read_aloud = Some(ReadAloud {
            message_id,
            playback_id: playback_id.clone(),
            remaining,
            clip_done: false,
            current_text: first.clone(),
        });
        self.play_speech(first, playback_id);
    }

    /// Only an actual played receipt advances the bounded queue. A paused
    /// reading may finish at the same instant as onset; it waits for the verdict.
    fn advance_reading(&mut self) {
        if self.assistant.speech_paused {
            return;
        }
        let Some(read) = self
            .assistant
            .read_aloud
            .as_mut()
            .filter(|read| read.clip_done)
        else {
            return;
        };
        if let Some(text) = read.remaining.pop_front() {
            let playback_id = id();
            read.playback_id = playback_id.clone();
            read.clip_done = false;
            read.current_text = text.clone();
            self.play_speech(text, playback_id);
        } else {
            self.assistant.read_aloud = None;
        }
    }

    /// The decoder refused this clip before playing it. An unusually slow
    /// provider is split further; every other failure releases the queue.
    pub(super) fn speech_clip_refused(&mut self, command_id: &str, code: &str) -> bool {
        let Some(read) = self
            .assistant
            .read_aloud
            .as_mut()
            .filter(|read| read.playback_id == command_id)
        else {
            return false;
        };
        if code == "clip_too_long"
            && let Some(mut shorter) = orbit_core::speech::shorter_parts(&read.current_text)
        {
            shorter.append(&mut read.remaining);
            read.remaining = shorter;
            read.clip_done = true;
            self.advance_reading();
            return true;
        }
        self.assistant.read_aloud = None;
        self.assistant.speech_task.cancel();
        false
    }

    /// Tentative sound pauses only; it never cancels a reading or a request.
    pub(super) async fn pause_call_reading(&mut self) -> Result<()> {
        if !self.assistant.speech_paused {
            self.native_send("playback_pause", json!({})).await?;
            self.assistant.speech_paused = true;
        }
        Ok(())
    }

    /// An empty or failed transcription lets the same scheduled clip continue.
    pub(super) async fn resume_call_reading(&mut self) -> Result<()> {
        let capturing = self.capture.buffer.voice.is_some();
        if self.assistant.speech_paused && !capturing {
            self.native_send("playback_resume", json!({})).await?;
            self.assistant.speech_paused = false;
            self.advance_reading();
        }
        Ok(())
    }

    /// Fetches `text` in the hub's voice and hands it to the helper as
    /// `playback_id`, replacing the speech in flight.
    fn play_speech(&mut self, text: String, playback_id: String) {
        let Some(token) = self.link.token.clone() else {
            return;
        };
        let settings = self.settings.clone();
        let development = self.development;
        let events = self.work_events.clone();
        let generation = self.state.generation();
        // The voice of the agent the answer is from: the open conversation's.
        let agent_id = self.turn_agent_id();
        // What the result says is remembered for the voice that reads it.
        let voice = self.reading_voice();
        // A replay request does not erase a known capability. The worker's
        // result may update it; until then the last observed fact still holds.
        self.assistant.speech_task.replace_logical(
            generation,
            self.turn.op.clone(),
            (),
            move |ticket| {
                tokio::spawn(async move {
                    // One clip belongs to this ticket, never directly to the
                    // helper queue. The loop checks the ticket before writing.
                    let (native_tx, mut clips) = tokio::sync::mpsc::channel(1);
                    let result = crate::speech::play(
                        settings,
                        token,
                        development,
                        text,
                        agent_id,
                        playback_id,
                        native_tx,
                        ticket.op().clone(),
                    )
                    .await;
                    let event = match result {
                        Ok(()) => match clips.try_recv() {
                            Ok(command) => WorkResult::SpeechClip {
                                ticket,
                                voice,
                                command,
                            },
                            Err(_) => WorkResult::Problem {
                                ticket,
                                message: "The speech service did not deliver a clip".into(),
                            },
                        },
                        Err(error)
                            if error
                                .downcast_ref::<crate::speech::SpeechUnavailable>()
                                .is_some() =>
                        {
                            WorkResult::SpeechAvailability {
                                ticket,
                                voice,
                                available: false,
                            }
                        }
                        Err(error) => WorkResult::Problem {
                            ticket,
                            message: error.to_string(),
                        },
                    };
                    crate::daemon::deliver_work(&events, event).await;
                })
            },
        );
    }
}

impl crate::daemon::assistant::AssistantHandle {
    /// The helper says a playback ended, played or cancelled.
    pub(super) fn playback_ended(&mut self, playback_id: &str) {
        if self
            .read_aloud
            .as_ref()
            .is_some_and(|read| read.playback_id == playback_id)
        {
            self.read_aloud = None;
        }
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::WorkResult;
    use orbit_core::{Message, Turn};

    #[tokio::test]
    async fn a_known_voice_absence_stays_known_while_replay_starts() {
        let directory = tempfile::tempdir().unwrap();
        let (mut runtime, _native) = super::super::audio::tests::fixture(directory.path()).unwrap();
        {
            let fixture_value = serde_json::json!(true);
            crate::daemon::authority_fixture::set(
                &mut runtime.state,
                "capabilities.native_helper",
                fixture_value,
            );
        }
        runtime.state.durable.assistant.turn = Some(Turn {
            user: Message {
                attachments: Vec::new(),
                id: "synthetic".into(),
                role: "user".into(),
                text: "Pregunta de prueba".into(),
                complete: true,
                at_ms: 1,
                voice: None,
            },
            answer: Some(Message {
                attachments: Vec::new(),
                id: "synthetic:assistant".into(),
                role: "assistant".into(),
                text: "Texto de prueba".into(),
                complete: true,
                at_ms: 2,
                voice: None,
            }),
        });
        runtime.link.token = Some("synthetic-token".into());
        runtime.assistant.speak_next = true;
        let voice = runtime.reading_voice();
        runtime.assistant.speech_voices.insert(voice.clone(), false);
        runtime.start_speech();
        assert!(runtime.assistant.speech_task.active().is_some());
        assert_eq!(runtime.assistant.speech_voices.get(&voice), Some(&false));
        if let Some(task) = runtime.assistant.speech_task.take() {
            task.abort();
        }
    }

    use crate::daemon::Runtime;
    use orbit_core::protocol::Command;
    use orbit_core::refusal::{Expected, Parameter, Refusal};
    use serde_json::json;

    fn speaking(runtime: &Runtime) -> serde_json::Value {
        let mut board = crate::daemon::topics::TopicBoard::new("synthetic".into());
        let channels = board.channels();
        let assistant = orbit_core::topics::TopicId::Assistant;
        let _receiver = channels[&assistant].subscribe();
        board.publish(&crate::daemon::topics::TopicSource {
            state: &runtime.state,
            runtime: Some(runtime),
            now_ms: 0,
        });
        let data = serde_json::to_value(channels[&assistant].borrow().data.clone()).unwrap();
        data["knowledge"]["value"]["speaking"].clone()
    }

    fn speakable(
        directory: &std::path::Path,
    ) -> (Runtime, tokio::sync::mpsc::Receiver<serde_json::Value>) {
        let (mut runtime, native) = super::super::audio::tests::fixture(directory).unwrap();
        runtime.link.token = Some("synthetic-not-a-credential-speak".into());
        // Nothing listens there: the fetch fails, which these tests do not read.
        runtime.settings.hub_url = Some("http://127.0.0.1:9/".into());
        (runtime, native)
    }

    fn progress(playback_id: &str, state: &str) -> serde_json::Value {
        json!({
            "v": 1, "event": "playback_progress", "playback_id": playback_id,
            "state": state, "playback_active": false
        })
    }

    #[tokio::test]
    async fn the_answer_read_aloud_is_named_until_the_helper_ends_its_playback() {
        let directory = tempfile::tempdir().unwrap();
        let (mut runtime, _native) = speakable(directory.path());
        assert_eq!(speaking(&runtime), serde_json::Value::Null);
        runtime
            .command(Command::Speak {
                message_id: "answer-1".into(),
                text: "La sala Norte tiene 8 plazas.".into(),
            })
            .await
            .unwrap();
        assert_eq!(speaking(&runtime), "answer-1");
        assert!(runtime.assistant.speech_task.active().is_some());
        let playback = runtime.assistant.read_aloud.clone().unwrap().playback_id;
        runtime
            .native_event(progress(&playback, "started"))
            .await
            .unwrap();
        runtime
            .native_event(progress("another-playback", "played"))
            .await
            .unwrap();
        assert_eq!(
            speaking(&runtime),
            "answer-1",
            "another playback ending ends nothing here"
        );
        runtime
            .native_event(progress(&playback, "played"))
            .await
            .unwrap();
        assert_eq!(speaking(&runtime), serde_json::Value::Null);
    }

    /// After a voice note Orbit reads the answer by itself: «Escuchar» says
    /// «Parar» for it as for one the user asked to hear, and «Parar» stops it.
    #[tokio::test]
    async fn the_answer_read_by_itself_is_named_until_it_ends_or_is_stopped() {
        let directory = tempfile::tempdir().unwrap();
        let (mut runtime, mut native) = speakable(directory.path());
        crate::daemon::authority_fixture::set(
            &mut runtime.state,
            "capabilities.native_helper",
            json!(true),
        );
        let answer = |text: &str| orbit_core::Message {
            attachments: Vec::new(),
            id: "nota-1:assistant".into(),
            role: "assistant".into(),
            text: text.into(),
            complete: true,
            at_ms: 2,
            voice: None,
        };
        runtime.state.durable.assistant.turn = Some(orbit_core::Turn {
            user: orbit_core::Message {
                role: "user".into(),
                id: "nota-1".into(),
                ..answer("Lo que dije")
            },
            answer: Some(answer("La sala Norte tiene 8 plazas.")),
        });
        for stop in [false, true] {
            runtime.assistant.speak_next = true;
            runtime.start_speech();
            assert_eq!(speaking(&runtime), "nota-1:assistant");
            let playback = runtime.assistant.read_aloud.clone().unwrap().playback_id;
            if stop {
                runtime.command(Command::SpeakStop).await.unwrap();
                let order = native.try_recv().unwrap();
                assert_eq!(order["command"], "playback_stop");
            } else {
                runtime
                    .native_event(progress(&playback, "played"))
                    .await
                    .unwrap();
            }
            assert_eq!(speaking(&runtime), serde_json::Value::Null);
        }
    }

    #[tokio::test]
    async fn a_second_answer_replaces_the_first_and_stop_silences_the_helper() {
        let directory = tempfile::tempdir().unwrap();
        let (mut runtime, mut native) = speakable(directory.path());
        for id in ["answer-1", "answer-2"] {
            runtime
                .command(Command::Speak {
                    message_id: id.into(),
                    text: "Texto".into(),
                })
                .await
                .unwrap();
        }
        assert_eq!(speaking(&runtime), "answer-2");
        runtime.command(Command::SpeakStop).await.unwrap();
        assert_eq!(speaking(&runtime), serde_json::Value::Null);
        assert!(runtime.assistant.speech_task.active().is_none());
        let mut orders = Vec::new();
        while let Ok(order) = native.try_recv() {
            orders.push(order["command"].as_str().unwrap_or_default().to_owned());
        }
        assert_eq!(orders, ["playback_stop"]);
        runtime.command(Command::SpeakStop).await.unwrap();
    }

    #[tokio::test]
    async fn a_long_reading_continues_after_its_first_clip() {
        let directory = tempfile::tempdir().unwrap();
        let (mut runtime, _native) = speakable(directory.path());
        runtime
            .command(Command::Speak {
                message_id: "long-answer".into(),
                text: "Una frase sintética sobre la sala Norte. ".repeat(90),
            })
            .await
            .unwrap();
        let first = runtime.assistant.read_aloud.clone().unwrap().playback_id;
        runtime
            .native_event(progress(&first, "played"))
            .await
            .unwrap();
        assert_eq!(
            speaking(&runtime),
            "long-answer",
            "only the first clip ended"
        );
        let next = runtime.assistant.read_aloud.clone().unwrap().playback_id;
        assert_ne!(next, first);
        runtime.command(Command::SpeakStop).await.unwrap();
        runtime
            .native_event(progress(&next, "played"))
            .await
            .unwrap();
        assert_eq!(speaking(&runtime), serde_json::Value::Null);
    }

    #[tokio::test]
    async fn a_synthesized_clip_whose_ticket_was_cancelled_never_reaches_the_helper() {
        let directory = tempfile::tempdir().unwrap();
        let (mut runtime, mut native) = speakable(directory.path());
        runtime
            .command(Command::Speak {
                message_id: "late-answer".into(),
                text: "La sala Norte está lista.".into(),
            })
            .await
            .unwrap();
        let ticket = runtime.assistant.speech_task.ticket().cloned().unwrap();
        runtime.command(Command::SpeakStop).await.unwrap();
        assert_eq!(native.recv().await.unwrap()["command"], "playback_stop");
        runtime
            .work_event(WorkResult::SpeechClip {
                ticket,
                voice: "atlas".into(),
                command: crate::native::command(
                    "playback_clip",
                    json!({
                        "playback_id":"late-clip", "mime_type":"audio/wav", "provider":"synthetic",
                        "audio_base64":"AQI="
                    }),
                )
                .unwrap(),
            })
            .await
            .unwrap();
        assert!(native.try_recv().is_err());
        assert!(runtime.assistant.read_aloud.is_none());
    }

    #[tokio::test]
    async fn speak_refuses_with_its_cause_and_its_numbers() {
        let directory = tempfile::tempdir().unwrap();
        let (mut runtime, _native) = speakable(directory.path());
        let text = "x".repeat(orbit_core::MAX_TEXT_BYTES + 1);
        let error = runtime
            .command(Command::Speak {
                message_id: "long".into(),
                text,
            })
            .await
            .unwrap_err();
        assert_eq!(
            error.downcast_ref::<Refusal>(),
            Some(&Refusal::InvalidParameter {
                parameter: Parameter::Text,
                expected: Expected::TextLength {
                    bytes: orbit_core::MAX_TEXT_BYTES as u64 + 1,
                    max_bytes: orbit_core::MAX_TEXT_BYTES as u64,
                },
            })
        );
        crate::daemon::authority_fixture::set(&mut runtime.state, "privacy_stopped", json!(true));
        let error = runtime
            .command(Command::Speak {
                message_id: "stopped".into(),
                text: "Texto".into(),
            })
            .await
            .unwrap_err();
        assert!(
            matches!(
                error.downcast_ref::<Refusal>(),
                Some(Refusal::PrivacyStopped { .. })
            ),
            "{error}"
        );
        crate::daemon::authority_fixture::set(&mut runtime.state, "privacy_stopped", json!(false));
        // A call speaks its own answers; another one would talk over it.
        runtime.state.durable.voice.call = Some(orbit_core::CallState {
            id: "call-speak".into(),
            phase: orbit_core::CallPhase::Listening,
            started_at_ms: 1,
            expires_at_ms: 60_001,
            turns: 0,
            origin: orbit_core::CallOrigin::App,
            ceiling_notice_given: false,
        });
        let error = runtime
            .command(Command::Speak {
                message_id: "call".into(),
                text: "Texto".into(),
            })
            .await
            .unwrap_err();
        assert_eq!(
            error.to_string(),
            "Orbit does not read other responses aloud during a call"
        );
        assert_eq!(speaking(&runtime), serde_json::Value::Null);
    }
}
