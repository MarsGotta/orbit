//! The public assistant reply in the voice of an agent, as the hub's voice
//! relay gives it (hub-v1 `POST /v1/audio/speech`).
use crate::{HubEndpoint, hub, validate_hub_url};
use anyhow::{Context, Result, bail};
use futures_util::StreamExt;
use orbit_core::notice::Op;
use serde_json::json;

const MAX_SPEECH_CLIP_BYTES: usize = orbit_core::limits::MAX_SPEECH_CLIP_BYTES;

/// The hub refused the voice: its status, and the code it named, if it
/// named one.
#[derive(Debug, thiserror::Error)]
#[error("The hub refused the speech ({status})")]
pub struct SpeechRefused {
    pub status: reqwest::StatusCode,
    pub code: Option<String>,
}

/// One clip to play, with the content type and provider the hub declared.
pub struct Clip {
    pub content_type: String,
    pub provider: String,
    pub audio: Vec<u8>,
}

/// Asks the hub to read `text` in the voice of `agent_id`; `None`, the hub's
/// default agent, for a hub that lists no agents. A refusal is
/// [`SpeechRefused`].
pub async fn clip(
    endpoint: &HubEndpoint,
    token: &str,
    development: bool,
    text: &str,
    agent_id: Option<&str>,
    op: &Op,
) -> Result<Clip> {
    let base = validate_hub_url(
        endpoint.hub_url.as_deref().context("Hub not configured")?,
        development,
    )?;
    let response = hub::request(
        reqwest::Method::POST,
        base.join("v1/audio/speech")?,
        Some(token),
        op,
    )?
    .json(&match agent_id {
        Some(agent_id) => json!({"text": text, "agent_id": agent_id}),
        None => json!({"text": text}),
    })
    .send()
    .await?;
    if !response.status().is_success() {
        return Err(speech_refusal(response).await.into());
    }
    let (content_type, provider) = clip_kind(&response)?;
    let audio = read_clip(response).await?;
    Ok(Clip {
        content_type,
        provider,
        audio,
    })
}

/// Why the hub gave no voice: its status with the code it named, if it
/// named one.
async fn speech_refusal(response: reqwest::Response) -> SpeechRefused {
    let status = response.status();
    let code = hub::read_refusal(response, orbit_core::limits::MAX_HUB_HTTP_BODY_BYTES)
        .await
        .ok()
        .and_then(|refusal| refusal.code);
    SpeechRefused { status, code }
}

/// The clip's content type and provider, as the hub declared them; only a
/// clip is played.
fn clip_kind(response: &reqwest::Response) -> Result<(String, String)> {
    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("")
        .to_owned();
    let kind = response
        .headers()
        .get("x-orbit-speech-kind")
        .and_then(|header| header.to_str().ok())
        .unwrap_or("")
        .to_owned();
    let provider = response
        .headers()
        .get("x-orbit-speech-provider")
        .and_then(|header| header.to_str().ok())
        .map(str::to_owned);
    if kind != "clip" {
        bail!("Unsupported speech format; no audio was played");
    }
    if content_type.is_empty() || content_type.len() > 128 {
        bail!("The hub did not declare the speech clip type");
    }
    let provider = provider.context("The hub did not declare the speech clip origin")?;
    if provider.is_empty() || provider.len() > 128 {
        bail!("The hub declared an invalid speech origin");
    }
    Ok((content_type, provider))
}

/// The clip's audio, refused past [`MAX_SPEECH_CLIP_BYTES`] and when empty.
async fn read_clip(response: reqwest::Response) -> Result<Vec<u8>> {
    let mut stream = response.bytes_stream();
    let mut audio = Vec::new();
    while let Some(bytes) = stream.next().await {
        let bytes = bytes?;
        if bytes.len() > MAX_SPEECH_CLIP_BYTES.saturating_sub(audio.len()) {
            bail!("The speech clip exceeds the 4 MiB limit");
        }
        audio.extend_from_slice(&bytes);
    }
    if audio.is_empty() {
        bail!("The hub delivered an empty speech clip");
    }
    Ok(audio)
}
