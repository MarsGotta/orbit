//! What a tool answered: its structured evidence, its text, and at most one
//! PNG, bounded by what a capture can deliver.
use super::transport::{MAX_FRAME_BYTES, ResponseTooLarge, TooLargePart};
use anyhow::{Context, Result, bail};
use serde::Serialize;
use serde_json::Value;

/// The base64 of the largest capture Orbit can deliver: the sources
/// (`orbit_core::sources::MAX_SOURCE_BYTES`) and the hub's capture upload
/// admit 700 000 bytes of PNG. A larger image could not reach anyone, so it is
/// refused where it arrives, with its size. Not a message bound.
const MAX_IMAGE_BASE64_BYTES: usize = orbit_core::sources::MAX_SOURCE_BYTES.div_ceil(3) * 4;
// A capture at its own ceiling has to fit in one message: what is left of
// the message (115 240 bytes) is its envelope.
const _: () = assert!(MAX_IMAGE_BASE64_BYTES < MAX_FRAME_BYTES);

/// Platform-native app/window schemas vary; preserve their structured evidence.
#[derive(Debug, Clone, Serialize)]
pub struct NativeObservation {
    pub structured: Option<Value>,
    pub text: Vec<String>,
    pub images: Vec<NativeImage>,
}

#[derive(Debug, Clone, Serialize)]
pub struct NativeImage {
    pub data: String,
    pub mime_type: String,
}

pub(super) fn parse_observation(result: &Value) -> Result<NativeObservation> {
    if result["isError"] == true {
        // The engine's own words are the only cause there is; without them a
        // refused read is indistinguishable from any other.
        // Bounded: the line goes to daemon.log, never to the report log.
        let said: String = result["content"]
            .as_array()
            .and_then(|items| items.iter().find_map(|item| item["text"].as_str()))
            .unwrap_or("sin texto")
            .chars()
            .take(400)
            .collect();
        bail!("CUA refused the read tool: {said}");
    }
    let structured = result
        .get("structuredContent")
        .filter(|value| !value.is_null())
        .cloned();
    let mut text = Vec::new();
    let mut images = Vec::new();
    for content in result["content"]
        .as_array()
        .context("Contenido MCP ausente")?
    {
        if content["type"] == "image" {
            if !images.is_empty() {
                bail!("CUA returned more than one image for a single observation");
            }
            let data = content["data"].as_str().context("Imagen MCP sin datos")?;
            let mime_type = content["mimeType"]
                .as_str()
                .context("Imagen MCP sin tipo")?;
            if mime_type != "image/png" {
                bail!("MCP image type {mime_type}; Orbit supports only image/png");
            }
            if data.len() > MAX_IMAGE_BASE64_BYTES {
                return Err(ResponseTooLarge {
                    part: TooLargePart::Image,
                    bytes: data.len(),
                    max_bytes: MAX_IMAGE_BASE64_BYTES,
                }
                .into());
            }
            images.push(NativeImage {
                data: data.to_owned(),
                mime_type: mime_type.to_owned(),
            });
            continue;
        }
        if content["type"] != "text" {
            bail!("Unrequested CUA content");
        }
        text.push(
            content["text"]
                .as_str()
                .context("Invalid MCP text")?
                .to_owned(),
        );
    }
    Ok(NativeObservation {
        structured,
        text,
        images,
    })
}
