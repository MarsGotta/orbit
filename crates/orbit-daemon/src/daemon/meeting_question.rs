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
use orbit_core::{
    Connection,
    meeting_question::{
        AskFacts, BusyTurn, DEFAULT_WINDOW_MINUTES, MAX_VOICE_TRIGGER_DETAIL_BYTES,
        MeetingQuestion, QuestionCause, QuestionOrigin, QuestionStage, TurnFacts, VoiceTrigger,
        admit, clean_question,
    },
    wire::native::WakePhraseAbandonmentCause,
};

mod causes;
mod wake_phrase;

use causes::{bounded, origin_label, submit_refusal_cause};

/// `audio_wake_phrase` orders whose answer the daemon waits for. Each phase
/// change of a meeting sends at most two (off the old stream, on the new);
/// eight cover several changes before an answer. It bounds memory, not the
/// orders sent.
const MAX_WAKE_ORDERS: usize = 8;

#[derive(Default)]
pub(super) struct QuestionRuntime {
    /// The last question and where its turn is; what the panel shows.
    pub(super) current: Option<MeetingQuestion>,
    /// «Oye Deimos» heard and its question abandoned: the helper's cause and
    /// when the daemon heard it, for the panel's line (`voice_status`).
    abandoned: Option<(WakePhraseAbandonmentCause, i64)>,
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
    pub(super) fn question_now(&self) -> i64 {
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
            .capture
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
        if self.state.durable.assistant.pending_approval.is_some() {
            Some(BusyTurn::PendingApproval)
        } else if self.state.durable.assistant.active_request.is_some() {
            Some(BusyTurn::AssistantTurn)
        } else if !dictated
            && (self.state.durable.voice.voice_active || self.capture.buffer.voice.is_some())
        {
            Some(BusyTurn::VoiceNote)
        } else if !dictated && self.conversation.voice_task.active().is_some() {
            Some(BusyTurn::VoiceTranscription)
        } else if self.state.durable.voice.call.is_some() {
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
                attachments: Vec::new(),
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
    pub(super) fn conversation_agent_id(&self) -> String {
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
                    && self.state.durable.assistant.active_request.as_deref()
                        == Some(question.id.as_str())
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
        let in_flight =
            self.state.durable.assistant.active_request.as_deref() == Some(question.id.as_str());
        // The answer is the turn's, while it is still the question's.
        let answer = self
            .state
            .durable
            .assistant
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

#[path = "meeting_question_hold.rs"]
mod hold;

#[cfg(test)]
#[path = "meeting_question_tests.rs"]
mod tests;
