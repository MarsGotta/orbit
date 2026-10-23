//! Reconcile the selected session after reconnect without resuming an upstream
//! agent or replaying a prompt. Pending questions are facts, not authorization.
use super::{Runtime, id};
use anyhow::{Context, Result, bail};
use orbit_core::notice::{Notice, NoticeCode, NoticeKey};
use orbit_core::wire::hub::{ErrorData, InteractionData, SnapshotData};
use orbit_core::{Approval, Connection};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::VecDeque;

#[path = "interaction_snapshot.rs"]
mod snapshot;

/// Bounds volatile interrupted-turn identities so repeated local churn cannot
/// grow daemon memory forever. 128 covers one identity for every session in
/// the largest snapshot this module accepts. This is not a timeout or an
/// authorization boundary: entries retire when a snapshot reports that exact
/// turn as non-live (or oldest-first when this capacity is exhausted), and a
/// daemon restart forgets them because only the user's privacy stop is durable.
const MAX_INTERRUPTED_TURNS: usize = 128;

/// The answers and questions the Mac keeps for one pending request: the
/// hub's own bounds for what it publishes (`interactions.pending_data`:
/// 32 choices of 256 characters, 16 questions), applied again because they
/// come from outside and live in the durable state.
const MAX_CHOICES: usize = 32;
const MAX_CHOICE_CHARS: usize = 256;
const MAX_QUESTIONS: usize = 16;
/// A question's id, as `request_id` is bounded below and by the hub.
const MAX_QUESTION_ID_BYTES: usize = 160;
/// The pending question's own bound, below, applied to each of several.
const MAX_QUESTION_BYTES: usize = 16_000;

fn choices_of(choices: Option<&Vec<String>>) -> Vec<String> {
    choices
        .into_iter()
        .flatten()
        .filter(|choice| !choice.is_empty() && choice.chars().count() <= MAX_CHOICE_CHARS)
        .take(MAX_CHOICES)
        .cloned()
        .collect()
}

/// What the agent offers for a pending request: its choices and, for a
/// clarification of several questions, each readable question. One the hub
/// passed on without a readable id is left out: it could not be answered.
pub(super) fn offered(
    choices: Option<&Vec<String>>,
    questions: Option<&Vec<Value>>,
) -> (Vec<String>, Vec<orbit_core::Question>) {
    let questions = questions
        .into_iter()
        .flatten()
        .filter_map(|question| {
            let qid = question.get("qid")?.as_str()?;
            let text = question.get("question")?.as_str()?;
            if qid.is_empty()
                || qid.len() > MAX_QUESTION_ID_BYTES
                || text.len() > MAX_QUESTION_BYTES
            {
                return None;
            }
            let choices = question
                .get("choices")
                .and_then(|choices| serde_json::from_value::<Vec<String>>(choices.clone()).ok());
            Some(orbit_core::Question {
                qid: qid.into(),
                question: text.into(),
                choices: choices_of(choices.as_ref()),
                multi_select: question
                    .get("multi_select")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
            })
        })
        .take(MAX_QUESTIONS)
        .collect();
    (choices_of(choices), questions)
}

struct InterruptedTurn {
    session_id: String,
    turn_id: String,
}

/// The `chat.interrupt` command whose hub answer is still pending.
/// One conversation is visible at a time, so one interrupt is in flight at a
/// time; a stale answer cannot match a newer command's id, and a success
/// (`chat.interrupted`) needs no slot of its own because nothing is retired
/// on it.
struct PendingInterruptAck {
    command_id: String,
    turn: InterruptedTurn,
}

#[derive(Default)]
pub(super) struct InterruptedTurns {
    entries: VecDeque<InterruptedTurn>,
    pending_ack: Option<PendingInterruptAck>,
}

impl InterruptedTurns {
    fn remember(&mut self, session_id: String, turn_id: String) {
        self.entries
            .retain(|entry| entry.session_id != session_id || entry.turn_id != turn_id);
        if self.entries.len() == MAX_INTERRUPTED_TURNS {
            self.entries.pop_front();
        }
        self.entries.push_back(InterruptedTurn {
            session_id,
            turn_id,
        });
    }

    fn contains(&self, session_id: &str, turn_id: &str) -> bool {
        self.entries
            .iter()
            .any(|entry| entry.session_id == session_id && entry.turn_id == turn_id)
    }

    fn forget(&mut self, session_id: &str, turn_id: &str) {
        self.entries
            .retain(|entry| entry.session_id != session_id || entry.turn_id != turn_id);
    }

    fn await_ack(&mut self, command_id: String, session_id: String, turn_id: String) {
        self.pending_ack = Some(PendingInterruptAck {
            command_id,
            turn: InterruptedTurn {
                session_id,
                turn_id,
            },
        });
    }

