//! The assistant's events from the hub: the turn's text, its steps and its
//! work, its end, the questions the agent asks, and the hub's errors.
use super::*;

impl Runtime {
    pub(super) fn on_hub_chat_status(&mut self, data: &orbit_core::wire::hub::StatusData) {
        // A progress notice is informational; only terminal receipts
        // or snapshots change the turn's authoritative state.
        if self.state.session_id().as_deref() == Some(data.session_id.as_str())
            && !data.text.trim().is_empty()
        {
            // The hub's progress line is about the conversation; the
            // chat shows the turn, so it is only recorded.
            self.say(NoticeCode::AssistantProgress);
        }
    }

    pub(super) async fn on_hub_chat_delta(&mut self, data: DeltaData) -> Result<()> {
        if let Some(request_id) = self.state.durable.assistant.active_request.clone() {
            self.apply(Event::Delta {
                request_id,
                text: data.text,
            })
            .await?;
        }
        Ok(())
    }

    /// One step of the turn in flight: frames of another turn never reach
    /// here (`on_hub_decoded_frame`), and a kind or phase this Mac does not
    /// know is recorded once by name and left out.
    pub(super) fn on_hub_chat_item(&mut self, data: &ItemData) {
        let Some(turn_id) = self.state.durable.assistant.active_request.clone() else {
            return;
        };
        let Some(item) = orbit_core::agent_work::WorkItem::from_wire(data) else {
            // Once per kind and phase, in the same bounded set as unknown events.
            let name: String = format!("chat.item:{}:{}", data.kind, data.phase)
                .chars()
                .take(64)
                .collect();
            if self.link.unrecognized_events.len() < MAX_UNRECOGNIZED_HUB_EVENTS
                && self.link.unrecognized_events.insert(name.clone())
            {
                self.log("hub_item_unknown", &[("name", &name)]);
            }
            return;
        };
        self.work_of(&turn_id).record(item);
    }

    /// The work of turn `turn_id`, begun when its question was sent; another
    /// turn's is let go.
    pub(super) fn work_of(&mut self, turn_id: &str) -> &mut orbit_core::agent_work::AgentWork {
        let started_at_ms = self
            .state
            .durable
            .assistant
            .turn
            .as_ref()
            .filter(|turn| turn.user.id == turn_id)
            .map_or_else(
                || u64::try_from(now()).unwrap_or_default(),
                |turn| turn.user.at_ms,
            );
        let work = self
            .assistant
            .work
            .take()
            .filter(|work| work.turn_id == turn_id)
            .unwrap_or_else(|| orbit_core::agent_work::AgentWork::new(turn_id, started_at_ms));
        self.assistant.work.insert(work)
    }

    /// The turn `request_id` ended now: the chat says how long it worked,
    /// with or without steps.
    pub(in crate::daemon) fn end_work(&mut self, request_id: &str) {
        let at_ms = u64::try_from(now()).unwrap_or_default();
        self.work_of(request_id).end(at_ms);
    }

    pub(super) async fn on_hub_chat_completed(&mut self, data: &CompletedData) -> Result<()> {
        if let Some(request_id) = self.state.durable.assistant.active_request.clone() {
            self.end_work(&request_id);
            if data.status.as_deref() == Some("error") {
                self.assistant.speak_next = false;
                self.meeting_question_turn_failed();
                self.apply(Event::Failure {
                    code: "assistant_error".into(),
                    numbers: Vec::new(),
                })
                .await?;
            } else {
                self.apply(Event::Completed {
                    request_id,
                    text: Some(data.text.clone()),
                })
                .await?;
                self.start_speech();
            }
        }
        Ok(())
    }

