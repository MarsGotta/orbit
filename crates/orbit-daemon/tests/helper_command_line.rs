//! A bounded helper command, serialized without launching the helper.
#![allow(clippy::expect_used)]

use anyhow::Result;
use base64::{Engine, engine::general_purpose::STANDARD};
use orbit_core::limits::{
    MAX_HELPER_COMMAND_LINE_BYTES, MAX_HELPER_EVENT_LINE_BYTES, MAX_SPEECH_CLIP_BYTES,
};
use orbit_daemon::native::{command, encode_command_line};
use serde_json::json;

#[test]
fn playback_clip_fits_command_line_and_larger_frame_is_refused() -> Result<()> {
    let clip = vec![0; MAX_SPEECH_CLIP_BYTES];
    let frame = command(
        "playback_clip",
        json!({
            "playback_id": "x".repeat(128),
            "mime_type": "audio/ogg",
            "provider": "x".repeat(128),
            "audio_base64": STANDARD.encode(clip),
        }),
    )?;
    let line = encode_command_line(&frame)?;
    println!("helper playback line: {} bytes", line.len());
    assert!(line.len() <= MAX_HELPER_COMMAND_LINE_BYTES);
    assert!(line.len() > MAX_HELPER_EVENT_LINE_BYTES);
    let too_large = json!({"payload": "x".repeat(MAX_HELPER_COMMAND_LINE_BYTES)});
    let error = encode_command_line(&too_large).expect_err("over command line");
    assert!(
        error
            .to_string()
            .contains(&MAX_HELPER_COMMAND_LINE_BYTES.to_string())
    );
    Ok(())
}
