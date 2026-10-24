//! The files that go to the hub and come back: a capture's picture, an
//! attachment for the composer, a capture kept as one, and its bytes read
//! again.
use super::*;

/// The one way a capture's bytes reach the hub.
///
/// Both captures use this. The native helper's picture and the engine's are
/// the same kind of thing — a PNG far too big for a control frame — and the
/// split that makes them work is the same split: the bytes go over HTTP, bound
/// to the bridge request that asked for them, and the frame that answers
/// carries only the identifier. Two uploads that merely resembled each other
/// would drift, and only one of them would be the one anybody remembered.
///
/// Returns the digest it sent, so the answer can name the exact bytes the hub
/// now holds rather than recomputing them somewhere else.
pub async fn upload_capture(
    hub_url: &str,
    token: &str,
    development: bool,
    capture_id: &str,
    request_id: &str,
    bytes: Vec<u8>,
    op: &Op,
) -> Result<String> {
    let sha256 = format!("{:x}", <sha2::Sha256 as sha2::Digest>::digest(&bytes));
    let base = validate_hub_url(hub_url, development)?;
    let response = request(
        reqwest::Method::PUT,
        base.join(&format!("v1/captures/{capture_id}"))?,
        Some(token),
        op,
    )?
    .header("Content-Type", "image/png")
    .header("X-Orbit-Request", request_id)
    .header("X-Orbit-Sha256", &sha256)
    .body(bytes)
    .send()
    .await?;
    if !response.status().is_success() {
        let status = response.status();
        let body = read_refusal(response, MAX_HUB_HTTP_BODY_BYTES).await?;
        if let Ok(refusal) = serde_json::from_value::<orbit_core::refusal::Refusal>(body.details) {
            if body.code.as_deref() == Some("capture_capacity")
                && matches!(
                    &refusal,
                    orbit_core::refusal::Refusal::Busy {
                        operation: orbit_core::refusal::Operation::Capture,
                        ..
                    }
                )
            {
                return Err(orbit_core::refusal::Refusal::UpstreamFailed {
                    service: orbit_core::refusal::Service::Hub,
                    status: Some(status.as_u16()),
                }
                .into());
            }
            return Err(refusal.into());
        }
        bail!("El hub rechazó la captura (HTTP {status})")
    }
    Ok(sha256)
}

/// The header that names an attachment, percent-encoded UTF-8 (hub-v1).
const ATTACHMENT_NAME_HEADER: &str = "X-Orbit-Attachment-Name";

