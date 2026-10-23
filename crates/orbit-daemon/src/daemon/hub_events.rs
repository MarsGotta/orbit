//! Hub event dispatch.
use super::*;
use orbit_core::notice::{Notice, NoticeCode, NoticeKey};
use orbit_core::wire::hub::{
    AgentsChangedData, CompletedData, DeltaData, ErrorData, HubEvent as WireHubEvent, HubEventKind,
    ItemData, ReadyData, RequiredData, SessionData,
};

#[path = "hub_decode.rs"]
mod decode;
use decode::safe_decode_error;

#[path = "hub_link_events.rs"]
mod link;

#[path = "hub_chat_events.rs"]
mod chat;

/// Bounds names retained only to suppress repeated diagnostics: 128 distinct
/// names cover eight times the 16 event names in hub v1. Keys hold at most 64
/// Unicode scalars (256 UTF-8 bytes), so the name payload is at most 32 KiB.
/// This is a memory/cardinality guard, not a wire or authorization limit.
/// Extra unknown events stay ignored.
const MAX_UNRECOGNIZED_HUB_EVENTS: usize = 128;

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
        self.assistant.snapshot_revision = None;
        self.link.protocol_version = None;
        self.link.unrecognized_events.clear();
        self.assistant.interaction_response = None;
        // The interrupt answer this connection owed can no longer
        // arrive; the interrupted-turn memory itself survives.
        self.assistant.interrupted_turns.forget_pending_ack();
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

    /// A turn's event counts only for the request in flight and its
    /// conversation; anything else about another turn is stale.
    fn admits_hub_frame(&self, frame: &WireHubEvent) -> bool {
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
            && (self.state.durable.assistant.active_request.is_none()
                || frame_id != self.state.durable.assistant.active_request.as_deref())
        {
            return false;
        }
        if matches!(
            frame.kind,
            HubEventKind::ChatDelta(_) | HubEventKind::ChatCompleted(_)
        ) && turn_session != self.state.session_id().as_deref()
        {
            return false;
        }
        if let Some(session) = turn_session
            && Some(session) != self.state.session_id().as_deref()
        {
            return false;
        }
        true
    }

    async fn on_hub_decoded_frame(&mut self, frame: WireHubEvent) -> Result<()> {
        if !self.admits_hub_frame(&frame) {
            return Ok(());
        }
        let frame_id = frame.id.as_deref();
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
            HubEventKind::ChatStatus(data) => self.on_hub_chat_status(&data),
            HubEventKind::ChatItem(data) => self.on_hub_chat_item(&data),
            HubEventKind::AgentsChanged(data) => self.on_hub_agents_changed(&data).await?,
            HubEventKind::ChatInterrupted(_) => {
                // The local interrupt already hid the turn. Its terminal state
                // is reconciled from state.snapshot, not this request ACK.
            }
            HubEventKind::Unrecognized { name } => self.note_unrecognized_hub_event(&name),
        }
        Ok(())
    }

    /// An event this daemon does not know is logged once per name, with
    /// both protocols, and otherwise ignored.
    fn note_unrecognized_hub_event(&mut self, name: &str) {
        let tracked_name: String = name.chars().take(64).collect();
        if self.link.unrecognized_events.len() < MAX_UNRECOGNIZED_HUB_EVENTS
            && self.link.unrecognized_events.insert(tracked_name)
        {
            let version = self
                .link
                .protocol_version
                .map_or("sin anunciar".into(), |value| value.to_string());
            if let Some(diagnostics) = &self.diagnostics {
                diagnostics.record(
                    "hub_event_unrecognized",
                    &[
                        ("name", name),
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
