//! Deimos during a meeting, as the daemon carries it out
//! (`orbit_core::meeting_question`). A question arrives typed («Preguntar a
//! Deimos»), spoken («Oye Deimos, …», the helper's `assistant_question`) or
//! dictated with the shortcut during a recording; it goes to Deimos as a
//! conversation turn about the meeting (`chat.send` with `meeting`, and the
//! hub adds its last minutes of transcript), and the recording topic
//! follows that turn to its answer or its cause for the floating panel.
//!
//! RAM only: the question is the panel's, the answer stays in the
//! conversation. Nothing heard reaches the daemon except the question text.
use super::*;
use orbit_core::notice::{Notice, NoticeCode, NoticeKey};
use orbit_core::{
    Connection,
    meeting_question::{
        AskFacts, BusyTurn, DEFAULT_WINDOW_MINUTES, MAX_VOICE_TRIGGER_DETAIL_BYTES,
        MeetingQuestion, QuestionCause, QuestionOrigin, QuestionStage, TurnFacts, VoiceTrigger,
        admit, clean_question, same_question,
    },
    wire::native::NativeEvent,
};

/// `audio_wake_phrase` orders whose answer the daemon waits for. Each phase
/// change of a meeting sends at most two (off the old stream, on the new);
/// eight cover several changes before an answer. It bounds memory, not the
/// orders sent.
const MAX_WAKE_ORDERS: usize = 8;

#[derive(Default)]
pub(super) struct QuestionRuntime {
    /// The last question and where its turn is; what the panel shows.
    pub(super) current: Option<MeetingQuestion>,
    /// The cause heard for the question's turn before it ended (a hub error,
    /// an interruption), kept until the turn is followed to its end.
    failure: Option<QuestionCause>,
    /// The «Oye Deimos» detector, as the helper last said; `None` before the
    /// helper says anything (unknown, never «unavailable»).
    pub(super) voice_trigger: Option<VoiceTrigger>,
    /// The bundled helper has announced ready and accepts wake-phrase orders.
    pub(super) helper_listens: bool,
    /// The microphone stream the helper was last asked to listen on: the
    /// meeting recording's, whoever opened it.
    pub(super) wake_stream: Option<String>,
    /// `audio_wake_phrase` orders in flight: id → (stream, enabled).
    wake_orders: std::collections::BTreeMap<String, (String, bool)>,
    /// Tests drive the clock; production reads the wall.
    #[cfg(test)]
    pub(super) clock_ms: Option<i64>,
}

/// The recording a question is about, owned.
struct Meeting {
    recording_id: String,
    title: String,
}

impl Runtime {
    fn question_now(&self) -> i64 {
        #[cfg(test)]
        if let Some(now_ms) = self.features.question.clock_ms {
            return now_ms;
        }
        now()
    }

    /// The recording in progress (recording or paused): what the user can ask
    /// about. A recording starting or closing has nothing to ask yet.
    fn meeting_in_progress(&self) -> Option<Meeting> {
        self.state
            .durable
            .recording
            .as_ref()
            .filter(|recording| {
                recording.ended_at_ms.is_none()
                    && matches!(
                        recording.phase,
                        CapturePhase::Recording | CapturePhase::Paused
                    )
            })
            .map(|recording| Meeting {
                recording_id: recording.id.clone(),
                title: recording.title.clone(),
            })
    }

    /// The first thing in progress that a new turn would collide with.
    /// `dictated`: the question is the voice note itself, so the voice note
    /// and its transcription are not in the way.
    fn busy_turn(&self, dictated: bool) -> Option<BusyTurn> {
        if self.state.durable.pending_approval.is_some() {
            Some(BusyTurn::PendingApproval)
        } else if self.state.durable.active_request.is_some() {
            Some(BusyTurn::AssistantTurn)
        } else if !dictated
            && (self.state.durable.voice_active || self.capture.buffer.voice.is_some())
        {
            Some(BusyTurn::VoiceNote)
        } else if !dictated && self.conversation.voice_task.active().is_some() {
            Some(BusyTurn::VoiceTranscription)
        } else if self.state.durable.call.is_some() {
            Some(BusyTurn::Call)
        } else {
            None
        }
    }

