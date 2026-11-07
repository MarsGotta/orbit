//! Play the public assistant reply through the server's voice relay.
use crate::{
    config::{Settings, hub_endpoint},
    native,
};
use anyhow::{Result, anyhow, bail};
use base64::{Engine, engine::general_purpose::STANDARD};
use orbit_hub_client::speech;
use serde_json::{Value, json};
use std::time::Duration;
use tokio::sync::mpsc;

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
    let clip = match speech::clip(
        &hub_endpoint(&settings),
        &token,
        development,
        &text,
        agent_id.as_deref(),
        &op,
    )
    .await
    {
        Ok(clip) => clip,
        Err(error) => {
            return Err(match error.downcast::<speech::SpeechRefused>() {
                Ok(refused) => speech_refusal(refused, &settings, agent_id.as_deref()),
                Err(error) => error,
            });
        }
    };
    let command = native::command(
        "playback_clip",
        json!({
            "id": playback_id,
            "playback_id": playback_id,
            "mime_type": clip.content_type,
            "provider": clip.provider,
            "audio_base64": STANDARD.encode(clip.audio),
        }),
    )?;
    tokio::time::timeout(Duration::from_secs(2), native_tx.send(command)).await??;
    Ok(())
}

/// Why the hub gave no voice: the agent's voice is unavailable, or its
/// status with the code it named, if it named one.
fn speech_refusal(
    speech::SpeechRefused { status, code }: speech::SpeechRefused,
    settings: &Settings,
    agent_id: Option<&str>,
) -> anyhow::Error {
    match code.as_deref() {
        Some("speech_unavailable") => SpeechUnavailable {
            agent_name: agent_name(settings, agent_id),
        }
        .into(),
        Some(code) => anyhow!("Speech unavailable ({status}, {code}); the text is retained"),
        None => anyhow!("Speech unavailable ({status}); the text is retained"),
    }
}