    /// A reconnect ends every answer that had not
    /// arrived: the connection that owed this ack is gone, so no frame can
    /// match its command id again and the slot would forever describe an
    /// interrupt as answered-pending when nothing will answer it. The
    /// interrupted-turn memory is a fact about the turn, not the command, and
    /// stays.
    pub(super) fn forget_pending_ack(&mut self) {
        self.pending_ack = None;
    }

    /// Whether `frame_id` answers the interrupt command still in
    /// flight, and if so ends that memory: the hub refused or could not
    /// perform the interruption, so the turn stays whatever the hub publishes
    /// instead of staying hidden here.
    fn rejected(&mut self, frame_id: Option<&str>) -> bool {
        let matched = self
            .pending_ack
            .as_ref()
            .is_some_and(|ack| frame_id == Some(ack.command_id.as_str()));
        if !matched {
            return false;
        }
        if let Some(ack) = self.pending_ack.take() {
            self.forget(&ack.turn.session_id, &ack.turn.turn_id);
        }
        matched
    }
}

pub(super) struct PendingResponse {
    command_id: String,
    request_id: String,
    session_id: String,
    clarification: bool,
    /// When it left for the hub: past `INTERACTION_RESPONSE_DEADLINE` it is let go.
    sent_at: std::time::Instant,
}

/// How long an answer to the agent (an approval or a clarification) waits for
/// the hub to confirm it. The hub hands it to the agent within its own 30 s
/// call (`rpc_timeout_seconds` in `server/orbit_hub/assistant/hermes.py`) and
/// replies on the link; this is the daemon's bound for any ordinary hub
/// request (`HUB_HTTP_TIMEOUT`), three times that. Past it the answer's outcome
/// is unknown and said so, and the person may answer again: until then every
/// other answer was refused as «not confirmed yet». Nothing is resent. It does
/// not limit how long the agent's turn runs.
pub(super) const INTERACTION_RESPONSE_DEADLINE: std::time::Duration = crate::hub::HUB_HTTP_TIMEOUT;

impl Runtime {
    pub(super) fn remember_interrupted_turn(&mut self, turn_id: &str) {
        let Some(session_id) = self.state.session_id().clone() else {
            return;
        };
        self.assistant
            .interrupted_turns
            .remember(session_id, turn_id.into());
    }

    fn forget_interrupted_turn(&mut self, turn_id: &str) {
        let Some(session_id) = self.state.session_id().clone() else {
            return;
        };
        self.assistant
            .interrupted_turns
            .forget(&session_id, turn_id);
    }

    /// The interrupt command left Orbit; its `error` answer, when
    /// it comes, is what decides the memory, so keep the pair (command, turn)
    /// until then.
    pub(super) fn await_interrupt_ack(&mut self, command_id: &str, turn_id: &str) {
        let Some(session_id) = self.state.session_id().clone() else {
            return;
        };
        self.assistant.interrupted_turns.await_ack(
            command_id.to_owned(),
            session_id,
            turn_id.to_owned(),
        );
    }

    /// The interrupt request never left Orbit (channel saturated
    /// or gone, privacy latched), so the hub still owns a live turn: stop
    /// hiding it and say why it stays on screen.
    pub(super) fn interrupt_not_delivered(&mut self, turn_id: &str, cause: &str) {
        self.forget_interrupted_turn(turn_id);
        self.log("interrupt_not_delivered", &[("cause", cause)]);
        self.say(NoticeCode::AssistantInterruptNotDelivered);
    }

    /// The socket could not confirm writing a queued interrupt. A partial
    /// write is possible, so the turn is reconciled with the hub on rearm;
    /// neither success nor refusal is invented from a dropped sender.
    pub(super) fn interrupt_delivery_unknown(&mut self, turn_id: &str, cause: &str) {
        self.forget_interrupted_turn(turn_id);
        self.assistant.interrupted_turns.forget_pending_ack();
        self.say_failure(NoticeCode::AssistantInterruptUnconfirmed, &cause);
    }

    /// The hub answered our `chat.interrupt` with `error`, naming
    /// its own cause (turn already terminal, session gone, upstream refusal).
    /// With codes like `outcome_unknown` the order may still have arrived, so
    /// the notice must not claim Hermes did not interrupt: the turn is
    /// whatever the hub still publishes, the memory retires, and the user can
    /// interrupt again. Returns whether this frame was that answer.
    pub(super) async fn interrupt_error(
        &mut self,
        frame_id: Option<&str>,
        data: &ErrorData,
    ) -> Result<bool> {
        if !self.assistant.interrupted_turns.rejected(frame_id) {
            return Ok(false);
        }
        let code = data.code.as_deref().unwrap_or("not_reported");
        self.log(
            "interrupt_refused",
            &[("code", code), ("cause", &data.message)],
        );
        self.say(
            Notice::new(NoticeCode::AssistantInterruptRefused).with_token(NoticeKey::Code, code),
        );
        self.store_actor.snapshot(&self.state).await?;
        Ok(true)
    }

