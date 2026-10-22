//! Hub event dispatch.
use super::*;
use orbit_core::notice::{Notice, NoticeCode, NoticeKey};
use orbit_core::wire::hub::{
    AgentsChangedData, CompletedData, DeltaData, ErrorData, HubEvent as WireHubEvent, HubEventKind,
    ItemData, ReadyData, RequiredData, SessionData,
};

/// Bounds names retained only to suppress repeated diagnostics: 128 distinct
/// names cover eight times the 16 event names in hub v1. Keys hold at most 64
/// Unicode scalars (256 UTF-8 bytes), so the name payload is at most 32 KiB.
/// This is a memory/cardinality guard, not a wire or authorization limit.
/// Extra unknown events stay ignored.
const MAX_UNRECOGNIZED_HUB_EVENTS: usize = 128;

/// serde errors can quote the rejected value. Diagnostics retain only schema
/// fields and type classes, never a hub-supplied value or map key.
fn safe_decode_error(reason: &str) -> (&'static str, &'static str, &'static str) {
    const FIELDS: &[&str] = &[
        "v",
        "id",
        "data",
        "device_id",
        "hub_protocol",
        "hub_version",
        "device_version",
        "session_id",
        "turn_id",
        "text",
        "status",
        "kind",
        "state",
        "request_id",
        "description",
        "description/question",
        "question",
        "questions",
        "qid",
        "expires_at",
        "choices",
        "revision",
        "sessions",
        "pending",
        "omitted",
        "message",
        "persisted",
        "warning",
        "applied",
        "resolved",
        "remaining",
        "stale_interaction",
        "capability",
        "parameters",
        "code",
        "refusal",
        "retry",
        "ms",
        "operation",
        "resource",
        "bytes",
        "max_bytes",
        "retry_after_ms",
        "last_seen_ms",
        "waited_ms",
        "deadline_ms",
        "item_id",
        "phase",
        "label",
        "subject",
        "steps",
        "cause",
        "multi_select",
        "storage",
        "used_bytes",
        "remaining_bytes",
        "limit",
        "nearly_full",
    ];
    let path = reason.split_once(':').map_or(reason, |(path, _)| path);
    let field_name = reason
        .split_once("missing field `")
        .and_then(|(_, tail)| tail.split_once('`').map(|(field, _)| field))
        .or_else(|| path.rsplit(['.', '[', ']']).find(|part| !part.is_empty()))
        .unwrap_or("data");
    let field = FIELDS
        .iter()
        .copied()
        .find(|known| *known == field_name)
        .unwrap_or("data");
    let actual = if reason.contains("missing field") {
        "missing"
    } else {
        match reason.split_once("invalid type: ").map(|(_, tail)| tail) {
            Some(tail) if tail.starts_with("string") => "string",
            Some(tail) if tail.starts_with("integer") => "integer",
            Some(tail) if tail.starts_with("floating point") => "number",
            Some(tail) if tail.starts_with("boolean") => "boolean",
            Some(tail) if tail.starts_with("null") => "null",
            Some(tail) if tail.starts_with("sequence") => "array",
            Some(tail) if tail.starts_with("map") => "object",
            Some(_) | None => "invalid",
        }
    };
    let expected = match reason.split_once("expected ").map(|(_, tail)| tail) {
        Some(tail) if tail.starts_with("a string") || tail.starts_with("string") => "string",
        Some(tail)
            if tail.starts_with("u64")
                || tail.starts_with("i64")
                || tail.starts_with("u8")
                || tail.starts_with("integer") =>
        {
            "integer"
        }
        Some(tail) if tail.starts_with("a sequence") || tail.starts_with("array") => "array",
        Some(tail) if tail.starts_with("a map") || tail.starts_with("object") => "object",
        Some(tail) if tail.starts_with("a boolean") || tail.starts_with("boolean") => "boolean",
        Some(_) | None => "unknown",
    };
    (field, actual, expected)
}

