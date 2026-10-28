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
            "Interaction snapshot has a request_id of {} bytes; allowed range is 1 to 160",
            request_id.len()
        );
    }
    if question.state != "pending" || !matches!(question.kind.as_str(), "approval" | "clarify") {
        bail!("Incompatible interaction snapshot");
    }
    let description = question
        .description
        .as_deref()
        .or(question.question.as_deref())
        .context("Interaction snapshot has no description or question")?;
    if description.len() > 16_000 {
        bail!(
            "Pending question is {} bytes; 16000 allowed",
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
            bail!("Snapshot has {} sessions; 128 allowed", sessions.len());
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
        self.remember_waiting_sessions(snapshot);
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
                "Snapshot has {} pending interactions; 128 allowed",
                pending.len()
            );
        }
        next.durable.assistant.pending_approval = None;
        if turn_state.keeps_response_visible() && !was_interrupted {
            let turn_id = session
                .turn_id
                .as_deref()
                .context("Active turn has no identity")?;
            uuid::Uuid::parse_str(turn_id).context("Incompatible turn identity")?;
            next.durable.assistant.active_request = Some(turn_id.into());
            if let Some(turn) = &session.turn {
                next.durable.assistant.turn = Some(recovered_turn(turn, turn_id)?);
            }
            self.assistant.retry = retry_activity(session.retry.as_deref());
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
            self.assistant.retry = None;
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

/// Only closed progress tokens reach presentation, never provider diagnostics.
pub(in crate::daemon) fn retry_activity(
    kind: Option<&str>,
) -> Option<orbit_core::topics::assistant::Activity> {
    use orbit_core::topics::assistant::Activity;
    match kind {
        Some("retrying_openai") => Some(Activity::RetryingOpenai),
        Some("retrying_anthropic") => Some(Activity::RetryingAnthropic),
        _ => None,
    }
}

impl Runtime {
    fn remember_waiting_sessions(&mut self, snapshot: &SnapshotData) {
        if snapshot.sessions.len() > 128
            || self
                .assistant
                .waiting_revision
                .is_some_and(|r| snapshot.revision < r)
        {
            return;
        }
        self.assistant.waiting_revision = Some(snapshot.revision);
        // A targeted observation is partial; absence never clears another session.
        if snapshot.omitted == Some(false) {
            self.assistant.waiting_sessions.clear();
        }
        for session in &snapshot.sessions {
            self.assistant.waiting_sessions.remove(&session.session_id);
            if let Some(pending) = session
                .pending
                .iter()
                .find(|p| p.state == "pending" && matches!(p.kind.as_str(), "approval" | "clarify"))
                && self.assistant.waiting_sessions.len() < 128
            {
                self.assistant
                    .waiting_sessions
                    .insert(session.session_id.clone(), pending.kind.clone());
            }
        }
    }
}

/// The observed turn has the selected identity and the existing text ceilings.
fn recovered_turn(turn: &orbit_core::Turn, turn_id: &str) -> Result<orbit_core::Turn> {
    if turn.user.id != turn_id {
        bail!("Turn snapshot user.id differs from turn_id");
    }
    for (field, text, max_bytes) in [
        (
            "user.text",
            turn.user.text.as_str(),
            orbit_core::MAX_TEXT_BYTES,
        ),
        (
            "answer.text",
            turn.answer.as_ref().map_or("", |a| a.text.as_str()),
            orbit_core::MAX_TEXT_BYTES * 3,
        ),
    ] {
        if text.len() > max_bytes {
            bail!(
                "Snapshot {field} is {} bytes; {max_bytes} allowed",
                text.len()
            );
        }
    }
    Ok(turn.clone())
}