    pub(super) async fn respond_interaction(
        &mut self,
        method: &str,
        mut params: Value,
    ) -> Result<()> {
        if self.state.connection() != Connection::Online || self.state.privacy_stopped() {
            bail!("Conecta y reactiva Orbit antes de responder");
        }
        if self.assistant.interaction_response.is_some() {
            bail!("La respuesta anterior aún no está confirmada; no se reenvía");
        }
        let turn = self
            .state
            .durable
            .assistant
            .active_request
            .as_deref()
            .context("El turno pendiente necesita reconciliarse con el hub")?;
        let session_id = self
            .state
            .session_id()
            .clone()
            .context("Respuesta sin sesión seleccionada")?;
        #[derive(Deserialize)]
        struct ResponseIdentity {
            request_id: String,
        }
        let request = serde_json::from_value::<ResponseIdentity>(params.clone())
            .context("Respuesta sin identidad")?
            .request_id;
        if method == "approval.respond" {
            params
                .as_object_mut()
                .context("Respuesta sin objeto de parámetros")?
                .insert("turn_id".into(), json!(turn));
        }
        let command_id = id();
        self.send(crate::hub::command(&command_id, method, params)?)
            .await?;
        self.assistant.interaction_response = Some(PendingResponse {
            command_id,
            request_id: request,
            session_id,
            clarification: method == "clarify.respond",
            sent_at: std::time::Instant::now(),
        });
        self.say(NoticeCode::ApprovalSent);
        Ok(())
    }

    /// Lets go of an answer the hub has not confirmed within
    /// `INTERACTION_RESPONSE_DEADLINE`: its outcome is unknown, said so.
    pub(super) async fn expire_interaction_response(&mut self) -> Result<()> {
        if self
            .assistant
            .interaction_response
            .as_ref()
            .is_some_and(|pending| pending.sent_at.elapsed() > INTERACTION_RESPONSE_DEADLINE)
        {
            self.assistant.interaction_response = None;
            self.say(NoticeCode::ApprovalOutcomeUnknown);
            self.store_actor.snapshot(&self.state).await?;
        }
        Ok(())
    }

    pub(super) async fn interaction_error(&mut self, frame_id: Option<&str>) -> Result<()> {
        if self
            .assistant
            .interaction_response
            .as_ref()
            .is_some_and(|pending| Some(pending.command_id.as_str()) == frame_id)
        {
            self.assistant.interaction_response = None;
            self.say(NoticeCode::ApprovalOutcomeUnknown);
            self.store_actor.snapshot(&self.state).await?;
        }
        Ok(())
    }

    pub(super) async fn interaction_result_typed(
        &mut self,
        resolved: bool,
        frame_id: Option<&str>,
        data: &InteractionData,
    ) -> Result<()> {
        let Some(pending) = self.assistant.interaction_response.as_ref() else {
            return Ok(());
        };
        if frame_id != Some(pending.command_id.as_str()) {
            return Ok(());
        }
        let request_id = pending.request_id.clone();
        let matches_target =
            data.request_id == pending.request_id && data.session_id == pending.session_id;
        let applied = matches_target
            && resolved
            && data.status.as_deref() == Some("ok")
            && data.applied == Some(true)
            && (pending.clarification || data.resolved == Some(1));
        let expired = matches_target && !resolved && data.applied == Some(false);
        self.assistant.interaction_response = None;
        if (applied || expired) && data.stale_interaction == Some(true) {
            // The receipt describes an earlier pending object, even when its
            // request ID was reused. Only the current snapshot owns that state.
            self.say(NoticeCode::ApprovalReceiptStale);
        } else if applied || expired {
            let remaining = data.remaining.as_deref().unwrap_or_default();
            if let Some(approval) = self
                .state
                .durable
                .assistant
                .pending_approval
                .as_mut()
                .filter(|approval| approval.request_id == request_id)
            {
                // One of several questions answered: the others stay open,
                // as the hub's next snapshot will also say.
                approval
                    .questions
                    .retain(|question| remaining.contains(&question.qid));
                if !applied || approval.questions.is_empty() {
                    self.state.durable.assistant.pending_approval = None;
                }
            }
            self.say(if applied {
                NoticeCode::ApprovalApplied
            } else {
                NoticeCode::ApprovalExpired
            });
        } else {
            self.say(NoticeCode::ApprovalOutcomeUnknown);
        }
        self.store_actor.snapshot(&self.state).await?;
        Ok(())
    }
}

#[cfg(test)]
#[path = "interaction_tests.rs"]
mod tests;
