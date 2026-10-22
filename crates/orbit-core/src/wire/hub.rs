//! Typed hub-v1 WebSocket frames. Unknown names survive mixed-version deployments.
use super::bridge::{DeviceReplyParams, DeviceRequestData};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;

pub const HUB_PROTOCOL: u8 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "method")]
pub enum HubRequest {
    #[serde(rename = "chat.send")]
    ChatSend {
        v: u8,
        id: String,
        params: ChatSendParams,
    },
    #[serde(rename = "chat.interrupt")]
    ChatInterrupt {
        v: u8,
        id: String,
        params: InterruptParams,
    },
    #[serde(rename = "approval.respond")]
    ApprovalRespond {
        v: u8,
        id: String,
        params: ApprovalParams,
    },
    #[serde(rename = "clarify.respond")]
    ClarifyRespond {
        v: u8,
        id: String,
        params: ClarifyParams,
    },
    #[serde(rename = "ping")]
    Ping {
        v: u8,
        id: String,
        params: EmptyParams,
    },
    #[serde(rename = "device.reply")]
    DeviceReply {
        v: u8,
        id: String,
        params: Box<DeviceReplyParams>,
    },
    #[serde(skip)]
    Unrecognized { name: String },
}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct EmptyParams {}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChatSendParams {
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub call_active: Option<bool>,
    /// The Mac's id for the message, kept in the hub's row so the
    /// Mac finds its own turn and its voice note's audio by it.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub client_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub voice: Option<ChatVoice>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub meeting: Option<ChatMeeting>,
    /// The agent the turn is for (hub-v1 «Agents»). Sent only to a hub
    /// that listed its agents in `ready`: a 0.9.0 hub refuses the field.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub agent_id: Option<String>,
}
/// The turn is a voice note of this length; its audio stays on the Mac.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatVoice {
    pub duration_ms: u64,
}
/// The turn is a question about this meeting. The hub adds its last
/// `minutes` of transcript and keeps only the question and its mark; the
/// title travels because the hub learns it only when the recording closes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatMeeting {
    pub recording_id: String,
    pub minutes: u8,
    pub title: String,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InterruptParams {
    pub session_id: String,
    pub turn_id: String,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ApprovalParams {
    pub session_id: String,
    pub request_id: String,
    pub approved: bool,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub turn_id: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClarifyParams {
    pub session_id: String,
    pub request_id: String,
    pub answer: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub question_id: Option<String>,
}
impl HubRequest {
    pub fn new(id: String, method: &str, params: Value) -> Result<Self, String> {
        fn typed<T: serde::de::DeserializeOwned>(params: Value, method: &str) -> Result<T, String> {
            serde_json::from_value(params).map_err(|error| format!("{method}.params: {error}"))
        }
        let v = HUB_PROTOCOL;
        Ok(match method {
            "chat.send" => Self::ChatSend {
                v,
                id,
                params: typed(params, method)?,
            },
            "chat.interrupt" => Self::ChatInterrupt {
                v,
                id,
                params: typed(params, method)?,
            },
            "approval.respond" => Self::ApprovalRespond {
                v,
                id,
                params: typed(params, method)?,
            },
            "clarify.respond" => Self::ClarifyRespond {
                v,
                id,
                params: typed(params, method)?,
            },
            "ping" => Self::Ping {
                v,
                id,
                params: typed(params, method)?,
            },
            "device.reply" => Self::DeviceReply {
                v,
                id,
                params: Box::new(typed(params, method)?),
            },
            _ => return Err(format!("Método de hub no compatible: {method}")),
        })
    }

    pub fn decode(value: Value) -> Result<Self, String> {
        let version = value
            .get("v")
            .and_then(Value::as_u64)
            .ok_or("hub request.v: expected integer")?;
        if version != u64::from(HUB_PROTOCOL) {
            return Err(format!(
                "hub request.v: expected {HUB_PROTOCOL}, got {version}"
            ));
        }
        let name = value
            .get("method")
            .and_then(Value::as_str)
            .ok_or("hub request.method: expected string")?
            .to_owned();
        const NAMES: &[&str] = &[
            "chat.send",
            "chat.interrupt",
            "approval.respond",
            "clarify.respond",
            "ping",
            "device.reply",
        ];
        if !NAMES.contains(&name.as_str()) {
            return Ok(Self::Unrecognized { name });
        }
        serde_json::from_value(value).map_err(|error| format!("{name}: {error}"))
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReadyData {
    /// The default agent's name, kept for Macs that do not read `agents`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assistant_name: Option<String>,
    /// The hub's agents in its order; absent from a 0.9.0 hub, which has
    /// one, named by `assistant_name`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agents: Option<Vec<HubAgent>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_agent_id: Option<String>,
    #[serde(default)]
    pub device_id: Option<String>,
    #[serde(default)]
    pub hub_protocol: Option<u64>,
    #[serde(default)]
    pub hub_version: Option<String>,
    #[serde(default)]
    pub device_version: Option<String>,
}
/// One agent as `ready` lists it. The Mac reads `id` and `name`; `kind`
/// and `capabilities` are the hub's and travel for the contract.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HubAgent {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub capabilities: Vec<String>,
}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct EmptyData {}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionData {
    pub session_id: String,
}
/// Something of one of this device's recordings changed on the hub
/// (a group transcribed or failed, its note, its speakers' names, its private
/// notes). The Mac reads that recording again; the event carries no content.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecordingChangedData {
    pub recording_id: String,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeltaData {
    #[serde(default)]
    pub session_id: Option<String>,
    pub text: String,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CompletedData {
    #[serde(default)]
    pub session_id: Option<String>,
    pub text: String,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub persisted: Option<bool>,
    #[serde(default)]
    pub warning: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StatusData {
    pub session_id: String,
    pub kind: String,
    pub text: String,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InterruptedData {
    pub session_id: String,
    pub status: String,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PendingInteraction {
    pub request_id: String,
    pub kind: String,
    pub state: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub question: Option<String>,
    #[serde(default)]
    pub expires_at: Option<u64>,
    #[serde(default)]
    pub choices: Option<Vec<String>>,
    #[serde(default)]
    pub questions: Option<Vec<Value>>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SnapshotSession {
    pub session_id: String,
    #[serde(default)]
    pub turn_id: Option<String>,
    pub status: String,
    pub pending: Vec<PendingInteraction>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SnapshotData {
    pub revision: u64,
    pub sessions: Vec<SnapshotSession>,
    #[serde(default)]
    pub omitted: Option<bool>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RequiredData {
    pub session_id: String,
    pub request_id: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub question: Option<String>,
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub state: Option<String>,
    #[serde(default)]
    pub expires_at: Option<u64>,
    #[serde(default)]
    pub choices: Option<Vec<String>>,
    #[serde(default)]
    pub questions: Option<Vec<Value>>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InteractionData {
    pub session_id: String,
    pub request_id: String,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub applied: Option<bool>,
    #[serde(default)]
    pub resolved: Option<u64>,
    #[serde(default)]
    pub remaining: Option<Vec<String>>,
    #[serde(default)]
    pub stale_interaction: Option<bool>,
    #[serde(default)]
    pub turn_id: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ErrorData {
    #[serde(default)]
    pub code: Option<String>,
    pub message: String,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub numbers: Option<std::collections::BTreeMap<String, ErrorDetail>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refusal: Option<RefusalData>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ErrorDetail {
    Number(i64),
    Text(String),
    Flag(bool),
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RefusalData {
    pub code: Option<String>,
    pub retry: RetryDisposition,
    #[serde(default)]
    pub operation: Option<String>,
    #[serde(default)]
    pub resource: Option<String>,
    #[serde(default)]
    pub bytes: Option<u64>,
    #[serde(default)]
    pub max_bytes: Option<u64>,
    #[serde(default)]
    pub retry_after_ms: Option<u64>,
    #[serde(default)]
    pub last_seen_ms: Option<u64>,
    #[serde(default)]
    pub waited_ms: Option<u64>,
    #[serde(default)]
    pub deadline_ms: Option<u64>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RetryDisposition {
    AfterMs { ms: u64 },
    AfterFix,
    CheckOutcome,
    Never,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", content = "data")]
pub enum HubEventKind {
    #[serde(rename = "ready")]
    Ready(ReadyData),
    #[serde(rename = "pong")]
    Pong(EmptyData),
    #[serde(rename = "chat.interrupted")]
    ChatInterrupted(InterruptedData),
    #[serde(rename = "chat.session")]
    ChatSession(SessionData),
    #[serde(rename = "chat.delta")]
    ChatDelta(DeltaData),
    #[serde(rename = "chat.completed")]
    ChatCompleted(CompletedData),
    #[serde(rename = "chat.status")]
    ChatStatus(StatusData),
    #[serde(rename = "approval.required")]
    ApprovalRequired(RequiredData),
    #[serde(rename = "clarify.required")]
    ClarifyRequired(RequiredData),
    #[serde(rename = "interaction.resolved")]
    InteractionResolved(InteractionData),
    #[serde(rename = "interaction.expired")]
    InteractionExpired(InteractionData),
    #[serde(rename = "device.request")]
    DeviceRequest(DeviceRequestData),
    #[serde(rename = "state.snapshot")]
    StateSnapshot(SnapshotData),
    #[serde(rename = "error")]
    Error(ErrorData),
    #[serde(rename = "revoked")]
    Revoked(EmptyData),
    #[serde(rename = "recording.changed")]
    RecordingChanged(RecordingChangedData),
    #[serde(skip)]
    Unrecognized { name: String },
}
#[derive(Debug, Clone, PartialEq)]
pub struct HubEvent {
    pub id: Option<String>,
    pub kind: HubEventKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HubEnvelopeError {
    NotObject,
    MissingOrInvalidVersion,
    UnsupportedVersion { received: u64 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HubDecodeError {
    Envelope(HubEnvelopeError),
    Event(String),
}

impl std::fmt::Display for HubDecodeError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Envelope(HubEnvelopeError::NotObject) => {
                formatter.write_str("hub frame: expected object")
            }
            Self::Envelope(HubEnvelopeError::MissingOrInvalidVersion) => {
                formatter.write_str("hub frame.v: expected integer")
            }
            Self::Envelope(HubEnvelopeError::UnsupportedVersion { received }) => {
                write!(
                    formatter,
                    "hub frame.v: expected {HUB_PROTOCOL}, got {received}"
                )
            }
            Self::Event(reason) => formatter.write_str(reason),
        }
    }
}

impl From<String> for HubDecodeError {
    fn from(reason: String) -> Self {
        Self::Event(reason)
    }
}

#[derive(Serialize, Deserialize)]
struct KnownEventFrame {
    v: u8,
    #[serde(default)]
    id: Option<String>,
    #[serde(flatten)]
    kind: HubEventKind,
}
impl HubEvent {
    pub fn decode(value: Value) -> Result<Self, String> {
        Self::decode_detailed(value).map_err(|error| error.to_string())
    }

    pub fn decode_detailed(value: Value) -> Result<Self, HubDecodeError> {
        let object = value
            .as_object()
            .ok_or(HubDecodeError::Envelope(HubEnvelopeError::NotObject))?;
        let version = object
            .get("v")
            .and_then(Value::as_u64)
            .ok_or(HubDecodeError::Envelope(
                HubEnvelopeError::MissingOrInvalidVersion,
            ))?;
        if version != u64::from(HUB_PROTOCOL) {
            return Err(HubDecodeError::Envelope(
                HubEnvelopeError::UnsupportedVersion { received: version },
            ));
        }
        let name = object
            .get("event")
            .and_then(Value::as_str)
            .ok_or_else(|| HubDecodeError::Event("hub frame.event: expected string".into()))?;
        let id = match object.get("id") {
            Some(Value::String(id)) => Some(id.clone()),
            Some(Value::Null) | None => None,
            Some(_) => {
                return Err(HubDecodeError::Event(format!(
                    "{name}.id: expected string or null"
                )));
            }
        };
        let data = object
            .get("data")
            .ok_or_else(|| format!("{name}.data: expected object"))?;
        macro_rules! decode {
            ($data_type:ty, $variant:ident) => {
                decode_data::<$data_type>(data, name).map(HubEventKind::$variant)?
            };
        }
        let kind = match name {
            "ready" => decode!(ReadyData, Ready),
            "pong" => decode!(EmptyData, Pong),
            "chat.interrupted" => decode!(InterruptedData, ChatInterrupted),
            "chat.session" => decode!(SessionData, ChatSession),
            "chat.delta" => decode!(DeltaData, ChatDelta),
            "chat.completed" => decode!(CompletedData, ChatCompleted),
            "chat.status" => decode!(StatusData, ChatStatus),
            "approval.required" => decode!(RequiredData, ApprovalRequired),
            "clarify.required" => decode!(RequiredData, ClarifyRequired),
            "interaction.resolved" => decode!(InteractionData, InteractionResolved),
            "interaction.expired" => decode!(InteractionData, InteractionExpired),
            "device.request" => decode!(DeviceRequestData, DeviceRequest),
            "state.snapshot" => decode!(SnapshotData, StateSnapshot),
            "error" => decode!(ErrorData, Error),
            "revoked" => decode!(EmptyData, Revoked),
            "recording.changed" => decode!(RecordingChangedData, RecordingChanged),
            _ => HubEventKind::Unrecognized { name: name.into() },
        };
        Ok(Self { id, kind })
    }
}

fn decode_data<T: serde::de::DeserializeOwned>(data: &Value, name: &str) -> Result<T, String> {
    let text = serde_json::to_string(data).map_err(|error| error.to_string())?;
    let mut deserializer = serde_json::Deserializer::from_str(&text);
    serde_path_to_error::deserialize(&mut deserializer)
        .map_err(|error| format!("{name}.data.{error}"))
}
impl Serialize for HubEvent {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        KnownEventFrame {
            v: HUB_PROTOCOL,
            id: self.id.clone(),
            kind: self.kind.clone(),
        }
        .serialize(serializer)
    }
}
impl<'de> Deserialize<'de> for HubEvent {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Self::decode(Value::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn screenshot_filter_with_wrong_type_is_rejected_at_wire_boundary() {
        for parameters in [
            serde_json::json!({"application": 5}),
            serde_json::json!({"title": {}}),
            serde_json::json!({"application": null}),
        ] {
            let frame = serde_json::json!({"v":1,"id":null,"event":"device.request","data":{
                "request_id":"capture-1","capability":"screenshot","parameters":parameters
            }});
            assert!(HubEvent::decode(frame).is_err(), "accepted {parameters}");
        }
    }

    #[test]
    fn omitted_screenshot_filters_remain_explicitly_absent() -> Result<(), String> {
        let frame = HubEvent::decode(serde_json::json!({
            "v":1,"id":null,"event":"device.request","data":{
                "request_id":"capture-1","capability":"screenshot","parameters":{}
            }
        }))?;
        let HubEventKind::DeviceRequest(super::DeviceRequestData {
            request: crate::wire::bridge::BridgeRequest::Screenshot { parameters },
            ..
        }) = frame.kind
        else {
            return Err("wrong request shape".into());
        };
        assert_eq!(parameters.application, None);
        assert_eq!(parameters.title, None);
        Ok(())
    }

    #[test]
    fn reply_result_must_have_a_capability_result_shape() {
        let frame = serde_json::json!({
            "v":1,"id":"reply-1","method":"device.reply",
            "params":{"request_id":"capture-1","result":{"windows":3}}
        });
        assert!(HubRequest::decode(frame).is_err());
    }

    #[test]
    fn reply_error_is_an_additive_wire_field() -> Result<(), String> {
        let error: ErrorData = serde_json::from_value(serde_json::json!({
            "code":"device_reply_invalid", "message":"respuesta fuera de contrato",
            "numbers":{"field":"result.windows", "expected":"lista", "device_version":"0.3.13"},
            "refusal":{"code":"device_reply_invalid","field":"result.windows",
                "expected":"lista","device_version":"0.3.13","retry":{"kind":"check_outcome"}}
        }))
        .map_err(|error| error.to_string())?;
        assert_eq!(error.code.as_deref(), Some("device_reply_invalid"));
        assert!(
            error
                .numbers
                .as_ref()
                .is_some_and(|details| details.contains_key("field"))
        );
        Ok(())
    }

    #[test]
    fn invalid_delta_identifies_event_field_and_type() -> Result<(), String> {
        let Err(error) = HubEvent::decode(
            serde_json::json!({"v":1,"id":"turn","event":"chat.delta","data":{"session_id":"s","text":42}}),
        ) else {
            return Err("numeric delta was accepted".into());
        };
        assert!(error.contains("chat.delta"), "{error}");
        assert!(error.contains("text"), "{error}");
        assert!(error.contains("string"), "{error}");
        assert_eq!(error.matches("text:").count(), 1, "{error}");
        Ok(())
    }

    #[test]
    fn clarification_question_from_old_hub_is_opaque() -> Result<(), String> {
        let frame = HubEvent::decode(serde_json::json!({
            "v": 1, "event": "clarify.required", "data": {
                "session_id": "s", "request_id": "r",
                "questions": [{"qid": 42, "question": "primera"}]
            }
        }))?;
        assert!(
            matches!(frame.kind, HubEventKind::ClarifyRequired(RequiredData { questions: Some(questions), .. }) if questions[0]["qid"] == 42)
        );
        Ok(())
    }

    #[test]
    fn future_event_preserves_name() {
        let frame =
            HubEvent::decode(serde_json::json!({"v":1,"id":null,"event":"future.event","data":{}}));
        assert!(
            matches!(frame, Ok(HubEvent { kind: HubEventKind::Unrecognized { name }, .. }) if name == "future.event")
        );
    }

    #[test]
    fn future_method_preserves_name() {
        let frame = HubRequest::decode(
            serde_json::json!({"v":1,"id":"x","method":"future.method","params":{}}),
        );
        assert!(matches!(frame, Ok(HubRequest::Unrecognized { name }) if name == "future.method"));
    }

    #[test]
    fn known_event_round_trips_with_hub_envelope() -> Result<(), String> {
        let raw = serde_json::json!({"v":1,"id":null,"event":"chat.delta","data":{"session_id":"s","text":"hola"}});
        let frame = HubEvent::decode(raw.clone())?;
        assert_eq!(
            serde_json::to_value(frame).map_err(|error| error.to_string())?,
            raw
        );
        Ok(())
    }
}
