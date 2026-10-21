use super::{Runtime, WorkResult, id};

impl Runtime {
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
                        id(),
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
}