    fn ask_facts(&self, meeting: Option<&Meeting>, dictated: bool) -> AskFacts {
        AskFacts {
            privacy_stopped: self.state.privacy_stopped(),
            connection: self.state.connection(),
            busy: self.busy_turn(dictated),
            in_meeting: meeting.is_some(),
        }
    }

    /// A question typed or spoken during the meeting. Refused, it is shown
    /// with its cause; admitted, it goes to the assistant as a turn about the
    /// meeting, whose text the hub builds. `agent` is the agent the user
    /// named («Oye Atlas»): a conversation is with one agent, so when it is
    /// not the open conversation's, the question begins one of its own.
    /// `None` asks in the open conversation.
    pub(super) async fn ask_meeting(
        &mut self,
        question: String,
        minutes: u16,
        origin: QuestionOrigin,
        agent: Option<String>,
    ) -> Result<()> {
        let now_ms = self.question_now();
        let meeting = self.meeting_in_progress();
        let facts = self.ask_facts(meeting.as_ref(), false);
        let request_id = id();
        let window = u8::try_from(minutes).unwrap_or(DEFAULT_WINDOW_MINUTES);
        let question = clean_question(&question).to_owned();
        let admitted = admit(&facts, &question, minutes);
        let (Ok(()), Some(meeting)) = (admitted.clone(), meeting.as_ref()) else {
            let cause = admitted.err().unwrap_or(QuestionCause::NoMeeting);
            self.refuse_question(
                request_id,
                meeting.as_ref(),
                question,
                window,
                origin,
                cause,
                now_ms,
            );
            return Ok(());
        };
        if let Some(agent) = agent.filter(|agent| *agent != self.conversation_agent_id()) {
            self.apply(Event::NewSession {
                agent_id: Some(agent),
            })
            .await?;
        }
        let decision = match decide(
            &self.state,
            Event::Submit {
                id: request_id.clone(),
                text: question.clone(),
            },
            now(),
        ) {
            Ok(decision) => decision,
            Err(refusal) => {
                let cause = submit_refusal_cause(&refusal, &facts);
                self.refuse_question(
                    request_id,
                    Some(meeting),
                    question,
                    window,
                    origin,
                    cause,
                    now_ms,
                );
                return Ok(());
            }
        };
        self.send_meeting_turn(
            decision, meeting, question, window, origin, request_id, now_ms,
        )
        .await
    }

    /// The agent the open conversation is with: its own, or the default one.
    fn conversation_agent_id(&self) -> String {
        self.state.agent_id().map_or_else(
            || self.settings.roster().default_agent_id().to_owned(),
            str::to_owned,
        )
    }

    /// The dictation shortcut during a recording: the transcribed voice note
    /// is the question, about the meeting's default window. Returns whether
    /// it was taken as a meeting question (otherwise the voice note goes on
    /// as an ordinary turn).
    pub(super) async fn ask_meeting_by_dictation(
        &mut self,
        message_id: String,
        text: String,
        duration_ms: u64,
    ) -> Result<bool> {
        let Some(meeting) = self.meeting_in_progress() else {
            return Ok(false);
        };
        let now_ms = self.question_now();
        let facts = self.ask_facts(Some(&meeting), true);
        let question = clean_question(&text).to_owned();
        let window = DEFAULT_WINDOW_MINUTES;
        let origin = QuestionOrigin::Hotkey;
        if let Err(cause) = admit(&facts, &question, u16::from(window)) {
            self.refuse_question(
                message_id,
                Some(&meeting),
                question,
                window,
                origin,
                cause,
                now_ms,
            );
            return Ok(true);
        }
        let decision = match decide(
            &self.state,
            Event::SubmitVoice {
                id: message_id.clone(),
                text,
                duration_ms,
            },
            now(),
        ) {
            Ok(decision) => decision,
            Err(refusal) => {
                let cause = submit_refusal_cause(&refusal, &facts);
                self.refuse_question(
                    message_id,
                    Some(&meeting),
                    question,
                    window,
                    origin,
                    cause,
                    now_ms,
                );
                return Ok(true);
            }
        };
        self.send_meeting_turn(
            decision, &meeting, question, window, origin, message_id, now_ms,
        )
        .await?;
        Ok(true)
    }

