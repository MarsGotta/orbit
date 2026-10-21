//! Encoder-produced CRC controls, then one-byte corruption. No live screenshots.
use super::{SourceError, validate_png};
use std::ops::Range;

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn image_with_chunk(
    kind: &[u8; 4],
    payload: &[u8],
    after_image: bool,
) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, 2, 1);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header()?;
        if !after_image {
            writer.write_chunk(png::chunk::ChunkType(*kind), payload)?;
        }
        writer.write_image_data(&[0, 0, 0, 255, 255, 255, 255, 255])?;
        if after_image {
            writer.write_chunk(png::chunk::ChunkType(*kind), payload)?;
        }
        writer.finish()?;
    }
    Ok(bytes)
}

fn chunk_range(bytes: &[u8], kind: &[u8; 4]) -> Result<Range<usize>, Box<dyn std::error::Error>> {
    let mut offset = 8;
    while offset < bytes.len() {
        let header = bytes
            .get(offset..offset + 8)
            .ok_or("fixture chunk header")?;
        let length = u32::from_be_bytes(header[..4].try_into()?) as usize;
        let end = offset + 12 + length;
        if &header[4..8] == kind {
            return Ok(offset..end);
        }
        offset = end;
    }
    Err("fixture missing chunk".into())
}

fn rejects_changed_crc(kind: &[u8; 4], payload: &[u8], after_image: bool) -> TestResult {
    let bytes = image_with_chunk(kind, payload, after_image)?;
    validate_png(&bytes, 2, 1)?; // Control must decode before the only mutated CRC byte.
    let chunk = chunk_range(&bytes, kind)?;
    let mut changed = bytes;
    changed[chunk.end - 1] ^= 1;
    assert_eq!(
        validate_png(&changed, 2, 1),
        Err(SourceError::Image),
        "a damaged CRC must not become durable evidence: {kind:?}"
    );
    Ok(())
}

#[test]
fn text_crc_is_checked_even_when_text_is_ignored() -> TestResult {
    rejects_changed_crc(b"tEXt", b"Comment\0synthetic text", false)
}

#[test]
fn gamma_crc_is_checked_even_if_decoder_recovers_from_it() -> TestResult {
    rejects_changed_crc(b"gAMA", &45_455u32.to_be_bytes(), false)
}

#[test]
fn icc_crc_is_checked_without_inflating_the_ignored_profile() -> TestResult {
    // Intentionally not a zlib profile: ignoring semantics must not ignore its CRC.
    rejects_changed_crc(b"iCCP", b"synthetic\0\0not-a-zlib-stream", false)
}

#[test]
fn unknown_ancillary_crc_is_checked() -> TestResult {
    rejects_changed_crc(b"orBt", b"unknown synthetic ancillary data", false)
}

#[test]
fn empty_ancillary_chunk_still_requires_crc_over_its_type() -> TestResult {
    rejects_changed_crc(b"orBt", b"", false)
}

#[test]
fn crc_covers_unknown_chunk_type_and_raw_payload() -> TestResult {
    let bytes = image_with_chunk(b"orBt", b"unchanged CRC", false)?;
    validate_png(&bytes, 2, 1)?;
    let chunk = chunk_range(&bytes, b"orBt")?;
    for changed_index in [chunk.start + 4, chunk.start + 8] {
        let mut changed = bytes.clone();
        changed[changed_index] ^= 1; // Another ancillary type or a data bit; keep CRC.
        assert_eq!(validate_png(&changed, 2, 1), Err(SourceError::Image));
    }
    Ok(())
}

#[test]
fn unbounded_chunk_lengths_are_rejected_before_crc_or_decoder_work() -> TestResult {
    let mut bytes = image_with_chunk(b"orBt", b"bounded", false)?;
    let chunk = chunk_range(&bytes, b"orBt")?;
    bytes[chunk.start..chunk.start + 4].copy_from_slice(&u32::MAX.to_be_bytes());
    assert_eq!(validate_png(&bytes, 2, 1), Err(SourceError::Image));
    Ok(())
}

#[test]
fn valid_container_crc_does_not_replace_pixel_stream_validation() -> TestResult {
    let mut bytes = image_with_chunk(b"orBt", b"control", false)?;
    let chunk = chunk_range(&bytes, b"IDAT")?;
    bytes[chunk.start + 8] ^= 0xff; // Damage the zlib stream, then give it a valid chunk CRC.
    let crc = crc32fast::hash(&bytes[chunk.start + 4..chunk.end - 4]);
    bytes[chunk.end - 4..chunk.end].copy_from_slice(&crc.to_be_bytes());
    assert_eq!(validate_png(&bytes, 2, 1), Err(SourceError::Image));
    Ok(())
}

#[test]
fn text_crc_after_image_data_is_checked() -> TestResult {
    rejects_changed_crc(b"tEXt", b"Comment\0synthetic trailer", true)
}

#[test]
fn critical_chunk_crc_controls_remain_strict() -> TestResult {
    let bytes = image_with_chunk(b"orBt", b"control", false)?;
    validate_png(&bytes, 2, 1)?;
    for kind in [b"IHDR", b"IDAT", b"IEND"] {
        let chunk = chunk_range(&bytes, kind)?;
        let mut changed = bytes.clone();
        changed[chunk.end - 1] ^= 1;
        assert_eq!(
            validate_png(&changed, 2, 1),
            Err(SourceError::Image),
            "{kind:?}"
        );
    }
    Ok(())
}

#[test]
fn every_truncation_and_trailing_data_are_still_rejected() -> TestResult {
    let bytes = image_with_chunk(b"tEXt", b"Comment\0bounded text", true)?;
    validate_png(&bytes, 2, 1)?;
    for end in 0..bytes.len() {
        assert_eq!(validate_png(&bytes[..end], 2, 1), Err(SourceError::Image));
    }
    let mut trailing = bytes.clone();
    trailing.push(0);
    assert_eq!(validate_png(&trailing, 2, 1), Err(SourceError::Image));
    trailing.extend_from_slice(&bytes[8..]);
    assert_eq!(validate_png(&trailing, 2, 1), Err(SourceError::Image));
    Ok(())
}

#[test]
fn valid_crc_does_not_make_animation_chunks_acceptable() -> TestResult {
    for kind in [b"acTL", b"fcTL", b"fdAT"] {
        for after_image in [false, true] {
            let bytes = image_with_chunk(kind, &[0; 26], after_image)?;
            assert_eq!(validate_png(&bytes, 2, 1), Err(SourceError::Image));
        }
    }
    Ok(())
}
