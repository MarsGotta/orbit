//! A frame's envelope read in place: its name, its identity and its outcome,
//! borrowed from the frame so a large audio frame is never copied to route it.
use super::*;
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NativeError {
    pub code: String,
    pub message: String,
    /// The refusal's numbers and names, by code (the calendar write
    /// refusals). Absent from every other error.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<Value>,
}

/// The fields shared by every helper response. The bridge validates the
/// native-v1 event; the owner supplies the command before decoding its body.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NativeResponseFields {
    pub id: String,
    pub ok: bool,
    #[serde(default)]
    pub result: Option<Value>,
    #[serde(default)]
    pub error: Option<NativeError>,
}

impl NativeResponseFields {
    pub fn from_value(value: &Value) -> Result<Self, serde_json::Error> {
        Self::deserialize(value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct NativeResponseId {
    pub id: String,
}

#[derive(Debug, Clone, Copy, Deserialize)]
pub struct NativeEventHeader<'a> {
    pub event: &'a str,
}

impl<'a> NativeEventHeader<'a> {
    pub fn from_value(value: &'a Value) -> Result<Self, serde_json::Error> {
        Self::deserialize(value)
    }
}

#[derive(Debug, Clone, Copy, Deserialize)]
pub struct NativeVersionHeader {
    pub v: u8,
}

impl NativeVersionHeader {
    pub fn from_value(value: &Value) -> Result<Self, serde_json::Error> {
        Self::deserialize(value)
    }
}

#[derive(Debug, Clone, Copy, Deserialize)]
pub struct NativeResponseIdRef<'a> {
    pub id: &'a str,
}

#[derive(Debug, Clone, Copy)]
pub struct NativeResponseView<'a> {
    pub id: &'a str,
    pub ok: bool,
    pub result: Option<&'a Value>,
    pub error: Option<NativeErrorRef<'a>>,
}

impl<'a> NativeResponseView<'a> {
    pub fn from_value(value: &'a Value) -> Result<Self, NativeDecodeProblem> {
        fn problem(field: &str, value: Option<&Value>) -> NativeDecodeProblem {
            NativeDecodeProblem {
                event: "response",
                command: "unknown".into(),
                field: field.into(),
                observed_type: match value {
                    None => "absent",
                    Some(Value::Null) => "null",
                    Some(Value::Bool(_)) => "boolean",
                    Some(Value::Number(_)) => "number",
                    Some(Value::String(_)) => "string",
                    Some(Value::Array(_)) => "array",
                    Some(Value::Object(_)) => "object",
                },
            }
        }
        let map = value.as_object().ok_or_else(|| problem(".", Some(value)))?;
        let id_value = map.get("id");
        let id = id_value
            .and_then(Value::as_str)
            .ok_or_else(|| problem("id", id_value))?;
        let ok_value = map.get("ok");
        let ok = ok_value
            .and_then(Value::as_bool)
            .ok_or_else(|| problem("ok", ok_value))?;
        let error = match map.get("error") {
            None | Some(Value::Null) => None,
            Some(value) => {
                let fields = value
                    .as_object()
                    .ok_or_else(|| problem("error", Some(value)))?;
                let code = fields.get("code");
                let message = fields.get("message");
                Some(NativeErrorRef {
                    code: code
                        .and_then(Value::as_str)
                        .ok_or_else(|| problem("error.code", code))?,
                    message: message
                        .and_then(Value::as_str)
                        .ok_or_else(|| problem("error.message", message))?,
                })
            }
        };
        Ok(Self {
            id,
            ok,
            result: map.get("result").filter(|result| !result.is_null()),
            error,
        })
    }
}

#[derive(Debug, Clone, Copy)]
pub struct NativeErrorRef<'a> {
    pub code: &'a str,
    pub message: &'a str,
}

impl<'a> NativeResponseIdRef<'a> {
    pub fn from_value(value: &'a Value) -> Result<Self, serde_json::Error> {
        Self::deserialize(value)
    }
}

#[derive(Debug, Clone, Copy, Deserialize)]
pub struct NativeCommandHeader<'a> {
    pub id: &'a str,
    pub command: &'a str,
}

impl<'a> NativeCommandHeader<'a> {
    pub fn from_value(value: &'a Value) -> Result<Self, serde_json::Error> {
        Self::deserialize(value)
    }
}

/// Borrow the PCM text and frame identity while validating its numeric fields.
/// The daemon can inspect every audio frame without copying its base64 body.
#[derive(Debug, Clone, Copy, Deserialize)]
pub struct NativeAudioFrameRef<'a> {
    pub source: &'a str,
    pub stream_id: &'a str,
    pub sample_rate: u32,
    pub channels: u8,
    pub frames: u32,
    pub sequence: u64,
    pub monotonic_ns: u64,
    pub pcm_s16le_base64: &'a str,
    #[serde(default)]
    pub capture_epoch_id: Option<&'a str>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
pub struct NativeAudioIdentityRef<'a> {
    pub source: &'a str,
    pub stream_id: &'a str,
}

impl<'a> NativeAudioIdentityRef<'a> {
    pub fn from_value(value: &'a Value) -> Result<Self, serde_json::Error> {
        Self::deserialize(value)
    }
}

impl<'a> NativeAudioFrameRef<'a> {
    pub fn from_value(value: &'a Value) -> Result<Self, serde_json::Error> {
        Self::deserialize(value)
    }
}
