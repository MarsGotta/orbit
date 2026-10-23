//! «Oye Deimos» during a meeting, as the daemon follows it: the helper's
//! detector on the meeting's microphone stream (on and off with the
//! meeting), the steps it reports, the question it hears, and the line the
//! panel says when a phrase was heard and its question abandoned.
use super::super::*;
use super::MAX_WAKE_ORDERS;
use super::causes::bounded;
use orbit_core::meeting_question::{
    DEFAULT_WINDOW_MINUTES, FAILURE_SHOWN_MS, MAX_VOICE_TRIGGER_DETAIL_BYTES, QuestionOrigin,
    VoiceTrigger,
};
use orbit_core::notice::{Notice, NoticeCode, NoticeKey};
use orbit_core::wire::native::{NativeEvent, WakePhraseAbandonmentCause};

impl Runtime {
    /// The helper heard «Oye {nombre}, …» on the user's microphone. Taken only
    /// from the microphone stream of the recording in progress, with the
    /// user's stop off and no voice note being dictated (the dictation is
    /// then the question).
    pub(in crate::daemon) async fn on_assistant_question(&mut self, event: &Value) -> Result<()> {
        let decoded: NativeEvent = serde_json::from_value(event.clone())?;
        let NativeEvent::AssistantQuestion {
            stream_id,
            text,
            started_ms,
            ended_ms,
            assistant_name,
            ..
        } = decoded
        else {
            bail!("Evento assistant_question incoherente");
        };
        // A question came: no phrase is left abandoned.
        self.features.question.abandoned = None;
        // The agent whose name was heard; a helper that knows one name heard
        // the primary's. A name no longer listed (renamed or removed while it
        // was said) was nobody's: neither the primary nor the open
        // conversation was called.
        let roster = self.settings.roster();
        let agent = match &assistant_name {
            None => Some(roster.default_agent_id().to_owned()),
            Some(name) => roster.named(name).map(|agent| agent.id.clone()),
        };
        let ignored = if self.state.privacy_stopped() {
            Some("privacy_stopped")
        } else if self.capture.mic_stream.as_deref() != Some(stream_id.as_str()) {
            Some("other_stream")
        } else if !self
            .state
            .durable
            .recording
            .as_ref()
            .is_some_and(|recording| {
                recording.phase == CapturePhase::Recording && recording.ended_at_ms.is_none()
            })
        {
            Some("not_recording")
        } else if self.state.durable.voice_active || self.capture.buffer.voice.is_some() {
            Some("dictating")
        } else if agent.is_none() {
            Some("agent_not_listed")
        } else {
            None
        };
        // Never the text: only that a question was heard, and how long it took.
        let spoken_ms = ended_ms.saturating_sub(started_ms).max(0).to_string();
        let latency_ms = self
            .question_now()
            .saturating_sub(ended_ms)
            .max(0)
            .to_string();
        self.log(
            "assistant_question_heard",
            &[
                ("ignored", ignored.unwrap_or("no")),
                ("spoken_ms", &spoken_ms),
                ("delivery_ms", &latency_ms),
            ],
        );
        if let Some(cause) = ignored {
            self.record(
                Notice::new(NoticeCode::WakePhraseQuestionIgnored)
                    .with_token(NoticeKey::Cause, cause),
            );
            return Ok(());
        }
        self.record(NoticeCode::WakePhraseQuestionReceived);
        self.ask_meeting(
            text,
            u16::from(DEFAULT_WINDOW_MINUTES),
            QuestionOrigin::Voice,
            agent,
        )
        .await
    }

    /// The helper's `voice_trigger`: where the «Oye Deimos» detector is.
    pub(in crate::daemon) fn on_voice_trigger(&mut self, event: &Value) -> Result<()> {
        let decoded: NativeEvent = serde_json::from_value(event.clone())?;
        let NativeEvent::VoiceTrigger {
            state,
            cause,
            detail,
            percent,
            os_version,
            ..
        } = decoded
        else {
            bail!("Evento voice_trigger incoherente");
        };
        let detail = detail.map(|text| bounded(&text, MAX_VOICE_TRIGGER_DETAIL_BYTES));
        let os_version = os_version.map(|text| bounded(&text, MAX_VOICE_TRIGGER_DETAIL_BYTES));
        let cause = cause.map(|text| bounded(&text, MAX_VOICE_TRIGGER_DETAIL_BYTES));
        let trigger = match state.as_str() {
            "unavailable" => VoiceTrigger::Unavailable {
                cause: cause.unwrap_or_else(|| "unspecified".into()),
                os_version,
            },
            "idle" => VoiceTrigger::Idle,
            "downloading" => VoiceTrigger::Downloading {
                percent: percent.map(|value| value.min(100)),
            },
            "listening" => VoiceTrigger::Listening,
            "failed" => VoiceTrigger::Failed {
                cause: cause.unwrap_or_else(|| "unspecified".into()),
                detail,
            },
            other => VoiceTrigger::Failed {
                cause: "unrecognized_state".into(),
                detail: Some(bounded(other, MAX_VOICE_TRIGGER_DETAIL_BYTES)),
            },
        };
        let said = wake_phrase_notice(&trigger);
        if self
            .features
            .question
            .voice_trigger
            .as_ref()
            .map(wake_phrase_notice)
            != Some(said.clone())
        {
            self.record(said);
        }
        self.features.question.voice_trigger = Some(trigger);
        Ok(())
    }