    /// A question the agent asks in the turn `turn_id`, the frame's `id`; its
    /// `request_id` is the request's own and never the turn's (hub-v1).
    pub(super) async fn on_hub_approval_required(
        &mut self,
        turn_id: Option<&str>,
        data: &RequiredData,
        clarification: bool,
        event_name: &str,
    ) -> Result<()> {
        let Some(turn_id) = turn_id else {
            return Ok(());
        };
        if self.state.durable.assistant.pending_approval.is_some() {
            return Ok(());
        }
        let Some(description) = data.description.as_deref().or(data.question.as_deref()) else {
            self.record_invalid_hub_frame(
                event_name,
                "missing field `description/question`: expected string",
            );
            return Ok(());
        };
        let (choices, questions) =
            super::interactions::offered(data.choices.as_ref(), data.questions.as_ref());
        self.apply(Event::Approval {
            turn_id: turn_id.into(),
            approval: Approval {
                request_id: data.request_id.clone(),
                description: description.into(),
                clarification,
                choices,
                questions,
                deletes: orbit_core::Deletion::from_wire(data.deletes.as_ref()),
            },
        })
        .await?;
        Ok(())
    }

    pub(super) async fn on_hub_error(
        &mut self,
        frame_id: Option<&str>,
        data: &ErrorData,
    ) -> Result<()> {
        // An error answering our own chat.interrupt
        // already said its cause with what to do about it; the
        // generic notice below must not talk over it.
        let interrupt_refused = self.interrupt_error(frame_id, data).await?;
        self.interaction_error(frame_id).await?;
        let code = data.code.as_deref().unwrap_or("not_reported");
        let fields = [("code", code)];
        if let Some(request_id) = self
            .state
            .durable
            .assistant
            .active_request
            .clone()
            .filter(|active| frame_id == Some(active.as_str()))
        {
            self.end_work(&request_id);
            self.meeting_question_hub_error(data.code.as_deref(), &data.message);
            self.assistant.speak_next = false;
            self.log("chat_rejected", &fields);
            // The refusal's numbers and closed tokens reach the person with
            // its code; its message, free text, never does.
            let numbers = data
                .refusal
                .as_ref()
                .map(|refusal| {
                    let unsigned = |value: Option<u64>| value.map(|value| value.to_string());
                    let signed = |value: Option<i64>| value.map(|value| value.to_string());
                    [
                        (NoticeKey::Bytes, unsigned(refusal.bytes)),
                        (NoticeKey::MaxBytes, unsigned(refusal.max_bytes)),
                        (NoticeKey::Count, unsigned(refusal.files)),
                        (NoticeKey::Max, unsigned(refusal.max_files)),
                        (NoticeKey::Stored, unsigned(refusal.session_bytes)),
                        (NoticeKey::ProviderStatus, unsigned(refusal.provider_status)),
                        (NoticeKey::RetryAfterMs, unsigned(refusal.retry_after_ms)),
                        // A refusal carries one deadline: the connection's or the answer's.
                        (
                            NoticeKey::TimeoutMs,
                            unsigned(refusal.connect_timeout_ms.or(refusal.timeout_ms)),
                        ),
                        (NoticeKey::HistoryChars, unsigned(refusal.history_chars)),
                        (NoticeKey::Tokens, unsigned(refusal.tokens)),
                        (NoticeKey::MaxTokens, unsigned(refusal.max_tokens)),
                        (NoticeKey::MaxRequests, unsigned(refusal.max_requests)),
                        (NoticeKey::AcpCode, signed(refusal.acp_code)),
                        (NoticeKey::ExitStatus, signed(refusal.exit_status)),
                        (NoticeKey::StopReason, refusal.stop_reason.clone()),
                        (NoticeKey::ResetsAtMs, unsigned(refusal.resets_at_ms)),
                    ]
                    .into_iter()
                    .filter_map(|(key, value)| value.map(|value| (key, value)))
                    .collect()
                })
                .unwrap_or_default();
            self.apply(Event::Failure {
                code: code.into(),
                numbers,
            })
            .await?;
        } else if !interrupt_refused {
            self.log("hub_error", &fields);
            self.say(Notice::new(NoticeCode::HubError).with_token(
                NoticeKey::Code,
                data.code.as_deref().unwrap_or("not_reported"),
            ));
        }
        Ok(())
    }
}
