//! Play the public assistant reply through the server's voice relay.
use crate::{
    config::{Settings, validate_hub_url},
    hub, native,
};
use anyhow::{Context, Result, anyhow, bail};
use base64::{Engine, engine::general_purpose::STANDARD};
use futures_util::StreamExt;
use serde_json::{Value, json};
use std::time::Duration;
use tokio::sync::mpsc;

const MAX_SPEECH_CLIP_BYTES: usize = orbit_core::limits::MAX_SPEECH_CLIP_BYTES;

/// The hub could not give the voice of the agent asked for. True of any
/// kind of agent: the hub names the cause in its own refusal.
#[derive(Debug, thiserror::Error)]
#[error("Voice for {agent_name} is unavailable; reply in text")]
pub struct SpeechUnavailable {
    pub agent_name: String,
}

/// The name the user knows `agent_id` by; no id is the default agent.
pub fn agent_name(settings: &Settings, agent_id: Option<&str>) -> String {
    let roster = settings.roster();
    agent_id
        .and_then(|id| roster.agent(id))
        .map_or(roster.default_name(), |agent| agent.name.as_str())
        .to_owned()
}

/// Reads `text` in the voice of `agent_id` (hub-v1 `POST /v1/audio/speech`);
/// `None`, the hub's default agent, for a hub that lists no agents.
#[allow(clippy::too_many_arguments)]
pub async fn play(
    settings: Settings,
    token: String,
    development: bool,
    text: String,
    agent_id: Option<String>,
    playback_id: String,
    native_tx: mpsc::Sender<Value>,
    op: orbit_core::notice::Op,
) -> Result<()> {
    if text.is_empty() || text.len() > 32 * 1024 {
        bail!("The response exceeds the speech limit; the text remains available");
    }
    let base = validate_hub_url(
        settings.hub_url.as_deref().context("Hub not configured")?,
        development,
    )?;
    let response = hub::request(
        reqwest::Method::POST,
        base.join("v1/audio/speech")?,
        Some(&token),
        &op,
    )?
    .json(&match &agent_id {
        Some(agent_id) => json!({"text": text, "agent_id": agent_id}),
        None => json!({"text": text}),
    })
    .send()
    .await?;
    if !response.status().is_success() {
        return Err(speech_refusal(response, &settings, agent_id.as_deref()).await);
    }
    let (content_type, provider) = clip_kind(&response)?;
    let audio = read_clip(response).await?;
    let command = native::command(
        "playback_clip",
        json!({
            "id": playback_id,
            "playback_id": playback_id,
            "mime_type": content_type,
            "provider": provider,
            "audio_base64": STANDARD.encode(audio),
        }),
    )?;
    tokio::time::timeout(Duration::from_secs(2), native_tx.send(command)).await??;
    Ok(())
}

/// Why the hub gave no voice: the agent's voice is unavailable, or its
/// status with the code it named, if it named one.
async fn speech_refusal(
    response: reqwest::Response,
    settings: &Settings,
    agent_id: Option<&str>,
) -> anyhow::Error {
    let status = response.status();
    let code = hub::read_refusal(response, orbit_core::limits::MAX_HUB_HTTP_BODY_BYTES)
        .await
        .ok()
        .and_then(|refusal| refusal.code);
    match code.as_deref() {
        Some("speech_unavailable") => SpeechUnavailable {
            agent_name: agent_name(settings, agent_id),
        }
        .into(),
        Some(code) => anyhow!("Speech unavailable ({status}, {code}); the text is retained"),
        None => anyhow!("Speech unavailable ({status}); the text is retained"),
    }
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
