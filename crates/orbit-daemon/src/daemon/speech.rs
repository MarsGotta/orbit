use super::{Runtime, WorkResult, id};
use anyhow::{Result, bail};
use orbit_core::refusal::{Expected, Parameter, Refusal};
use serde_json::json;

/// The answer read aloud at the user's request, and the playback that
/// carries it: the helper names that playback when it ends.
#[derive(Debug, Clone)]
pub(super) struct ReadAloud {
    pub(super) message_id: String,
    pub(super) playback_id: String,
}

impl Runtime {
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
            bail!("Mientras se graba una reunión, Orbit no lee respuestas en voz alta");
        }
        if self.state.durable.call.is_some() {
            bail!("Durante una llamada, Orbit no lee otras respuestas en voz alta");
        }
        if self.token.is_none() {
            return Err(Refusal::NotPaired.into());
        }
        let playback_id = id();
        self.conversation.read_aloud = Some(ReadAloud {
            message_id,
            playback_id: playback_id.clone(),
        });
        self.play_speech(text, playback_id);
        Ok(())
    }

    /// «Parar»: what is being read stops, in the hub fetch or in the helper.
    pub(super) async fn stop_speaking(&mut self) -> Result<()> {
        if self.conversation.read_aloud.take().is_none() {
            return Ok(());
        }
        self.conversation.speech_task.cancel();
        self.native_send("playback_stop", json!({})).await
    }

    /// The helper says a playback ended, played or cancelled.
    pub(super) fn playback_ended(&mut self, playback_id: &str) {
        if self
            .conversation
            .read_aloud
            .as_ref()
            .is_some_and(|read| read.playback_id == playback_id)
        {
            self.conversation.read_aloud = None;
        }
    }

    /// Replay the last assistant turn locally. A replay replaces the
    /// playback in flight; the replaced turn's answer arrives foreign and is
    /// discarded. It is not a queue of turns to read.
    pub(super) fn start_speech(&mut self) {
        let requested =
            std::mem::take(&mut self.conversation.speak_next) || self.settings.speak_responses;
        // During meetings the default response stays private in the terminal.
        if !requested
            || self.recording_active()
            || self.state.privacy_stopped()
            || !self.state.capabilities().native_helper()
        {
            return;
        }
        let Some(text) = self
            .state
            .durable
            .turn
            .as_ref()
            .and_then(|turn| turn.answer.as_ref())
            .map(|answer| answer.text.clone())
        else {
            return;
        };
        // The turn's own voice replaces an answer the user was hearing.
        self.conversation.read_aloud = None;
        self.play_speech(text, id());
    }

    /// Fetches `text` in the hub's voice and hands it to the helper as
    /// `playback_id`, replacing the speech in flight.
    fn play_speech(&mut self, text: String, playback_id: String) {
        let Some(token) = self.token.clone() else {
            return;
        };
        let settings = self.settings.clone();
        let development = self.development;
        let native_tx = self.native.clone();
        let events = self.work.work_events.clone();
        let generation = self.state.generation();
        // A replay request does not erase a known capability. The worker's
        // result may update it; until then the last observed fact still holds.
        self.conversation.speech_task.replace_logical(
            generation,
            self.turn.op.clone(),
            (),
            move |ticket| {
                tokio::spawn(async move {
                    let result = crate::speech::play(
                        settings,
                        token,
                        development,
                        text,
                        playback_id,
                        native_tx,
                        ticket.op().clone(),
                    )
                    .await;
                    let event = match result {
                        Ok(()) => WorkResult::SpeechAvailability {
                            ticket,
                            available: true,
                        },
                        Err(error)
                            if error
                                .downcast_ref::<crate::speech::SpeechUnavailable>()
                                .is_some() =>
                        {
                            WorkResult::SpeechAvailability {
                                ticket,
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

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
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
        runtime.state.durable.turn = Some(Turn {
            user: Message {
                id: "synthetic".into(),
                role: "user".into(),
                text: "Pregunta de prueba".into(),
                complete: true,
                at_ms: 1,
                voice: None,
            },
            answer: Some(Message {
                id: "synthetic:assistant".into(),
                role: "assistant".into(),
                text: "Texto de prueba".into(),
                complete: true,
                at_ms: 2,
                voice: None,
            }),
        });
        runtime.token = Some("synthetic-token".into());
        runtime.conversation.speak_next = true;
        runtime.conversation.speech_available = Some(false);
        runtime.start_speech();
        assert!(runtime.conversation.speech_task.active().is_some());
        assert_eq!(runtime.conversation.speech_available, Some(false));
        if let Some(task) = runtime.conversation.speech_task.take() {
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
        runtime.token = Some("synthetic-not-a-credential-speak".into());
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
        assert!(runtime.conversation.speech_task.active().is_some());
        let playback = runtime.conversation.read_aloud.clone().unwrap().playback_id;
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
        assert!(runtime.conversation.speech_task.active().is_none());
        let mut orders = Vec::new();
        while let Ok(order) = native.try_recv() {
            orders.push(order["command"].as_str().unwrap_or_default().to_owned());
        }
        assert_eq!(orders, ["playback_stop"]);
        runtime.command(Command::SpeakStop).await.unwrap();
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
        runtime.state.durable.call = Some(orbit_core::CallState {
            id: "call-speak".into(),
            phase: orbit_core::CallPhase::Listening,
            started_at_ms: 1,
            expires_at_ms: 60_001,
            turns: 0,
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
            "Durante una llamada, Orbit no lee otras respuestas en voz alta"
        );
        assert_eq!(speaking(&runtime), serde_json::Value::Null);
    }
}