impl Runtime {
    pub(super) async fn hub_event(&mut self, event: HubEvent) -> Result<()> {
        if self.state.privacy_stopped() {
            return Ok(());
        }
        match event {
            HubEvent::Connected => self.on_hub_connected().await?,
            HubEvent::Disconnected => self.on_hub_disconnected().await?,
            HubEvent::Problem(message) => {
                // The link's state is on screen already; the cause is logged.
                self.log("hub_problem", &[("cause", &message)]);
                self.say(NoticeCode::HubProblem);
            }
            HubEvent::Revoked => self.apply(Event::Connect(Connection::Revoked)).await?,
            HubEvent::Frame(frame) => self.on_hub_frame(frame).await?,
            HubEvent::Decoded(frame) => self.on_hub_decoded_frame(*frame).await?,
            HubEvent::InvalidFrame { event, reason } => {
                self.record_invalid_hub_frame(&event, &reason)
            }
        }
        // A meeting question's turn may have ended with this event.
        self.follow_meeting_question();
        Ok(())
    }

    async fn on_hub_connected(&mut self) -> Result<()> {
        self.apply(Event::HubTransportReady).await?;
        self.snapshot_revision = None;
        self.hub_protocol_version = None;
        self.unrecognized_hub_events.clear();
        self.conversation.interaction_response = None;
        // The interrupt answer this connection owed can no longer
        // arrive; the interrupted-turn memory itself survives.
        self.conversation.interrupted_turns.forget_pending_ack();
        self.apply(Event::Connect(Connection::Connecting)).await?;
        Ok(())
    }

    async fn on_hub_disconnected(&mut self) -> Result<()> {
        // The engine and a claimed input do not depend on the hub.
        // A request still waiting on the hub is released below.
        // This path also retires pending bridge and journal work. A later
        // ready(Online) only closes its local verticals and context.
        self.apply(Event::HubDisconnected).await?;
        Ok(())
    }

    async fn on_hub_frame(&mut self, mut frame: Value) -> Result<()> {
        // The socket has already checked v. Older synthetic in-process callers
        // submit Frame directly without a version; preserve that internal API.
        if let Some(object) = frame.as_object_mut() {
            object.entry("v").or_insert_with(|| json!(1));
        }
        match WireHubEvent::decode(frame.clone()) {
            Ok(decoded) => self.on_hub_decoded_frame(decoded).await,
            Err(reason) => {
                if frame.get("event").and_then(Value::as_str) == Some("device.request")
                    && let Some(data) = frame.get("data")
                    && data.get("request_id").and_then(Value::as_str).is_some()
                {
                    return self.device_request_raw(data).await;
                }
                self.record_invalid_hub_frame(crate::hub::invalid_event_name(&reason), &reason);
                Ok(())
            }
        }
    }

    fn record_invalid_hub_frame(&self, event: &str, reason: &str) {
        if let Some(diagnostics) = &self.diagnostics {
            let (field, actual, expected) = safe_decode_error(reason);
            diagnostics.record(
                "hub_event_invalid",
                &[
                    ("event", event),
                    ("field", field),
                    ("actual_type", actual),
                    ("expected_type", expected),
                ],
            );
        }
    }