/// `name` as UTF-8 bytes, each one outside `A-Za-z0-9-._~` as `%XX`: a
/// header carries Latin-1, a file name anything.
fn percent_encoded(name: &str) -> String {
    let mut encoded = String::with_capacity(name.len());
    for byte in name.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            encoded.push(char::from(byte));
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

/// The inverse of `percent_encoded`; a `%` without two hex digits stays as it is.
fn percent_decoded(encoded: &str) -> String {
    let bytes = encoded.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        let pair = bytes
            .get(index + 1..index + 3)
            .and_then(|pair| std::str::from_utf8(pair).ok())
            .and_then(|pair| u8::from_str_radix(pair, 16).ok());
        match (bytes[index], pair) {
            (b'%', Some(byte)) => {
                decoded.push(byte);
                index += 3;
            }
            (byte, _) => {
                decoded.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8_lossy(&decoded).into_owned()
}

/// What a refused attachment request says: the hub's typed attachment
/// refusal when it is one, its status otherwise.
async fn attachment_refusal(response: reqwest::Response) -> anyhow::Error {
    let status = response.status().as_u16();
    match read_refusal(response, MAX_HUB_HTTP_BODY_BYTES).await {
        Ok(body) => {
            if let Some(refusal) = orbit_core::attachments::hub_refusal(&body.details) {
                return refusal.into();
            }
            if let Ok(refusal) =
                serde_json::from_value::<orbit_core::refusal::Refusal>(body.details)
            {
                return refusal.into();
            }
        }
        Err(error) => return error,
    }
    orbit_core::refusal::Refusal::UpstreamFailed {
        service: orbit_core::refusal::Service::Hub,
        status: Some(status),
    }
    .into()
}

/// An attachment request that got no answer from the hub. A hub stopped,
/// out of reach or gone mid-answer is said as such; one that kept quiet
/// past `HUB_HTTP_TIMEOUT` names that deadline. Never the request library's
/// text, which carries the hub's address.
fn attachment_unanswered(error: &reqwest::Error) -> anyhow::Error {
    use orbit_core::refusal::{Operation, Refusal};
    match TransportCause::of(error) {
        TransportCause::Timeout => {
            let deadline_ms = u64::try_from(HUB_HTTP_TIMEOUT.as_millis()).unwrap_or(u64::MAX);
            Refusal::Timeout {
                operation: Operation::Files,
                waited_ms: deadline_ms,
                deadline_ms,
            }
        }
        TransportCause::ConnectionTimeout | TransportCause::Connection | TransportCause::Cut => {
            Refusal::HubUnreachable { since_ms: None }
        }
    }
    .into()
}

/// One file for the composer, uploaded whole (`POST /v1/attachments`); the
/// hub classifies it and answers with what it keeps.
pub async fn upload_attachment(
    settings: &Settings,
    token: &str,
    development: bool,
    name: &str,
    bytes: Vec<u8>,
    op: &Op,
) -> Result<orbit_core::attachments::Attachment> {
    #[derive(Deserialize)]
    struct Uploaded {
        attachment: orbit_core::attachments::Attachment,
    }
    let base = validate_hub_url(
        settings.hub_url.as_deref().context("Falta el hub")?,
        development,
    )?;
    let sha256 = format!("{:x}", <sha2::Sha256 as sha2::Digest>::digest(&bytes));
    let encoded_name = percent_encoded(name);
    let response = request(
        reqwest::Method::POST,
        base.join("v1/attachments")?,
        Some(token),
        op,
    )?
    .header(ATTACHMENT_NAME_HEADER, encoded_name)
    .header("X-Orbit-Sha256", &sha256)
    .body(bytes)
    .send()
    .await
    .map_err(|error| attachment_unanswered(&error))?;
    if !response.status().is_success() {
        return Err(attachment_refusal(response).await);
    }
    let uploaded: Uploaded = serde_json::from_value(bounded_json(response).await?)
        .context("Respuesta del adjunto fuera de contrato")?;
    Ok(uploaded.attachment)
}

/// «Adjuntar ventana»: the capture uploaded for `request_id`
/// (`PUT /v1/captures`) kept as an attachment named `name`.
pub async fn attachment_from_capture(
    settings: &Settings,
    token: &str,
    development: bool,
    capture_id: &str,
    request_id: &str,
    name: &str,
    op: &Op,
) -> Result<orbit_core::attachments::Attachment> {
    #[derive(Deserialize)]
    struct Kept {
        attachment: orbit_core::attachments::Attachment,
    }
    let base = validate_hub_url(
        settings.hub_url.as_deref().context("Falta el hub")?,
        development,
    )?;
    let response = request(
        reqwest::Method::POST,
        base.join("v1/attachments/capture")?,
        Some(token),
        op,
    )?
    .json(&json!({"capture_id": capture_id, "request_id": request_id, "name": name}))
    .send()
    .await
    .map_err(|error| attachment_unanswered(&error))?;
    if !response.status().is_success() {
        return Err(attachment_refusal(response).await);
    }
    let kept: Kept = serde_json::from_value(bounded_json(response).await?)
        .context("Respuesta del adjunto fuera de contrato")?;
    Ok(kept.attachment)
}

/// One attachment's bytes as the hub keeps them, at most
/// `MAX_ATTACHMENT_BYTES` (`GET /v1/attachments/{id}`).
pub async fn fetch_attachment(
    settings: &Settings,
    token: &str,
    development: bool,
    attachment_id: &str,
    op: &Op,
) -> Result<orbit_core::protocol::AttachmentContent> {
    let base = validate_hub_url(
        settings.hub_url.as_deref().context("Falta el hub")?,
        development,
    )?;
    let mut url = base.join("v1/attachments/")?;
    url.path_segments_mut()
        .map_err(|()| anyhow::anyhow!("URL del hub sin ruta"))?
        .pop_if_empty()
        .push(attachment_id);
    let response = request(reqwest::Method::GET, url, Some(token), op)?
        .send()
        .await
        .map_err(|error| attachment_unanswered(&error))?;
    if !response.status().is_success() {
        return Err(attachment_refusal(response).await);
    }
    let mime = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("application/octet-stream")
        .to_owned();
    let name = response
        .headers()
        .get(ATTACHMENT_NAME_HEADER)
        .and_then(|value| value.to_str().ok())
        .map(percent_decoded)
        .unwrap_or_else(|| "adjunto".to_owned());
    let bytes = bounded_body(response, orbit_core::limits::MAX_ATTACHMENT_BYTES)
        .await
        .map_err(|error| match error {
            BodyError::TooLarge(size) => anyhow::Error::from(size),
            BodyError::Cut { source, .. } => attachment_unanswered(&source),
        })?;
    Ok(orbit_core::protocol::AttachmentContent { name, mime, bytes })
}
