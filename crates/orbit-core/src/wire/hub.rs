//! Typed hub-v1 WebSocket frames. Unknown names survive mixed-version deployments.
//!
//! This file holds the requests the Mac sends; `event` is what the hub sends
//! and how a frame is decoded, and `event_data` the payload of each event.
use super::bridge::DeviceReplyParams;
use serde::{Deserialize, Serialize};
use serde_json::Value;

mod event;
mod event_data;

pub use event::{HubDecodeError, HubEnvelopeError, HubEvent, HubEventKind};
pub use event_data::*;

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
    #[serde(rename = "chat.observe")]
    ChatObserve {
        v: u8,
        id: String,
        params: SessionData,
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
    /// The ids of the files uploaded for this turn (hub-v1 «Attachments»).
    /// Sent only to a hub that announced attachments in `ready`: a 0.9.0
    /// hub ignores unknown fields, and the turn would go without its files.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub attachments: Vec<String>,
    /// The conversation this new one takes over (hub-v1 «Handing a
    /// conversation over»). Sent only to a hub that lists `addable_agents`.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub handover_from: Option<String>,
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
    /// One of the request's `choices`; a 0.9.0 hub ignores it and answers
    /// `once` or `deny` by `approved`.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub choice: Option<String>,
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
            "chat.observe" => Self::ChatObserve {
                v: HUB_PROTOCOL,
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
            _ => return Err(format!("Unsupported hub method: {method}")),
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
            "chat.observe",
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
        let HubEventKind::DeviceRequest(crate::wire::bridge::DeviceRequestData {
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
            "code":"device_reply_invalid", "message":"response violates the contract",
            "refusal":{"code":"device_reply_invalid","field":"result.windows",
                "expected":"lista","device_version":"0.3.13","retry":{"kind":"check_outcome"}}
        }))
        .map_err(|error| error.to_string())?;
        assert_eq!(error.code.as_deref(), Some("device_reply_invalid"));
        assert_eq!(
            error
                .refusal
                .as_ref()
                .map(|refusal| refusal.code.as_deref()),
            Some(Some("device_reply_invalid"))
        );
        assert_eq!(
            error.refusal.map(|refusal| refusal.retry),
            Some(RetryDisposition::CheckOutcome)
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
