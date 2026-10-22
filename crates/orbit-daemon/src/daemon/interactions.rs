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

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum TurnState {
    Idle,
    Submitting,
    Streaming,
    Interrupting,
    Completed,
    Failed,
    OutcomeUnknown,
}

impl TurnState {
    fn keeps_response_visible(self) -> bool {
        match self {
            Self::Idle => false,
            Self::Submitting => true,
            Self::Streaming => true,
            // Orbit has already applied the local interrupt and discarded the
            // request identity. The hub may still observe a terminal upstream
            // event, but it is the answer the person chose to interrupt.
            Self::Interrupting => false,
            Self::Completed => false,
            Self::Failed => false,
            // The hub makes this state terminal and forbids a resend. There is
            // no response known safe to attach to the selected conversation.
            Self::OutcomeUnknown => false,
        }
    }
}

pub(super) struct PendingResponse {
    command_id: String,
    request_id: String,
    session_id: String,
    clarification: bool,
}

impl Runtime {
    pub(super) fn remember_interrupted_turn(&mut self, turn_id: &str) {
        let Some(session_id) = self.state.session_id().clone() else {
            return;
        };
        self.conversation
            .interrupted_turns
            .remember(session_id, turn_id.into());
    }

    fn forget_interrupted_turn(&mut self, turn_id: &str) {
        let Some(session_id) = self.state.session_id().clone() else {
            return;
        };
        self.conversation
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
        self.conversation.interrupted_turns.await_ack(
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
        self.conversation.interrupted_turns.forget_pending_ack();
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
        if !self.conversation.interrupted_turns.rejected(frame_id) {
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
            bail!("Conecta y rearma Orbit antes de responder");
        }
        if self.conversation.interaction_response.is_some() {
            bail!("La respuesta anterior aún no está confirmada; no se reenvía");
        }
        let turn = self
            .state
            .durable
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
        self.conversation.interaction_response = Some(PendingResponse {
            command_id,
            request_id: request,
            session_id,
            clarification: method == "clarify.respond",
        });
        self.say(NoticeCode::ApprovalSent);
        Ok(())
    }

    pub(super) async fn interaction_error(&mut self, frame_id: Option<&str>) -> Result<()> {
        if self
            .conversation
            .interaction_response
            .as_ref()
            .is_some_and(|pending| Some(pending.command_id.as_str()) == frame_id)
        {
            self.conversation.interaction_response = None;
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
        let Some(pending) = self.conversation.interaction_response.as_ref() else {
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
        self.conversation.interaction_response = None;
        if (applied || expired) && data.stale_interaction == Some(true) {
            // The receipt describes an earlier pending object, even when its
            // request ID was reused. Only the current snapshot owns that state.
            self.say(NoticeCode::ApprovalReceiptStale);
        } else if applied || expired {
            let remaining = data.remaining.as_deref().unwrap_or_default();
            if let Some(approval) = self
                .state
                .durable
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
                    self.state.durable.pending_approval = None;
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

    pub(super) async fn reconcile_typed_snapshot(&mut self, snapshot: &SnapshotData) -> Result<()> {
        let revision = snapshot.revision;
        if self
            .snapshot_revision
            .is_some_and(|current| revision < current)
        {
            return Ok(());
        }
        let sessions = &snapshot.sessions;
        if sessions.len() > 128 {
            bail!("Snapshot con {} sesiones; se admiten 128", sessions.len());
        }
        let Some(selected) = self.state.session_id().clone() else {
            self.snapshot_revision = Some(revision);
            return Ok(());
        };
        let Some(session) = sessions
            .iter()
            .find(|session| session.session_id == selected)
        else {
            // Omitted sessions are not proof of completion or loss of ownership.
            return Ok(());
        };
        if self
            .state
            .durable
            .active_request
            .as_deref()
            .is_some_and(|active| session.turn_id.as_deref() != Some(active))
        {
            // A snapshot emitted just before our new command reached the hub
            // must not erase that command's locally admitted identity.
            return Ok(());
        }
        let raw_status = &session.status;
        let Ok(turn_state) = serde_json::from_value::<TurnState>(Value::String(raw_status.clone()))
        else {
            // The hub runs from `main` and the Mac from its last release, so a
            // newer hub may publish a state this version does not know (the
            // two are tolerant of each other). Nothing is decided on it:
            // the turn, its pending questions and the revision stay as they
            // were, and the user is told the value, cut to what a notice can show.
            self.log("unknown_turn_state", &[("status", raw_status)]);
            self.say(
                Notice::new(NoticeCode::AssistantStateUnknown)
                    .with_token(NoticeKey::Code, raw_status),
            );
            return Ok(());
        };
        let snapshot_turn_id = session.turn_id.as_deref();
        let was_interrupted = snapshot_turn_id.is_some_and(|turn_id| {
            self.conversation
                .interrupted_turns
                .contains(&selected, turn_id)
        });
        let mut next = self.state.clone();
        let pending = &session.pending;
        if pending.len() > 128 {
            bail!(
                "Snapshot con {} interacciones pendientes; se admiten 128",
                pending.len()
            );
        }
        next.durable.pending_approval = None;
        if turn_state.keeps_response_visible() && !was_interrupted {
            let turn_id = session
                .turn_id
                .as_deref()
                .context("Turno activo sin identidad")?;
            uuid::Uuid::parse_str(turn_id).context("Identidad de turno incompatible")?;
            next.durable.active_request = Some(turn_id.into());
            if let Some(question) = pending.first() {
                let request_id = question.request_id.as_str();
                if request_id.is_empty() || request_id.len() > 160 {
                    bail!(
                        "Snapshot de interacción con request_id de {} bytes; se admiten 1 a 160",
                        request_id.len()
                    );
                }
                if question.state != "pending"
                    || !matches!(question.kind.as_str(), "approval" | "clarify")
                {
                    bail!("Snapshot de interacción incompatible");
                }
                let description = question
                    .description
                    .as_deref()
                    .or(question.question.as_deref())
                    .context("Snapshot de interacción sin descripción o pregunta")?;
                if description.len() > 16_000 {
                    bail!(
                        "Pregunta pendiente de {} bytes; se admiten 16000",
                        description.len()
                    );
                }
                let (choices, questions) =
                    offered(question.choices.as_ref(), question.questions.as_ref());
                next.durable.pending_approval = Some(Approval {
                    request_id: request_id.into(),
                    description: description.into(),
                    clarification: question.kind == "clarify",
                    choices,
                    questions,
                });
                next.set_notice(
                    self.scoped(
                        Notice::new(NoticeCode::ApprovalsRecovered)
                            .with(NoticeKey::Count, pending.len()),
                    ),
                );
            }
        } else {
            if turn_state == TurnState::OutcomeUnknown && next.durable.active_request.is_some() {
                // Otherwise the last thing said would stay «Deimos está
                // pensando…» for a turn the hub will never finish.
                next.set_notice(self.scoped(NoticeCode::AssistantOutcomeUnknown));
            }
            next.durable.active_request = None;
        }
        self.store_actor.snapshot(&next).await?;
        if !turn_state.keeps_response_visible()
            && let Some(turn_id) = snapshot_turn_id
        {
            self.conversation
                .interrupted_turns
                .forget(&selected, turn_id);
        }
        self.state = next;
        self.snapshot_revision = Some(revision);
        Ok(())
    }
}

#[cfg(test)]
#[path = "interaction_tests.rs"]
mod tests;
