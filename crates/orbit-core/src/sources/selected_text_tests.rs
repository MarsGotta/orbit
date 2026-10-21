use super::*;

fn text_media(length: u64) -> SourceMedia {
    SourceMedia::SelectedText {
        range: SelectedTextRange {
            location: 7,
            length,
            unit: "utf16_code_units".into(),
        },
        ax_binding_quality: "unique_owner_title_bounds".into(),
        selection_scope: "application_focused_element".into(),
        sensitivity: "not_certified".into(),
    }
}

#[test]
fn text_preserves_unicode_and_controls_without_normalization() -> Result<(), SourceError> {
    for text in [
        "a",
        "á",
        "e\u{301}",
        "🛰",
        "\u{feff}A\r\n\t\u{1b}[31m\u{202e}x",
    ] {
        validate_source_bytes(
            text.as_bytes(),
            &text_media(text.encode_utf16().count() as u64),
        )?;
    }
    assert_eq!(
        validate_source_bytes("🛰".as_bytes(), &text_media(1)),
        Err(SourceError::Text)
    );
    assert_eq!(
        validate_source_bytes("e\u{301}".as_bytes(), &text_media(1)),
        Err(SourceError::Text)
    );
    assert_eq!(
        validate_source_bytes(b"\0", &text_media(1)),
        Err(SourceError::Text)
    );
    assert_eq!(
        validate_source_bytes(b"", &text_media(1)),
        Err(SourceError::Text)
    );
    Ok(())
}

#[test]
fn text_rejects_invalid_utf8_without_surrogate_or_replacement_repair() {
    for bytes in [
        &[0xed, 0xa0, 0x80][..], // UTF-8 encoding of an isolated high surrogate.
        &[0xed, 0xa0, 0xbd, 0xed, 0xb0, 0x80], // CESU-8 is not UTF-8.
        &[0xc0, 0xaf],           // Overlong encoding.
        &[0xf0, 0x9f, 0x9b],     // Incomplete scalar.
        &[0xff],
    ] {
        assert_eq!(
            validate_source_bytes(bytes, &text_media(1)),
            Err(SourceError::Text)
        );
    }
    // An actual replacement character is valid original data, never a repair operation.
    assert!(validate_source_bytes("�".as_bytes(), &text_media(1)).is_ok());
}

#[test]
fn text_and_range_limits_are_independent_and_overflow_safe() -> Result<(), SourceError> {
    validate_source_bytes("界".repeat(4096).as_bytes(), &text_media(4096))?;
    validate_source_bytes("🛰".repeat(2048).as_bytes(), &text_media(4096))?;
    assert_eq!(
        validate_source_bytes(&vec![b'x'; 16_385], &text_media(4096)),
        Err(SourceError::Bounds)
    );
    for length in [0, 4097, u64::MAX] {
        assert_eq!(
            validate_source_bytes(b"a", &text_media(length)),
            Err(SourceError::Bounds)
        );
    }
    let mut range = SelectedTextRange {
        location: i64::MAX as u64 - 1,
        length: 1,
        unit: "utf16_code_units".into(),
    };
    range.validate()?;
    for location in [i64::MAX as u64, u64::MAX] {
        range.location = location;
        assert_eq!(range.validate(), Err(SourceError::Bounds));
    }
    range.location = 0;
    range.unit = "unicode_scalars".into();
    assert_eq!(range.validate(), Err(SourceError::Scope));
    Ok(())
}

#[test]
fn text_binding_scope_and_sensitivity_are_exact_not_claims_of_safety()
-> Result<(), Box<dyn std::error::Error>> {
    for (field, invalid) in [
        ("ax_binding_quality", "unique_owner_title_bounds "),
        ("selection_scope", "whole_window"),
        ("sensitivity", "certified_public"),
    ] {
        let mut value = serde_json::to_value(text_media(1))?;
        value[field] = invalid.into();
        let media: SourceMedia = serde_json::from_value(value)?;
        assert_eq!(validate_source_bytes(b"a", &media), Err(SourceError::Scope));
    }
    let mut value = serde_json::to_value(text_media(1))?;
    value["range"]["location"] = (-1).into();
    assert!(serde_json::from_value::<SourceMedia>(value).is_err());
    Ok(())
}

#[test]
fn intent_format_and_exclusive_capture_deadlines_cannot_be_substituted() -> Result<(), SourceError>
{
    let mut intent = intent()?;
    let mut metadata = SourceMetadata {
        origin: intent.target.clone(),
        media: text_media(1),
        clock_domain: "mach_absolute_time".into(),
        capture_started_monotonic_ns: 2_000_000,
        capture_completed_monotonic_ns: 3_000_000,
        received_at_ms: 200,
    };
    assert_eq!(metadata.validate_for(&intent), Err(SourceError::Scope));
    intent.format = SourceFormat::SelectedText;
    metadata.validate_for(&intent)?;
    metadata.capture_completed_monotonic_ns =
        metadata.capture_started_monotonic_ns + SELECTED_TEXT_TIMEOUT_NS - 1;
    metadata.validate_for(&intent)?;
    metadata.capture_completed_monotonic_ns += 1;
    assert_eq!(metadata.validate_for(&intent), Err(SourceError::Interval));
    metadata.media = SourceMedia::Png {
        width: 2,
        height: 1,
    };
    assert_eq!(metadata.validate_for(&intent), Err(SourceError::Scope));
    intent.format = SourceFormat::Png;
    metadata.capture_completed_monotonic_ns =
        metadata.capture_started_monotonic_ns + CAPTURE_TIMEOUT_NS - 1;
    metadata.validate_for(&intent)?;
    metadata.capture_completed_monotonic_ns += 1;
    assert_eq!(metadata.validate_for(&intent), Err(SourceError::Interval));
    Ok(())
}