    #[allow(clippy::too_many_arguments)]
    async fn send_meeting_turn(
        &mut self,
        mut decision: orbit_core::Decision,
        meeting: &Meeting,
        question: String,
        window: u8,
        origin: QuestionOrigin,
        request_id: String,
        now_ms: i64,
    ) -> Result<()> {
        // The turn names the meeting; the hub adds what was said in it and
        // keeps the question with its mark.
        for effect in &mut decision.effects {
            if let Effect::SendText { meeting: sent, .. } = effect {
                *sent = Some(orbit_core::wire::hub::ChatMeeting {
                    recording_id: meeting.recording_id.clone(),
                    minutes: window,
                    title: meeting.title.clone(),
                });
            }
        }
        // No speech during a meeting: the call would hear it.
        self.conversation.speak_next = false;
        self.features.question.failure = None;
        self.features.question.current = Some(MeetingQuestion::thinking(
            request_id,
            meeting.recording_id.clone(),
            question,
            window,
            origin,
            now_ms,
        ));
        self.log(
            "meeting_question_sent",
            &[
                ("origin", origin_label(origin)),
                ("minutes", &window.to_string()),
            ],
        );
        if let Err(error) = self.execute_decision(decision).await {
            // The turn could not leave: it never reached Deimos.
            if let Some(question) = self.features.question.current.as_mut() {
                question.stage = QuestionStage::Failed;
                question.cause = Some(QuestionCause::HubRefused {
                    code: "send_failed".into(),
                    message: bounded(&error.to_string(), MAX_VOICE_TRIGGER_DETAIL_BYTES),
                });
                question.finished_at_ms = Some(now_ms);
                question.sent = false;
            }
            return Err(error);
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn refuse_question(
        &mut self,
        request_id: String,
        meeting: Option<&Meeting>,
        question: String,
        window: u8,
        origin: QuestionOrigin,
        cause: QuestionCause,
        now_ms: i64,
    ) {
        let thinking = self
            .features
            .question
            .current
            .as_ref()
            .filter(|current| current.stage == QuestionStage::Thinking);
        // The helper heard the same words again: that question is the one
        // on its way, and nothing was refused that the user asked.
        let repeated = origin == QuestionOrigin::Voice
            && thinking.is_some_and(|current| same_question(&current.question, &question));
        let thinking = thinking.is_some();
        self.log(
            "meeting_question_refused",
            &[
                ("origin", origin_label(origin)),
                ("cause", cause_label(&cause)),
                ("repeated", if repeated { "yes" } else { "no" }),
            ],
        );
        if repeated {
            return;
        }
        // A question still waiting for Deimos keeps the panel: its answer is
        // on its way. The refused one is said in the notices instead.
        if thinking {
            let notice = if matches!(cause, QuestionCause::TurnInFlight { .. }) {
                Notice::new(NoticeCode::MeetingQuestionStillAnswering)
            } else {
                let notice = Notice::new(NoticeCode::MeetingQuestionNotSent)
                    .with_token(NoticeKey::Cause, cause_label(&cause));
                if let QuestionCause::QuestionTooLong { bytes, max_bytes } = cause {
                    notice
                        .with(NoticeKey::Bytes, bytes)
                        .with(NoticeKey::MaxBytes, max_bytes)
                } else {
                    notice
                }
            };
            self.say(notice);
            return;
        }
        self.features.question.failure = None;
        self.features.question.current = Some(MeetingQuestion::refused(
            request_id,
            meeting.map(|meeting| meeting.recording_id.clone()),
            question,
            window,
            origin,
            cause,
            now_ms,
        ));
    }

    /// The helper heard «Oye Deimos, …» on the user's microphone. Taken only
    /// from the microphone stream of the recording in progress, with the
    /// user's stop off and no voice note being dictated (the dictation is
    /// then the question).
    pub(super) async fn on_assistant_question(&mut self, event: &Value) -> Result<()> {
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
        // The agent whose name was heard; a helper that knows one name, or
        // a name no longer listed, asks in the open conversation.
        let agent = assistant_name.and_then(|name| {
            self.settings
                .roster()
                .named(&name)
                .map(|agent| agent.id.clone())
        });
        self.ask_meeting(
            text,
            u16::from(DEFAULT_WINDOW_MINUTES),
            QuestionOrigin::Voice,
            agent,
        )
        .await
    }

    /// The helper's `voice_trigger`: where the «Oye Deimos» detector is.
    pub(super) fn on_voice_trigger(&mut self, event: &Value) -> Result<()> {
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
    pub(super) fn wake_order_pending(&self, id: &str) -> bool {
        self.features.question.wake_orders.contains_key(id)
    }

    /// The helper came or went: what its detector said no longer holds, and
    /// it listens on nothing.
    pub(super) fn voice_trigger_unknown(&mut self) {
        let question = &mut self.features.question;
        question.voice_trigger = None;
        question.wake_stream = None;
        question.wake_orders.clear();
    }

    /// The helper bundled with the daemon accepts the wake-phrase order.
    pub(super) fn wake_helper_ready(&mut self) {
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
    pub(super) async fn sync_wake_phrase(&mut self) -> Result<()> {
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
    pub(super) async fn send_wake_order(&mut self, stream: String, enabled: bool) {
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
    pub(super) fn wake_phrase_response(
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
    pub(super) fn voice_status(&self) -> Option<orbit_core::meeting_question::VoiceStatus> {
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
        })
    }

    /// The hub's error for the turn in flight, when it is the question's.
    pub(super) fn meeting_question_hub_error(&mut self, code: Option<&str>, message: &str) {
        if !self.question_turn_in_flight() {
            return;
        }
        self.features.question.failure = Some(match code {
            Some("upstream_unavailable" | "upstream_rejected" | "upstream_protocol") => {
                QuestionCause::DeimosUnavailable {
                    code: code.unwrap_or_default().into(),
                }
            }
            Some("session_busy") => QuestionCause::TurnInFlight {
                turn: BusyTurn::AssistantTurn,
            },
            Some("revoked") => QuestionCause::Revoked,
            other => QuestionCause::HubRefused {
                code: other.unwrap_or("unspecified").into(),
                message: bounded(message, MAX_VOICE_TRIGGER_DETAIL_BYTES),
            },
        });
    }

    /// Deimos ended the question's turn with an error.
    pub(super) fn meeting_question_turn_failed(&mut self) {
        if self.question_turn_in_flight() {
            self.features.question.failure = Some(QuestionCause::TurnFailed);
        }
    }

    /// The user interrupted the turn in flight.
    pub(super) fn meeting_question_interrupted(&mut self) {
        if self.question_turn_in_flight() {
            self.features.question.failure = Some(QuestionCause::Interrupted);
        }
    }

    fn question_turn_in_flight(&self) -> bool {
        self.features
            .question
            .current
            .as_ref()
            .is_some_and(|question| {
                question.stage == QuestionStage::Thinking
                    && self.state.durable.active_request.as_deref() == Some(question.id.as_str())
            })
    }

    /// Follows the question's turn in the conversation: its answer, or the
    /// cause it ended with.
    pub(super) fn follow_meeting_question(&mut self) {
        let now_ms = self.question_now();
        let Some(question) = self.features.question.current.as_ref() else {
            return;
        };
        if question.stage != QuestionStage::Thinking {
            return;
        }
        let in_flight = self.state.durable.active_request.as_deref() == Some(question.id.as_str());
        // The answer is the turn's, while it is still the question's.
        let answer = self
            .state
            .durable
            .turn
            .as_ref()
            .filter(|turn| turn.user.id == question.id)
            .and_then(|turn| turn.answer.as_ref())
            .filter(|answer| answer.complete)
            .map(|answer| answer.text.clone());
        let derived = if self.state.privacy_stopped() {
            Some(QuestionCause::PrivacyStopped)
        } else {
            match self.state.connection() {
                Connection::Online => None,
                Connection::Revoked => Some(QuestionCause::Revoked),
                connection @ (Connection::Offline | Connection::Connecting) => {
                    Some(QuestionCause::HubUnreachable { connection })
                }
            }
        };
        let failure = self.features.question.failure.clone().or(derived);
        let Some(question) = self.features.question.current.as_mut() else {
            return;
        };
        if question.follow(
            TurnFacts {
                in_flight,
                answer: answer.as_deref(),
                failure: failure.as_ref(),
            },
            now_ms,
        ) {
            self.features.question.failure = None;
        }
    }

    /// Once a second: follow the turn, and let a question go once the panel
    /// has shown it long enough.
    pub(super) async fn maintain_meeting_question(&mut self) -> Result<()> {
        self.sync_wake_phrase().await?;
        self.follow_meeting_question();
        let now_ms = self.question_now();
        if self
            .features
            .question
            .current
            .as_ref()
            .is_some_and(|question| question.expired(now_ms))
        {
            self.features.question.current = None;
        }
        Ok(())
    }

    /// The user closed the question in the panel.
    pub(super) fn close_meeting_question(&mut self, id: &str) {
        if self
            .features
            .question
            .current
            .as_ref()
            .is_some_and(|question| question.id == id)
        {
            self.features.question.current = None;
            self.features.question.failure = None;
        }
    }
}

/// The core refused the turn after the admission let it through (the state
/// changed in between): name the same causes.
fn submit_refusal_cause(refusal: &orbit_core::refusal::Refusal, facts: &AskFacts) -> QuestionCause {
    use orbit_core::refusal::Refusal as Core;
    // Only these three repeat an admission cause; any other refusal of the
    // core travels with its own sentence.
    if matches!(
        refusal,
        Core::Busy {
            operation: orbit_core::refusal::Operation::Interaction,
            retry_after_ms: None
        }
    ) {
        QuestionCause::TurnInFlight {
            turn: facts.busy.unwrap_or(BusyTurn::AssistantTurn),
        }
    } else if matches!(refusal, Core::HubUnreachable { since_ms: None }) {
        QuestionCause::HubUnreachable {
            connection: facts.connection.clone(),
        }
    } else if matches!(refusal, Core::PrivacyStopped { since_ms: None }) {
        QuestionCause::PrivacyStopped
    } else {
        QuestionCause::HubRefused {
            code: "core_refused".into(),
            message: refusal.to_string(),
        }
    }
}

fn origin_label(origin: QuestionOrigin) -> &'static str {
    match origin {
        QuestionOrigin::Voice => "voice",
        QuestionOrigin::Typed => "typed",
        QuestionOrigin::Hotkey => "hotkey",
    }
}

fn cause_label(cause: &QuestionCause) -> &'static str {
    match cause {
        QuestionCause::PrivacyStopped => "privacy_stopped",
        QuestionCause::NoMeeting => "no_meeting",
        QuestionCause::HubUnreachable { .. } => "hub_unreachable",
        QuestionCause::DeimosUnavailable { .. } => "deimos_unavailable",
        QuestionCause::TurnInFlight { .. } => "turn_in_flight",
        QuestionCause::QuestionEmpty => "question_empty",
        QuestionCause::QuestionTooLong { .. } => "question_too_long",
        QuestionCause::WindowNotOffered { .. } => "window_not_offered",
        QuestionCause::TurnFailed => "turn_failed",
        QuestionCause::Interrupted => "interrupted",
        QuestionCause::HubRefused { .. } => "hub_refused",
        QuestionCause::NoAnswer => "no_answer",
        QuestionCause::Revoked => "revoked",
    }
}

/// `text` cut at a character boundary to `max` bytes: a diagnostic the
/// helper or the hub sent, never conversation content.
fn bounded(text: &str, max: usize) -> String {
    if text.len() <= max {
        return text.to_owned();
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_owned()
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

#[cfg(test)]
#[path = "meeting_question_tests.rs"]
mod tests;