    /// Whether `id` is an `audio_wake_phrase` order still waiting for its answer.
    pub(in crate::daemon) fn wake_order_pending(&self, id: &str) -> bool {
        self.features.question.wake_orders.contains_key(id)
    }

    /// The helper came or went: what its detector said no longer holds, and
    /// it listens on nothing.
    pub(in crate::daemon) fn voice_trigger_unknown(&mut self) {
        let question = &mut self.features.question;
        question.voice_trigger = None;
        question.wake_stream = None;
        question.wake_orders.clear();
    }

    /// The helper bundled with the daemon accepts the wake-phrase order.
    pub(in crate::daemon) fn wake_helper_ready(&mut self) {
        self.features.question.helper_listens = true;
    }

    /// The microphone stream «Oye Deimos» should listen on now: the meeting
    /// recording's while it starts or records, with the user's stop off and a
    /// helper that knows the order. A voice note's stream counts once the
    /// meeting uses it; a paused or finished meeting listens on nothing.
    fn wake_target(&self) -> Option<String> {
        let live = self
            .state
            .durable
            .recording
            .as_ref()
            .is_some_and(|recording| {
                recording.ended_at_ms.is_none()
                    && matches!(
                        recording.phase,
                        CapturePhase::Starting | CapturePhase::Recording
                    )
            });
        (live && !self.state.privacy_stopped() && self.state.helper_connected())
            .then(|| self.capture.mic_stream.clone())
            .flatten()
    }

    /// Turns the phrase on or off in the helper so it follows the meeting.
    /// The user's stop and a lost helper turn it off on the helper's side by
    /// themselves; the daemon then only forgets it.
    pub(in crate::daemon) async fn sync_wake_phrase(&mut self) -> Result<()> {
        if self.state.privacy_stopped() || !self.state.helper_connected() {
            self.features.question.wake_stream = None;
            return Ok(());
        }
        if !self.features.question.helper_listens {
            return Ok(());
        }
        let target = self.wake_target();
        if target == self.features.question.wake_stream {
            return Ok(());
        }
        // A phrase abandoned on another stream, or before a pause, is not
        // this one's.
        self.features.question.abandoned = None;
        if let Some(previous) = self.features.question.wake_stream.take() {
            self.send_wake_order(previous, false).await;
        }
        if let Some(stream) = target {
            // A fresh request: an old failure or idle no longer describes it,
            // while a Mac that can never listen keeps saying so.
            if !matches!(
                self.features.question.voice_trigger,
                Some(VoiceTrigger::Unavailable { .. })
            ) {
                self.features.question.voice_trigger = None;
            }
            self.features.question.wake_stream = Some(stream.clone());
            self.send_wake_order(stream, true).await;
        }
        Ok(())
    }

    /// Sends one order. A failure never stops what asked for it (a pause, a
    /// start): an «on» that did not leave is the status's cause.
    pub(in crate::daemon) async fn send_wake_order(&mut self, stream: String, enabled: bool) {
        let sent = self.try_send_wake_order(stream, enabled).await;
        if let Err(error) = sent {
            self.log(
                "audio_wake_phrase_not_sent",
                &[("enabled", if enabled { "true" } else { "false" })],
            );
            if enabled {
                self.features.question.voice_trigger = Some(VoiceTrigger::Failed {
                    cause: "wake_order_not_sent".into(),
                    detail: Some(bounded(&error.to_string(), MAX_VOICE_TRIGGER_DETAIL_BYTES)),
                });
            }
        }
    }

    async fn try_send_wake_order(&mut self, stream: String, enabled: bool) -> Result<()> {
        // Every agent's name; `assistant_name` stays the default one's.
        let roster = self.settings.roster();
        let command = crate::native::command(
            "audio_wake_phrase",
            json!({"stream_id": stream, "enabled": enabled,
                "assistant_name": roster.default_name(), "assistant_names": roster.names()}),
        )?;
        let header = orbit_core::wire::native::NativeCommandHeader::from_value(&command)?;
        let order_id = header.id.to_owned();
        let question = &mut self.features.question;
        // Bounded: two orders per meeting phase change; the oldest answer is
        // the least useful one to wait for.
        if question.wake_orders.len() >= MAX_WAKE_ORDERS
            && let Some(oldest) = question.wake_orders.keys().next().cloned()
        {
            question.wake_orders.remove(&oldest);
        }
        question.wake_orders.insert(order_id, (stream, enabled));
        self.native_send_command(command).await
    }