    async fn on_hub_decoded_frame(&mut self, frame: WireHubEvent) -> Result<()> {
        let frame_id = frame.id.as_deref();
        let turn_session = match &frame.kind {
            HubEventKind::ChatSession(_) => None,
            HubEventKind::ChatDelta(data) => data.session_id.as_deref(),
            HubEventKind::ChatCompleted(data) => data.session_id.as_deref(),
            HubEventKind::ApprovalRequired(data) | HubEventKind::ClarifyRequired(data) => {
                Some(data.session_id.as_str())
            }
            HubEventKind::ChatItem(data) => Some(data.session_id.as_str()),
            HubEventKind::Ready(_)
            | HubEventKind::Pong(_)
            | HubEventKind::ChatInterrupted(_)
            | HubEventKind::ChatStatus(_)
            | HubEventKind::InteractionResolved(_)
            | HubEventKind::InteractionExpired(_)
            | HubEventKind::DeviceRequest(_)
            | HubEventKind::StateSnapshot(_)
            | HubEventKind::Error(_)
            | HubEventKind::Revoked(_)
            | HubEventKind::RecordingChanged(_)
            | HubEventKind::StorageStatus(_)
            | HubEventKind::AgentsChanged(_)
            | HubEventKind::Unrecognized { .. } => None,
        };
        let is_turn_event = matches!(
            frame.kind,
            HubEventKind::ChatSession(_)
                | HubEventKind::ChatDelta(_)
                | HubEventKind::ChatCompleted(_)
                | HubEventKind::ApprovalRequired(_)
                | HubEventKind::ClarifyRequired(_)
                | HubEventKind::ChatItem(_)
        );
        if is_turn_event
            && (self.state.durable.active_request.is_none()
                || frame_id != self.state.durable.active_request.as_deref())
        {
            return Ok(());
        }
        if matches!(
            frame.kind,
            HubEventKind::ChatDelta(_) | HubEventKind::ChatCompleted(_)
        ) && turn_session != self.state.session_id().as_deref()
        {
            return Ok(());
        }
        if let Some(session) = turn_session
            && Some(session) != self.state.session_id().as_deref()
        {
            return Ok(());
        }
        match frame.kind {
            HubEventKind::StateSnapshot(data) => self.reconcile_typed_snapshot(&data).await?,
            HubEventKind::InteractionResolved(data) => {
                self.interaction_result_typed(true, frame_id, &data).await?
            }
            HubEventKind::InteractionExpired(data) => {
                self.interaction_result_typed(false, frame_id, &data)
                    .await?
            }
            HubEventKind::Ready(data) => self.on_hub_ready(&data).await?,
            HubEventKind::Pong(_) => {}
            HubEventKind::Revoked(_) => self.apply(Event::Connect(Connection::Revoked)).await?,
            HubEventKind::RecordingChanged(data) => self.on_recording_changed(&data.recording_id),
            HubEventKind::StorageStatus(storage) => self.on_hub_storage(&storage),
            HubEventKind::ChatSession(SessionData { session_id }) => {
                let agent_id = self.turn_agent_id();
                self.apply(Event::Session {
                    id: session_id,
                    agent_id,
                })
                .await?;
            }
            HubEventKind::ChatDelta(data) => self.on_hub_chat_delta(data).await?,
            HubEventKind::ChatCompleted(data) => self.on_hub_chat_completed(&data).await?,
            HubEventKind::ApprovalRequired(data) => {
                self.on_hub_approval_required(frame_id, &data, false, "approval.required")
                    .await?
            }
            HubEventKind::ClarifyRequired(data) => {
                self.on_hub_approval_required(frame_id, &data, true, "clarify.required")
                    .await?
            }
            HubEventKind::DeviceRequest(data) => self.device_request(&data).await?,
            HubEventKind::Error(data) => self.on_hub_error(frame_id, &data).await?,
            HubEventKind::ChatStatus(data) => {
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
            HubEventKind::ChatItem(data) => self.on_hub_chat_item(&data),
            HubEventKind::AgentsChanged(data) => self.on_hub_agents_changed(&data).await?,
            HubEventKind::ChatInterrupted(_) => {
                // The local interrupt already hid the turn. Its terminal state
                // is reconciled from state.snapshot, not this request ACK.
            }
            HubEventKind::Unrecognized { name } => {
                let tracked_name: String = name.chars().take(64).collect();
                if self.unrecognized_hub_events.len() < MAX_UNRECOGNIZED_HUB_EVENTS
                    && self.unrecognized_hub_events.insert(tracked_name)
                {
                    let version = self
                        .hub_protocol_version
                        .map_or("sin anunciar".into(), |value| value.to_string());
                    if let Some(diagnostics) = &self.diagnostics {
                        diagnostics.record(
                            "hub_event_unrecognized",
                            &[
                                ("name", &name),
                                (
                                    "daemon_protocol",
                                    &orbit_core::wire::hub::HUB_PROTOCOL.to_string(),
                                ),
                                ("hub_protocol", &version),
                            ],
                        );
                    }
                }
            }
        }
        Ok(())
    }

    async fn on_hub_ready(&mut self, data: &ReadyData) -> Result<()> {
        self.hub_protocol_version = data.hub_protocol;
        if !self.settings.device_id.as_deref().is_some_and(|expected| {
            !expected.is_empty() && data.device_id.as_deref() == Some(expected)
        }) {
            self.apply(Event::Connect(Connection::Revoked)).await?;
            if data.device_id.is_none() {
                self.record_invalid_hub_frame(
                    "ready",
                    "missing field `device_id`: expected string",
                );
                self.say(NoticeCode::HubDeviceIdentityMissing);
            } else if self
                .settings
                .device_id
                .as_deref()
                .is_some_and(|expected| !expected.is_empty())
            {
                self.say(
                    Notice::new(NoticeCode::DevicePairingMismatch)
                        .with_token(NoticeKey::Cause, "other_device"),
                );
            } else {
                self.say(
                    Notice::new(NoticeCode::DevicePairingMismatch)
                        .with_token(NoticeKey::Cause, "no_local_identity"),
                );
            }
        } else {
            let mut settings = self.settings.clone();
            if let Some(name) = &data.assistant_name {
                settings.assistant_name = Some(name.clone());
            }
            settings.hub_agents = self.learned_agents(data);
            // What this hub takes now: a hub that names no ceilings has no
            // attachments (0.9.0), and the composer offers none.
            self.conversation.hub_attachments = data.attachments;
            if settings.assistant_name != self.settings.assistant_name
                || settings.hub_agents != self.settings.hub_agents
            {
                self.paths.save_settings(&settings)?;
                self.settings = settings;
                if let Some(stream) = self.features.question.wake_stream.clone() {
                    self.send_wake_order(stream, true).await;
                }
            }
            self.notes_hub_ready();
            self.apply(Event::Connect(Connection::Online)).await?;
            if let Some(storage) = &data.storage {
                self.on_hub_storage(storage);
            }
        }
        Ok(())
    }

    /// What the hub's storage is: said once when the hub becomes nearly full
    /// (`orbit_core::hub_storage`), with its numbers.
    fn on_hub_storage(&mut self, storage: &orbit_core::wire::hub::HubStorage) {
        if let Some(notice) =
            orbit_core::hub_storage::storage_notice(self.hub_storage_nearly_full, storage)
        {
            self.say(notice);
        }
        self.hub_storage_nearly_full = Some(storage.nearly_full);
    }

    /// The hub's agents changed (a profile, one added or removed, an
    /// account): the Mac keeps the new list, and the wake phrase hears its
    /// names, as after `ready`.
    async fn on_hub_agents_changed(&mut self, data: &AgentsChangedData) -> Result<()> {
        let Some(roster) = self.admitted_agents(
            &data.agents,
            data.default_agent_id.clone(),
            data.addable_agents.clone(),
        ) else {
            return Ok(());
        };
        if self.settings.hub_agents.as_ref() == Some(&roster) {
            return Ok(());
        }
        let mut settings = self.settings.clone();
        settings.assistant_name = Some(roster.default_name().to_owned());
        settings.hub_agents = Some(roster);
        self.paths.save_settings(&settings)?;
        self.settings = settings;
        if let Some(stream) = self.features.question.wake_stream.clone() {
            self.send_wake_order(stream, true).await;
        }
        Ok(())
    }

    /// The agents this hub has, from its `ready`: none listed is a hub
    /// with one (0.9.0); a list the Mac cannot rely on is logged with its
    /// cause and numbers, and the one the Mac had stays.
    fn learned_agents(&self, data: &ReadyData) -> Option<orbit_core::agents::AgentRoster> {
        self.admitted_agents(
            data.agents.as_ref()?,
            data.default_agent_id.clone(),
            data.addable_agents.clone(),
        )
    }

    /// A list of agents as `ready` and `agents.changed` give it, admitted;
    /// one the Mac cannot rely on is logged with its cause and numbers, and
    /// the one the Mac had stays.
    fn admitted_agents(
        &self,
        listed: &[orbit_core::wire::hub::HubAgent],
        default_agent_id: Option<String>,
        addable: Option<Vec<String>>,
    ) -> Option<orbit_core::agents::AgentRoster> {
        let agents = listed
            .iter()
            .map(|agent| orbit_core::agents::Agent {
                id: agent.id.clone(),
                name: agent.name.clone(),
                voice: Some(agent.capabilities.iter().any(|name| name == "voice")),
                service: agent.service.clone(),
                description: agent.description.clone(),
                color: agent.color.clone(),
                removable: agent.removable.unwrap_or(false),
                account: agent
                    .account
                    .as_deref()
                    .map(orbit_core::agents::AgentAccount::from_wire),
                account_email: agent.account_email.clone(),
                plan: agent.plan.clone(),
                limit: agent
                    .limit
                    .as_ref()
                    .map(|limit| orbit_core::agents::AgentLimit {
                        until_ms: limit.until_ms,
                    }),
                usage: agent.usage.clone(),
            })
            .collect();
        match orbit_core::agents::AgentRoster::admit(agents, default_agent_id) {
            Ok(roster) => Some(roster.with_addable(addable)),
            Err(refusal) => {
                let numbers = refusal
                    .numbers()
                    .into_iter()
                    .map(|(key, value)| (key, value.to_string()))
                    .collect::<Vec<_>>();
                let mut fields = vec![("cause", refusal.code())];
                fields.extend(numbers.iter().map(|(key, value)| (*key, value.as_str())));
                self.log("hub_agents_refused", &fields);
                self.settings.hub_agents.clone()
            }
        }
    }

    async fn on_hub_chat_delta(&mut self, data: DeltaData) -> Result<()> {
        if let Some(request_id) = self.state.durable.active_request.clone() {
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
    fn on_hub_chat_item(&mut self, data: &ItemData) {
        let Some(turn_id) = self.state.durable.active_request.clone() else {
            return;
        };
        let Some(item) = orbit_core::agent_work::WorkItem::from_wire(data) else {
            // Once per kind and phase, in the same bounded set as unknown events.
            let name: String = format!("chat.item:{}:{}", data.kind, data.phase)
                .chars()
                .take(64)
                .collect();
            if self.unrecognized_hub_events.len() < MAX_UNRECOGNIZED_HUB_EVENTS
                && self.unrecognized_hub_events.insert(name.clone())
            {
                self.log("hub_item_unknown", &[("name", &name)]);
            }
            return;
        };
        self.work_of(&turn_id).record(item);
    }

    /// The work of turn `turn_id`, begun when its question was sent; another
    /// turn's is let go.
    fn work_of(&mut self, turn_id: &str) -> &mut orbit_core::agent_work::AgentWork {
        let started_at_ms = self
            .state
            .durable
            .turn
            .as_ref()
            .filter(|turn| turn.user.id == turn_id)
            .map_or_else(
                || u64::try_from(now()).unwrap_or_default(),
                |turn| turn.user.at_ms,
            );
        let work = self
            .conversation
            .work
            .take()
            .filter(|work| work.turn_id == turn_id)
            .unwrap_or_else(|| orbit_core::agent_work::AgentWork::new(turn_id, started_at_ms));
        self.conversation.work.insert(work)
    }

    /// The turn `request_id` ended now: the chat says how long it worked,
    /// with or without steps.
    pub(super) fn end_work(&mut self, request_id: &str) {
        let at_ms = u64::try_from(now()).unwrap_or_default();
        self.work_of(request_id).end(at_ms);
    }

    async fn on_hub_chat_completed(&mut self, data: &CompletedData) -> Result<()> {
        if let Some(request_id) = self.state.durable.active_request.clone() {
            self.end_work(&request_id);
            if data.status.as_deref() == Some("error") {
                self.conversation.speak_next = false;
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
    async fn on_hub_approval_required(
        &mut self,
        turn_id: Option<&str>,
        data: &RequiredData,
        clarification: bool,
        event_name: &str,
    ) -> Result<()> {
        let Some(turn_id) = turn_id else {
            return Ok(());
        };
        if self.state.durable.pending_approval.is_some() {
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
            },
        })
        .await?;
        Ok(())
    }

    async fn on_hub_error(&mut self, frame_id: Option<&str>, data: &ErrorData) -> Result<()> {
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
            .active_request
            .clone()
            .filter(|active| frame_id == Some(active.as_str()))
        {
            self.end_work(&request_id);
            self.meeting_question_hub_error(data.code.as_deref(), &data.message);
            self.conversation.speak_next = false;
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
