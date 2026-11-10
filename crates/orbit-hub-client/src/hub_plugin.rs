//! A plugin call to the hub (`contracts/plugin-v1.md`, «Call»). The device
//! checks the `id`'s shape and the two sizes, sends the call once, as
//! written, and hands back the hub's answer untouched. It keeps, logs,
//! interprets and repeats nothing.
use crate::hub::{BodyError, MAX_HUB_CODE_BYTES, TransportCause};
use crate::hub_notes::NoteHub;
use orbit_core::plugin::{
    PLUGIN_CALL_MAX_BYTES, PLUGIN_RESPONSE_MAX_BYTES, PluginCall, PluginOutcome, plugin_id_refusal,
};
use orbit_core::refusal::{InternalCause, Operation, Refusal, Service};
use serde_json::{Map, Value};
use std::time::{Duration, Instant};

/// The device's whole wait for one call, sending and reading included.
/// Longer than the hub's `PLUGIN_CALL_BUDGET_MS` (7 s), so a plugin that
/// answers late is the hub's `plugin_timeout`; shorter than the app's 10 s
/// (`CALL_TIMEOUT_SECONDS`), so this wait's own refusal reaches the caller.
/// The same 8 s the Notes calls wait. Not a retry interval.
pub const PLUGIN_CALL_WAIT: Duration = Duration::from_secs(8);

type Refused = Box<Refusal>;

pub async fn call(hub: &NoteHub, plugin: &str, call: &PluginCall) -> PluginOutcome {
    let started = Instant::now();
    let outcome = match admitted(plugin, call) {
        Ok(body) => tokio::time::timeout(PLUGIN_CALL_WAIT, exchange(hub, plugin, body, started))
            .await
            .unwrap_or_else(|_| Err(timeout(started))),
        Err(refusal) => Err(refusal),
    };
    outcome.unwrap_or_else(|refusal| PluginOutcome::Failed { refusal: *refusal })
}

/// The body to send, or why it never leaves the device.
fn admitted(plugin: &str, call: &PluginCall) -> Result<Vec<u8>, Refused> {
    if let Some(refusal) = plugin_id_refusal(plugin) {
        return Err(Box::new(refusal));
    }
    let body = serde_json::to_vec(call).map_err(|error| internal(error.to_string()))?;
    if body.len() > PLUGIN_CALL_MAX_BYTES {
        return Err(Box::new(Refusal::PluginCallTooLarge {
            bytes: body.len() as u64,
            max_bytes: PLUGIN_CALL_MAX_BYTES as u64,
        }));
    }
    Ok(body)
}

async fn exchange(
    hub: &NoteHub,
    plugin: &str,
    body: Vec<u8>,
    started: Instant,
) -> Result<PluginOutcome, Refused> {
    let mut url = hub.base.clone();
    url.path_segments_mut()
        .map_err(|()| internal("the hub address takes no path".into()))?
        .pop_if_empty()
        .extend(["v1", "plugins", plugin, "call"]);
    let response = crate::hub::request(reqwest::Method::POST, url, Some(&hub.token), &hub.op)
        .map_err(|error| internal(error.to_string()))?
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body(body)
        .send()
        .await
        .map_err(|error| match TransportCause::of(&error) {
            TransportCause::Connection => Box::new(Refusal::HubUnreachable { since_ms: None }),
            TransportCause::ConnectionTimeout | TransportCause::Timeout => timeout(started),
            TransportCause::Cut => Box::new(Refusal::UpstreamFailed {
                service: Service::Hub,
                status: None,
            }),
        })?;
    let status = response.status().as_u16();
    let bytes = crate::hub::bounded_body(response, PLUGIN_RESPONSE_MAX_BYTES)
        .await
        .map_err(|error| match error {
            BodyError::TooLarge(size) => Box::new(Refusal::PluginResponseTooLarge {
                bytes: size.bytes,
                max_bytes: size.max_bytes,
            }),
            BodyError::Cut { source, .. } if source.is_timeout() => timeout(started),
            BodyError::Cut { received_bytes, .. } => Box::new(Refusal::HubReplyInvalid {
                field: "body".into(),
                received_bytes: Some(received_bytes as u64),
            }),
        })?;
    decode(status, &bytes)
}

/// `200 {"value": …}` exactly, or a refusal of 400 or above in the
/// contract's shape; anything else names the part that was wrong.
fn decode(status: u16, bytes: &[u8]) -> Result<PluginOutcome, Refused> {
    let invalid = |field: &str| {
        Box::new(Refusal::HubReplyInvalid {
            field: field.into(),
            received_bytes: Some(bytes.len() as u64),
        })
    };
    let object = || match serde_json::from_slice(bytes) {
        Ok(Value::Object(body)) => Some(body),
        _ => None,
    };
    match status {
        200 => {
            let mut body = object().ok_or_else(|| invalid("value"))?;
            match body.remove("value") {
                Some(value) if body.is_empty() => Ok(PluginOutcome::Answered { value }),
                _ => Err(invalid("value")),
            }
        }
        400.. => match object() {
            Some(body) if is_refusal(&body) => Ok(PluginOutcome::Refused { status, body }),
            _ => Err(invalid("refusal")),
        },
        _ => Err(invalid("status")),
    }
}

/// `code` and `refusal.code` the same `[a-z0-9_]{1,64}`, and a `message`.
fn is_refusal(body: &Map<String, Value>) -> bool {
    let code = body.get("code").and_then(Value::as_str).filter(|code| {
        (1..=MAX_HUB_CODE_BYTES).contains(&code.len())
            && code
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
    });
    let inner = body
        .get("refusal")
        .and_then(|refusal| refusal.get("code"))
        .and_then(Value::as_str);
    code.is_some() && inner == code && body.get("message").is_some_and(Value::is_string)
}

fn timeout(started: Instant) -> Refused {
    Box::new(Refusal::Timeout {
        operation: Operation::PluginCall,
        waited_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
        deadline_ms: u64::try_from(PLUGIN_CALL_WAIT.as_millis()).unwrap_or(u64::MAX),
    })
}

/// The device could not even write the request: no call left it.
fn internal(error: String) -> Refused {
    Box::new(Refusal::Internal {
        cause: InternalCause::DaemonOperation { error },
    })
}
