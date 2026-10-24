//! The engine's transport: one JSON message per line each way, each bounded,
//! with end of file and a refused size carried as types.
use anyhow::Result;
use anyhow::bail;
use serde_json::Value;
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWrite, AsyncWriteExt};

/// The largest message Orbit holds whole from the engine, or sends to it: a
/// frame is kept entire until it is parsed, so this protects the daemon's
/// memory from a single message. 1 MiB because the largest message Orbit can
/// use is a capture at the capture ceiling (`MAX_IMAGE_BASE64_BYTES`, the
/// 700 000 bytes of PNG the sources and the hub admit, 933 336 in base64) with
/// its envelope; the engine's whole catalogue measured 147 909 bytes on a
/// MacBook. It is not an image bound, and a longer answer costs its request, not
/// the connection: it is read to its end, dropped, and refused with its size.
/// A Retina full screen is several MiB of base64, which is why it never comes
/// inline: the engine writes it to a file of Orbit's.
pub(super) const MAX_FRAME_BYTES: usize = 1_048_576;

pub(super) async fn write_json<W: AsyncWrite + Unpin>(writer: &mut W, value: &Value) -> Result<()> {
    let mut bytes = serde_json::to_vec(value)?;
    if bytes.len() + 1 > MAX_FRAME_BYTES {
        bail!(
            "El mensaje para el motor ocupa {} bytes y un mensaje del motor admite {MAX_FRAME_BYTES} bytes; no se envió",
            bytes.len() + 1
        );
    }
    bytes.push(b'\n');
    writer.write_all(&bytes).await?;
    writer.flush().await?;
    Ok(())
}

/// End of file on the MCP transport, carried as a type rather than a string so
/// the caller that can tell *why* the engine stopped talking recognizes this
/// one failure and leaves every other one exactly as it was.
#[derive(Debug)]
struct ConnectionClosed;

impl std::fmt::Display for ConnectionClosed {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("Conexión MCP cerrada antes de responder")
    }
}

impl std::error::Error for ConnectionClosed {}

/// Whether a transport failure says the engine's end of the pipes is gone: end
/// of file on what it writes, or a broken pipe on what Orbit writes to it. The
/// exit of the process closes both, so either can be how Orbit first learns of
/// a death; which one depends on whether Orbit's write lands before the exit
/// or after it (both were measured on the same fixture). Every other failure
/// is left exactly as it was.
pub(super) fn the_engine_end_is_gone(error: &anyhow::Error) -> bool {
    error.downcast_ref::<ConnectionClosed>().is_some()
        || error
            .downcast_ref::<std::io::Error>()
            .is_some_and(|error| error.kind() == std::io::ErrorKind::BrokenPipe)
}

/// An answer of the engine refused for its size, with both numbers, so that
/// whoever asked is told them. It is not an engine failure: the
/// engine answered, and the connection stays in step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResponseTooLarge {
    pub part: TooLargePart,
    pub bytes: usize,
    pub max_bytes: usize,
}

/// Which bound the answer met.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TooLargePart {
    /// The whole message, against `MAX_FRAME_BYTES`.
    Message,
    /// The image inside it, in base64, against `MAX_IMAGE_BASE64_BYTES`.
    Image,
}

impl std::fmt::Display for ResponseTooLarge {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.part {
            TooLargePart::Message => write!(
                formatter,
                "La respuesta del motor ocupa {} bytes y un mensaje del motor admite {} bytes; se leyó entera y se descartó, y la conexión sigue",
                self.bytes, self.max_bytes
            ),
            TooLargePart::Image => write!(
                formatter,
                "La imagen del motor ocupa {} bytes en base64 y una captura admite {} bytes en base64 ({} bytes de PNG)",
                self.bytes,
                self.max_bytes,
                orbit_core::sources::MAX_SOURCE_BYTES
            ),
        }
    }
}

impl std::error::Error for ResponseTooLarge {}

/// One line of the transport. A line longer than `MAX_FRAME_BYTES` is not
/// kept: it is read to its end and dropped, so the next line is the next
/// message, and it is refused with its length. A sender that never ends its
/// line is bounded by the request's own timeout.
pub(super) async fn read_json<R: AsyncBufRead + Unpin>(reader: &mut R) -> Result<Value> {
    let mut frame = Vec::new();
    let mut dropped = 0_usize;
    loop {
        let available = reader.fill_buf().await?;
        if available.is_empty() {
            return Err(ConnectionClosed.into());
        }
        let newline = available.iter().position(|byte| *byte == b'\n');
        let count = newline.map_or(available.len(), |index| index + 1);
        if dropped == 0 && frame.len() + count <= MAX_FRAME_BYTES {
            frame.extend_from_slice(&available[..count]);
        } else {
            dropped = dropped.saturating_add(frame.len()).saturating_add(count);
            frame = Vec::new();
        }
        reader.consume(count);
        if newline.is_some() {
            if dropped > 0 {
                return Err(ResponseTooLarge {
                    part: TooLargePart::Message,
                    bytes: dropped,
                    max_bytes: MAX_FRAME_BYTES,
                }
                .into());
            }
            return Ok(serde_json::from_slice(&frame)?);
        }
    }
}
