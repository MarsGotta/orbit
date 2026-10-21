//! Native response decoding, without clocks, storage, capture or retry effects.
use anyhow::Result;
use base64::{Engine, engine::general_purpose::STANDARD};
use orbit_core::refusal::{Refusal, Resource};
use orbit_core::sources::{
    MAX_SOURCE_BYTES, SelectedTextRange, SourceFormat, SourceIntent, SourceMedia, SourceMetadata,
    WindowBinding, validate_source_bytes,
};
use orbit_core::wire::native::decode_result_body;
use serde::Deserialize;
use serde_json::Value;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Screenshot {
    binding: WindowBinding,
    clock_domain: String,
    capture_started_monotonic_ns: u64,
    capture_completed_monotonic_ns: u64,
    mime_type: String,
    width: u32,
    height: u32,
    data_base64: String,
    source: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SelectedText {
    binding: WindowBinding,
    clock_domain: String,
    capture_started_monotonic_ns: u64,
    capture_completed_monotonic_ns: u64,
    mime_type: String,
    source: String,
    text: String,
    range: SelectedTextRange,
    ax_binding_quality: String,
    selection_scope: String,
    sensitivity: String,
}

pub(super) fn decode(
    intent: &SourceIntent,
    result: &Value,
    received_at_ms: i64,
) -> Result<(SourceMetadata, Vec<u8>)> {
    let (metadata, bytes) = match intent.format {
        SourceFormat::Png => {
            let image: Screenshot =
                decode_result_body("screenshot", result).map_err(Refusal::helper_reply_invalid)?;
            if image.binding != *intent.target.binding()
                || image.source != "native_window"
                || image.mime_type != "image/png"
            {
                return Err(Refusal::HelperReplyInvalid {
                    field: "screenshot.binding/source/mime_type".into(),
                    expected: "authorized_window_png".into(),
                    received_type: "different_binding_or_format".into(),
                }
                .into());
            }
            let max_bytes = MAX_SOURCE_BYTES.div_ceil(3) * 4;
            if image.data_base64.len() > max_bytes {
                return Err(Refusal::TooLarge {
                    resource: Resource::Source,
                    bytes: image.data_base64.len() as u64,
                    max_bytes: max_bytes as u64,
                }
                .into());
            }
            (
                SourceMetadata {
                    origin: intent.target.clone(),
                    media: SourceMedia::Png {
                        width: image.width,
                        height: image.height,
                    },
                    clock_domain: image.clock_domain,
                    capture_started_monotonic_ns: image.capture_started_monotonic_ns,
                    capture_completed_monotonic_ns: image.capture_completed_monotonic_ns,
                    received_at_ms,
                },
                STANDARD.decode(image.data_base64)?,
            )
        }
        SourceFormat::SelectedText => {
            let text: SelectedText = decode_result_body("selected_text", result)
                .map_err(Refusal::helper_reply_invalid)?;
            if text.binding != *intent.target.binding()
                || text.source != "native_selected_text"
                || text.mime_type != "text/plain; charset=utf-8"
            {
                return Err(Refusal::HelperReplyInvalid {
                    field: "selected_text.binding/source/mime_type".into(),
                    expected: "authorized_window_utf8_text".into(),
                    received_type: "different_binding_or_format".into(),
                }
                .into());
            }
            (
                SourceMetadata {
                    origin: intent.target.clone(),
                    media: SourceMedia::SelectedText {
                        range: text.range,
                        ax_binding_quality: text.ax_binding_quality,
                        selection_scope: text.selection_scope,
                        sensitivity: text.sensitivity,
                    },
                    clock_domain: text.clock_domain,
                    capture_started_monotonic_ns: text.capture_started_monotonic_ns,
                    capture_completed_monotonic_ns: text.capture_completed_monotonic_ns,
                    received_at_ms,
                },
                text.text.into_bytes(),
            )
        }
    };
    metadata.validate_for(intent)?;
    validate_source_bytes(&bytes, &metadata.media)?;
    Ok((metadata, bytes))
}
