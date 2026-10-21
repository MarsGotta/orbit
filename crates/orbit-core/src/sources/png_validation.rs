//! Decode one bounded PNG without interpreting ancillary text or ICC profiles.
use super::{MAX_SOURCE_BYTES, SourceError, validate_dimensions};
use std::io::Cursor;

#[cfg(test)]
#[path = "png_validation_tests.rs"]
mod tests;

/// Validate all image data before a source can become durable evidence.
/// Decoder scratch and caller-owned output have independent allocation limits.
pub fn validate_png(bytes: &[u8], width: u32, height: u32) -> Result<(), SourceError> {
    validate_dimensions(width, height)?;
    if bytes.len() > MAX_SOURCE_BYTES {
        return Err(SourceError::Bounds);
    }
    validate_container(bytes)?;
    let mut decoder = png::Decoder::new_with_limits(
        Cursor::new(bytes),
        png::Limits {
            bytes: 16 * 1024 * 1024,
        },
    );
    decoder.set_ignore_text_chunk(true);
    decoder.set_ignore_iccp_chunk(true);
    decoder.ignore_checksums(false);
    let header = decoder.read_header_info().map_err(|_| SourceError::Image)?;
    if header.width != width || header.height != height {
        return Err(SourceError::Image);
    }
    let mut reader = decoder.read_info().map_err(|_| SourceError::Image)?;
    if reader.info().animation_control.is_some() || reader.info().frame_control.is_some() {
        return Err(SourceError::Image);
    }
    let size = reader.output_buffer_size().ok_or(SourceError::Bounds)?;
    if size > 10 * 1024 * 1024 {
        return Err(SourceError::Bounds);
    }
    let mut decoded = vec![0; size];
    let frame = reader
        .next_frame(&mut decoded)
        .map_err(|_| SourceError::Image)?;
    if frame.width != width || frame.height != height {
        return Err(SourceError::Image);
    }
    reader.finish().map_err(|_| SourceError::Image)
}

/// Verify every chunk CRC and reject animation, truncation and material after IEND.
/// Text/profile chunks remain uninterpreted: CRC covers their raw bytes, without
/// inflating them. Pixel decoding and image semantics remain the decoder's job.
fn validate_container(bytes: &[u8]) -> Result<(), SourceError> {
    if !bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Err(SourceError::Image);
    }
    let mut offset = 8usize;
    while offset < bytes.len() {
        let header = bytes.get(offset..offset + 8).ok_or(SourceError::Image)?;
        let length =
            u32::from_be_bytes(header[..4].try_into().map_err(|_| SourceError::Image)?) as usize;
        let kind = &header[4..8];
        let end = offset
            .checked_add(12)
            .and_then(|start| start.checked_add(length))
            .ok_or(SourceError::Image)?;
        if end > bytes.len()
            || matches!(kind, b"acTL" | b"fcTL" | b"fdAT")
            || (offset == 8 && (kind != b"IHDR" || length != 13))
        {
            return Err(SourceError::Image);
        }
        // PNG CRC-32 covers type + data, but not the length or stored checksum.
        // The bounded container was checked above, so this adds no payload copy.
        let stored_crc = u32::from_be_bytes(
            bytes[end - 4..end]
                .try_into()
                .map_err(|_| SourceError::Image)?,
        );
        if crc32fast::hash(&bytes[offset + 4..end - 4]) != stored_crc {
            return Err(SourceError::Image);
        }
        if kind == b"IEND" {
            return if length == 0 && end == bytes.len() {
                Ok(())
            } else {
                Err(SourceError::Image)
            };
        }
        offset = end;
    }
    Err(SourceError::Image)
}
