//! Vault HTTP adapter: device credential only, no persistence, retries or content logs.
use orbit_core::{
    refusal::{Refusal, Resource, Service},
    vault::{VaultCall, VaultOperation, VaultOutcome},
};

mod routes;

pub async fn call(hub: &crate::hub_notes::NoteHub, call: &VaultCall) -> VaultOutcome {
    match exchange(hub, call).await {
        Ok(answer) => answer,
        Err(refusal) => VaultOutcome::Failed { refusal: *refusal },
    }
}

async fn exchange(
    hub: &crate::hub_notes::NoteHub,
    call: &VaultCall,
) -> Result<VaultOutcome, Box<Refusal>> {
    let routes::Request { method, url, body } = routes::request(&hub.base, call)?;
    let mut request =
        crate::hub::request(method, url, Some(&hub.token), &hub.op).map_err(|_| invalid())?;
    // Shorter than LocalClient's confirmation budget, including response decoding.
    request = request.timeout(std::time::Duration::from_secs(8));
    if let Some(body) = body {
        request = request.json(&body);
    }
    let response = request.send().await.map_err(|error| {
        Box::new(match crate::hub::TransportCause::of(&error) {
            crate::hub::TransportCause::Connection
            | crate::hub::TransportCause::ConnectionTimeout => {
                Refusal::HubUnreachable { since_ms: None }
            }
            crate::hub::TransportCause::Timeout | crate::hub::TransportCause::Cut => {
                Refusal::UpstreamFailed {
                    service: Service::Hub,
                    status: None,
                }
            }
        })
    })?;
    let status = response.status().as_u16();
    let bytes = crate::hub::bounded_body(response, orbit_core::vault::MAX_VAULT_RESPONSE_BYTES)
        .await
        .map_err(|error| {
            Box::new(match error {
                crate::hub::BodyError::TooLarge(size) => Refusal::TooLarge {
                    resource: Resource::HubResponse,
                    bytes: size.bytes,
                    max_bytes: size.max_bytes,
                },
                crate::hub::BodyError::Cut { .. } => *invalid(),
            })
        })?;
    decode(call, status, &bytes)
}

fn decode(call: &VaultCall, status: u16, bytes: &[u8]) -> Result<VaultOutcome, Box<Refusal>> {
    if status >= 400 {
        let value: serde_json::Value = serde_json::from_slice(bytes).map_err(|_| invalid())?;
        let code = value["code"]
            .as_str()
            .filter(|code| {
                code.len() <= 64 && code.bytes().all(|b| b.is_ascii_lowercase() || b == b'_')
            })
            .ok_or_else(invalid)?;
        // Compatibility text remains readable for older apps; current apps render the code.
        let message = value["message"]
            .as_str()
            .filter(|m| m.len() <= 1024)
            .ok_or_else(invalid)?;
        let numbers = value["refusal"]
            .as_object()
            .into_iter()
            .flatten()
            .filter_map(|(key, value)| value.as_i64().map(|n| (key.clone(), n)))
            .collect();
        return Ok(VaultOutcome::Refused {
            code: code.into(),
            message: message.into(),
            message_code: value["message_code"]
                .as_str()
                .filter(|code| {
                    code.len() <= 64 && code.bytes().all(|b| b.is_ascii_lowercase() || b == b'_')
                })
                .map(str::to_owned),
            message_fields: serde_json::from_value(value["message_fields"].clone()).ok(),
            numbers,
        });
    }
    match call {
        VaultCall::ListDistillations { .. } => {
            serde_json::from_slice(bytes).map(|page| VaultOutcome::Distillations { page })
        }
        VaultCall::Distillation { .. }
        | VaultCall::PrepareDistillation { .. }
        | VaultCall::ReviewDistillation { .. }
        | VaultCall::DistillationAction { .. } => {
            return decode_distillation(bytes);
        }
        VaultCall::Config | VaultCall::Configure { .. } | VaultCall::Disconnect => {
            serde_json::from_slice(bytes)
                .map(|configuration| VaultOutcome::Config { configuration })
        }
        VaultCall::Read { request } if matches!(request.operation, VaultOperation::Catalog) => {
            serde_json::from_slice(bytes).map(|catalog| VaultOutcome::Catalog { catalog })
        }
        VaultCall::Read { request } if matches!(request.operation, VaultOperation::Tasks) => {
            serde_json::from_slice(bytes).map(|tasks| VaultOutcome::Tasks { tasks })
        }
        VaultCall::Read { request }
            if matches!(
                request.operation,
                VaultOperation::Read | VaultOperation::Index
            ) =>
        {
            serde_json::from_slice(bytes).map(|note| VaultOutcome::Note { note })
        }
        VaultCall::Read { .. } => {
            serde_json::from_slice(bytes).map(|page| VaultOutcome::Page { page })
        }
    }
    .map_err(|_| invalid())
}

fn decode_distillation(bytes: &[u8]) -> Result<VaultOutcome, Box<Refusal>> {
    let value: serde_json::Value = serde_json::from_slice(bytes).map_err(|_| invalid())?;
    if value["prepared"] == false && value["reason"] == "vault_or_model_not_configured" {
        return Ok(VaultOutcome::DistillationDisabled {
            reason: "vault_or_model_not_configured".into(),
        });
    }
    serde_json::from_value(value)
        .map(|proposal| VaultOutcome::Distillation { proposal })
        .map_err(|_| invalid())
}

fn invalid() -> Box<Refusal> {
    Box::new(Refusal::HubReplyInvalid {
        field: "vault".into(),
        received_bytes: None,
    })
}
