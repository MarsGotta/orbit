//! An event the hub sends, by its name, and the decoding of its frame: the
//! protocol version first, then the payload its name promises.
use super::HUB_PROTOCOL;
use super::event_data::*;
use crate::wire::bridge::DeviceRequestData;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;
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
    #[serde(rename = "chat.item")]
    ChatItem(ItemData),
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
    /// The hub became nearly full, or stopped being (absent from 0.10.1).
    #[serde(rename = "storage.status")]
    StorageStatus(HubStorage),
    #[serde(rename = "agents.changed")]
    AgentsChanged(AgentsChangedData),
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
            "chat.item" => decode!(ItemData, ChatItem),
            "approval.required" => decode!(RequiredData, ApprovalRequired),
            "clarify.required" => decode!(RequiredData, ClarifyRequired),
            "interaction.resolved" => decode!(InteractionData, InteractionResolved),
            "interaction.expired" => decode!(InteractionData, InteractionExpired),
            "device.request" => decode!(DeviceRequestData, DeviceRequest),
            "state.snapshot" => decode!(SnapshotData, StateSnapshot),
            "error" => decode!(ErrorData, Error),
            "revoked" => decode!(EmptyData, Revoked),
            "recording.changed" => decode!(RecordingChangedData, RecordingChanged),
            "storage.status" => decode!(HubStorage, StorageStatus),
            "agents.changed" => decode!(AgentsChangedData, AgentsChanged),
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
