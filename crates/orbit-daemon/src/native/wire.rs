//! The frames the daemon writes to the helper, and the WAV of a sealed chunk.
use super::*;

/// Encodes the exact line sent to the helper, including its newline. The
/// helper's 6 MiB stdin contract bounds this allocation before any write.
pub fn encode_command_line(value: &Value) -> Result<Vec<u8>> {
    let mut bytes = serde_json::to_vec(value)?;
    bytes.push(b'\n');
    if bytes.len() > MAX_HELPER_COMMAND_LINE_BYTES {
        anyhow::bail!(
            "Helper command is {} bytes; maximum {MAX_HELPER_COMMAND_LINE_BYTES}",
            bytes.len()
        );
    }
    Ok(bytes)
}

pub fn command(name: &str, fields: Value) -> Result<Value> {
    let mut fields = match fields {
        Value::Object(fields) => fields,
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) | Value::Array(_) => {
            anyhow::bail!("Native command fields must be a JSON object")
        }
    };
    let id = match fields.remove("id") {
        Some(Value::String(id)) => id,
        Some(_) => anyhow::bail!("The native command ID must be text"),
        None => uuid::Uuid::new_v4().to_string(),
    };
    let frame =
        orbit_core::wire::native::NativeCommand::from_parts(id, name, Value::Object(fields))
            .map_err(anyhow::Error::msg)?;
    serde_json::to_value(frame).context("Could not serialize the native command")
}

/// Encode an observed PCM segment with its actual rate. No invented timestamps.
pub fn wav(pcm: &[u8], sample_rate: u32, channels: u16) -> Result<Vec<u8>> {
    if channels == 0
        || channels > 2
        || !(8_000..=192_000).contains(&sample_rate)
        || pcm.len() > 32 * 1024 * 1024
        || !pcm.len().is_multiple_of(usize::from(channels) * 2)
    {
        anyhow::bail!("Invalid PCM format");
    }
    let len = u32::try_from(pcm.len())?;
    let mut out = Vec::with_capacity(pcm.len() + 44);
    out.extend(b"RIFF");
    out.extend((len + 36).to_le_bytes());
    out.extend(b"WAVEfmt ");
    out.extend(16u32.to_le_bytes());
    out.extend(1u16.to_le_bytes());
    out.extend(channels.to_le_bytes());
    out.extend(sample_rate.to_le_bytes());
    out.extend((sample_rate * u32::from(channels) * 2).to_le_bytes());
    out.extend((channels * 2).to_le_bytes());
    out.extend(16u16.to_le_bytes());
    out.extend(b"data");
    out.extend(len.to_le_bytes());
    out.extend(pcm);
    Ok(out)
}
