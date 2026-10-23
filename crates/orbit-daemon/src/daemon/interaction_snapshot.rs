//! The hub's snapshot of the conversation: the open conversation's turn and
//! its pending question are adopted from it, never from an older revision.
use super::*;

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(super) enum TurnState {
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

/// The first pending question of a snapshot, as the Mac admits it: its
/// request id, its state and kind, and its text within their bounds.
pub(super) fn recovered_approval(
    question: &orbit_core::wire::hub::PendingInteraction,
) -> Result<Approval> {
    let request_id = question.request_id.as_str();
    if request_id.is_empty() || request_id.len() > 160 {
        bail!(
            "Snapshot de interacción con request_id de {} bytes; se admiten 1 a 160",
            request_id.len()
        );
    }
    if question.state != "pending" || !matches!(question.kind.as_str(), "approval" | "clarify") {
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
    let (choices, questions) = offered(question.choices.as_ref(), question.questions.as_ref());
    Ok(Approval {
        request_id: request_id.into(),
        description: description.into(),
        clarification: question.kind == "clarify",
        choices,
        questions,
        deletes: orbit_core::Deletion::from_wire(question.deletes.as_ref()),
    })
}

impl Runtime {
    /// The open conversation's session in a snapshot and its turn's state,
    /// or `None` when the snapshot decides nothing about it: an older
    /// revision, no conversation open, the session omitted, another turn
    /// admitted meanwhile, or a state this version does not know.
    pub(super) fn snapshot_turn<'a>(
        &mut self,
        snapshot: &'a SnapshotData,
    ) -> Result<
        Option<(
            String,
            &'a orbit_core::wire::hub::SnapshotSession,
            TurnState,
        )>,
    > {
        let revision = snapshot.revision;
        if self
            .assistant
            .snapshot_revision
            .is_some_and(|current| revision < current)
        {
            return Ok(None);
        }
        let sessions = &snapshot.sessions;
        if sessions.len() > 128 {
            bail!("Snapshot con {} sesiones; se admiten 128", sessions.len());
        }
        let Some(selected) = self.state.session_id().clone() else {
            self.assistant.snapshot_revision = Some(revision);
            return Ok(None);
        };
        let Some(session) = sessions
            .iter()
            .find(|session| session.session_id == selected)
        else {
            // Omitted sessions are not proof of completion or loss of ownership.
            return Ok(None);
        };
        if self
            .state
            .durable
            .assistant
            .active_request
            .as_deref()
            .is_some_and(|active| session.turn_id.as_deref() != Some(active))
        {
            // A snapshot emitted just before our new command reached the hub
            // must not erase that command's locally admitted identity.
            return Ok(None);
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
            return Ok(None);
        };
        Ok(Some((selected, session, turn_state)))
    }

    pub(in crate::daemon) async fn reconcile_typed_snapshot(
        &mut self,
        snapshot: &SnapshotData,
    ) -> Result<()> {
        let Some((selected, session, turn_state)) = self.snapshot_turn(snapshot)? else {
            return Ok(());
        };
        let revision = snapshot.revision;
        let snapshot_turn_id = session.turn_id.as_deref();
        let was_interrupted = snapshot_turn_id.is_some_and(|turn_id| {
            self.assistant
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
        next.durable.assistant.pending_approval = None;
        if turn_state.keeps_response_visible() && !was_interrupted {
            let turn_id = session
                .turn_id
                .as_deref()
                .context("Turno activo sin identidad")?;
            uuid::Uuid::parse_str(turn_id).context("Identidad de turno incompatible")?;
            next.durable.assistant.active_request = Some(turn_id.into());
            if let Some(question) = pending.first() {
                next.durable.assistant.pending_approval = Some(recovered_approval(question)?);
                next.set_notice(
                    self.scoped(
                        Notice::new(NoticeCode::ApprovalsRecovered)
                            .with(NoticeKey::Count, pending.len()),
                    ),
                );
            }
        } else {
            if turn_state == TurnState::OutcomeUnknown
                && next.durable.assistant.active_request.is_some()
            {
                // Otherwise the last thing said would stay «Deimos está
                // pensando…» for a turn the hub will never finish.
                next.set_notice(self.scoped(NoticeCode::AssistantOutcomeUnknown));
            }
            next.durable.assistant.active_request = None;
        }
        self.store_actor.snapshot(&next).await?;
        if !turn_state.keeps_response_visible()
            && let Some(turn_id) = snapshot_turn_id
        {
            self.assistant.interrupted_turns.forget(&selected, turn_id);
        }
        self.state = next;
        self.assistant.snapshot_revision = Some(revision);
        Ok(())
    }
}
