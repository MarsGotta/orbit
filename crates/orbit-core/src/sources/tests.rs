use super::*;

fn binding() -> WindowBinding {
    WindowBinding {
        native_instance_id: "00000000-0000-4000-8000-000000000001".into(),
        privacy_generation: 2,
        inventory_id: "00000000-0000-4000-8000-000000000002".into(),
        selection_token: "00000000-0000-4000-8000-000000000003".into(),
        window_id: 123,
        application_pid: 456,
        application_instance_id: "00000000-0000-4000-8000-000000000004".into(),
        binding_quality: "observed_owner_and_window".into(),
        descriptor_hash: "a".repeat(64),
    }
}

fn inventory() -> WindowInventory {
    let binding = binding();
    WindowInventory {
        native_instance_id: binding.native_instance_id.clone(),
        privacy_generation: binding.privacy_generation,
        inventory_id: binding.inventory_id.clone(),
        clock_domain: "mach_absolute_time".into(),
        observed_monotonic_ns: 1_000_000,
        expires_monotonic_ns: 30_001_000_000,
        selected_text_available: false,
        windows: vec![WindowCandidate {
            window_id: binding.window_id,
            application_pid: binding.application_pid,
            binding: Some(binding),
            application_name: "Editor".into(),
            bundle_id: "example.editor".into(),
            title: "Notas de prueba".into(),
            title_truncated: false,
            unavailable_reason: None,
        }],
        omitted_count: 0,
    }
}

fn intent() -> Result<SourceIntent, SourceError> {
    Ok(SourceIntent {
        id: "00000000-0000-4000-8000-000000000005".into(),
        device_id: None,
        session_id: None,
        generation: 7,
        requested_at_ms: 100,
        expires_at_ms: 30_100,
        target: inventory().select(&binding().selection_token)?,
        annotation: "Fuente local; no transmitir.".into(),
        format: SourceFormat::Png,
    })
}

fn png_bytes() -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, 2, 1);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()?
            .write_image_data(&[0, 0, 0, 255, 255, 255, 255, 255])?;
    }
    Ok(bytes)
}

#[test]
fn only_available_exact_inventory_bindings_are_selectable() -> Result<(), SourceError> {
    let original = inventory();
    original.select(&binding().selection_token)?.validate()?;
    assert!(original.select("an-old-token").is_err());
    let mut changed = original.clone();
    changed.windows[0]
        .binding
        .as_mut()
        .ok_or(SourceError::Identity)?
        .privacy_generation += 1;
    assert!(changed.validate().is_err());
    changed = original.clone();
    changed.windows.push(changed.windows[0].clone());
    assert!(changed.validate().is_err());
    changed = original;
    changed.windows[0].binding = None;
    changed.windows[0].unavailable_reason = Some("owner_identity_unavailable".into());
    changed.validate()?;
    assert!(changed.select(&binding().selection_token).is_err());
    Ok(())
}

#[test]
fn descriptors_and_clocks_are_bounded_without_inferring_selection_text() {
    for bad in [0, 1_000_000, u64::MAX] {
        let mut value = inventory();
        value.expires_monotonic_ns = bad;
        assert!(value.validate().is_err());
    }
    let mut value = inventory();
    value.selected_text_available = true;
    assert!(value.validate().is_ok()); // Capability is not a claim about any selection.
    value = inventory();
    value.windows[0].title = "á".repeat(129);
    assert_eq!(value.validate(), Err(SourceError::Bounds));
    value = inventory();
    value.clock_domain = "wall_clock".into();
    assert!(value.validate().is_err());
}

#[test]
fn receipt_cannot_substitute_source_or_claim_a_later_capture() -> Result<(), SourceError> {
    let intent = intent()?;
    let mut metadata = SourceMetadata {
        origin: intent.target.clone(),
        media: SourceMedia::Png {
            width: 2,
            height: 1,
        },
        clock_domain: "mach_absolute_time".into(),
        capture_started_monotonic_ns: 2_000_000,
        capture_completed_monotonic_ns: 3_000_000,
        received_at_ms: 200,
    };
    metadata.validate_for(&intent)?;
    metadata.capture_completed_monotonic_ns = 20_000_000_000;
    assert_eq!(metadata.validate_for(&intent), Err(SourceError::Interval));
    metadata.capture_completed_monotonic_ns = 3_000_000;
    metadata.capture_started_monotonic_ns = 30_001_000_000;
    metadata.capture_completed_monotonic_ns = 30_001_000_000;
    assert_eq!(metadata.validate_for(&intent), Err(SourceError::Interval));
    metadata.capture_started_monotonic_ns = 29_001_000_000;
    assert_eq!(metadata.validate_for(&intent), Err(SourceError::Interval));
    metadata.capture_started_monotonic_ns = 2_000_000;
    metadata.capture_completed_monotonic_ns = 3_000_000;
    let SourceTarget::NativeWindow { binding, .. } = &mut metadata.origin;
    binding.application_pid += 1;
    assert_eq!(metadata.validate_for(&intent), Err(SourceError::Scope));
    Ok(())
}

#[path = "selected_text_tests.rs"]
mod selected_text_tests;

#[test]
fn png_requires_matching_dimensions_and_bounds() -> Result<(), Box<dyn std::error::Error>> {
    let bytes = png_bytes()?;
    validate_png(&bytes, 2, 1)?;
    assert!(validate_png(&bytes, 1, 2).is_err());
    assert_eq!(
        validate_png(&vec![0; MAX_SOURCE_BYTES + 1], 2, 1),
        Err(SourceError::Bounds)
    );
    assert_eq!(validate_png(&bytes, u32::MAX, 1), Err(SourceError::Bounds));
    Ok(())
}

#[test]
fn unknown_json_contract_fields_cannot_smuggle_publication_authority()
-> Result<(), Box<dyn std::error::Error>> {
    let mut value = serde_json::to_value(intent()?)?;
    value["publish"] = true.into();
    assert!(serde_json::from_value::<SourceIntent>(value).is_err());
    Ok(())
}