    /// The helper's answer to an `audio_wake_phrase` order. Returns whether
    /// it was one. A refused «on» is the status's cause, with its code; a
    /// refused «off» only means the stream had already gone.
    pub(in crate::daemon) fn wake_phrase_response(
        &mut self,
        response: &orbit_core::wire::native::NativeResponseFields,
    ) -> bool {
        let Some((stream, enabled)) = self.features.question.wake_orders.remove(&response.id)
        else {
            return false;
        };
        let code = response
            .error
            .as_ref()
            .map_or("confirmed", |error| error.code.as_str());
        self.log(
            "audio_wake_phrase_answered",
            &[
                ("enabled", if enabled { "true" } else { "false" }),
                ("code", code),
            ],
        );
        if enabled && !response.ok && self.features.question.wake_stream.as_deref() == Some(&stream)
        {
            self.features.question.voice_trigger = Some(VoiceTrigger::Failed {
                cause: bounded(code, MAX_VOICE_TRIGGER_DETAIL_BYTES),
                detail: None,
            });
        }
        true
    }

    /// The recording topic's «Oye Deimos» status.
    pub(in crate::daemon) fn voice_status(
        &self,
    ) -> Option<orbit_core::meeting_question::VoiceStatus> {
        orbit_core::meeting_question::voice_status(&orbit_core::meeting_question::VoiceFacts {
            recording: self
                .state
                .durable
                .recording
                .as_ref()
                .filter(|recording| recording.ended_at_ms.is_none())
                .map(|recording| recording.phase.clone()),
            privacy_stopped: self.state.privacy_stopped(),
            helper_connected: self.state.helper_connected(),
            helper_listens: self.features.question.helper_listens,
            requested: self.features.question.wake_stream.is_some(),
            report: self.features.question.voice_trigger.as_ref(),
            abandoned: self.abandoned_phrase(),
        })
    }

    /// The phrase abandoned last, while the panel still says so: as long as
    /// a failure stays there.
    fn abandoned_phrase(&self) -> Option<WakePhraseAbandonmentCause> {
        let (cause, at_ms) = self.features.question.abandoned.as_ref()?;
        (self.question_now().saturating_sub(*at_ms) < FAILURE_SHOWN_MS).then(|| cause.clone())
    }

    /// A step the helper reports that answers no order. A code this
    /// daemon does not know is the helper's error, never silence. A phrase
    /// abandoned with its cause is also the panel's line; one heard again
    /// replaces it.
    pub(in crate::daemon) fn on_native_notice(&mut self, event: &Value) -> Result<()> {
        let NativeEvent::Notice {
            code, abandonment, ..
        } = serde_json::from_value(event.clone())?
        else {
            bail!("Evento notice incoherente");
        };
        match code.as_str() {
            "wake_phrase_heard" => {
                self.features.question.abandoned = None;
                self.record(NoticeCode::WakePhraseHeard);
            }
            "wake_phrase_abandoned" => {
                let mut notice = Notice::new(NoticeCode::WakePhraseAbandoned);
                if let Some(abandonment) = abandonment {
                    notice = notice
                        .with_token(NoticeKey::Cause, abandonment.cause.as_str())
                        .with(NoticeKey::ElapsedMs, abandonment.elapsed_ms)
                        .with(NoticeKey::SilentMs, abandonment.silent_ms);
                    self.features.question.abandoned =
                        Some((abandonment.cause, self.question_now()));
                }
                self.record(notice);
            }
            "wake_phrase_question_dropped" => self.record(NoticeCode::WakePhraseQuestionDropped),
            other => {
                self.say(Notice::new(NoticeCode::HelperError).with_token(NoticeKey::Code, other))
            }
        }
        Ok(())
    }
}

/// The notice a state of the «Oye Deimos» detector is recorded as.
/// Two states with the same notice are one step: a download's percent does
/// not record anything new; Apple's detail never leaves `daemon.log`.
fn wake_phrase_notice(trigger: &VoiceTrigger) -> Notice {
    match trigger {
        VoiceTrigger::Unavailable { cause, .. } => {
            Notice::new(NoticeCode::WakePhraseUnavailable).with_token(NoticeKey::Cause, cause)
        }
        VoiceTrigger::Idle => Notice::new(NoticeCode::WakePhraseOff),
        VoiceTrigger::Downloading { .. } => Notice::new(NoticeCode::WakePhraseDownloading),
        VoiceTrigger::Listening => Notice::new(NoticeCode::WakePhraseListening),
        VoiceTrigger::Failed { cause, .. } => {
            Notice::new(NoticeCode::WakePhraseFailed).with_token(NoticeKey::Cause, cause)
        }
    }
}
