use orbit_core::{
    device_bridge::Capability,
    wire::{
        bridge::BridgeResult,
        hub::{HubEvent, HubEventKind, HubRequest},
    },
};
use serde_json::Value;
use std::{collections::BTreeSet, error::Error, fs, path::Path};

fn names(path: &Path) -> Result<BTreeSet<String>, Box<dyn Error>> {
    fs::read_dir(path)?
        .map(|entry| {
            let entry = entry?;
            let path = entry.path();
            if path.extension().is_none_or(|extension| extension != "json") {
                return Err(format!("unexpected fixture: {}", path.display()).into());
            }
            Ok(path
                .file_stem()
                .ok_or("missing stem")?
                .to_string_lossy()
                .into_owned())
        })
        .collect()
}

#[test]
fn every_bridge_capability_has_a_consumed_request_and_both_replies() -> Result<(), Box<dyn Error>> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../contracts/fixtures/bridge");
    let expected: BTreeSet<String> = Capability::ALL
        .into_iter()
        .map(|capability| capability.wire_name().to_owned())
        .collect();
    assert_eq!(names(&root.join("requests"))?, expected);
    let expected_replies: BTreeSet<String> = expected
        .iter()
        .flat_map(|name| [format!("{name}-result"), format!("{name}-refusal")])
        .collect();
    assert_eq!(names(&root.join("replies"))?, expected_replies);
    for name in expected {
        let request: Value = serde_json::from_slice(&fs::read(
            root.join("requests").join(format!("{name}.json")),
        )?)?;
        let parsed = HubEvent::decode(request.clone())?;
        assert!(matches!(parsed.kind, HubEventKind::DeviceRequest(_)));
        assert_eq!(serde_json::to_value(parsed)?, request);
        for suffix in ["result", "refusal"] {
            let raw: Value = serde_json::from_slice(&fs::read(
                root.join("replies").join(format!("{name}-{suffix}.json")),
            )?)?;
            let parsed = HubRequest::decode(raw.clone())?;
            let HubRequest::DeviceReply { params, .. } = &parsed else {
                return Err("not a device reply".into());
            };
            assert_eq!(params.result.is_some(), suffix == "result");
            assert_eq!(params.refusal.is_some(), suffix == "refusal");
            if let Some(result) = &params.result {
                let shape_matches = matches!(
                    (name.as_str(), result),
                    ("screenshot", BridgeResult::Screenshot(_))
                        | ("screen_sources", BridgeResult::ScreenSources(_))
                        | ("selected_text", BridgeResult::SelectedText(_))
                        | ("control_tools", BridgeResult::ControlTools(_))
                        | ("perform_action", BridgeResult::PerformAction(_))
                        | ("calendar_read", BridgeResult::CalendarRead(_))
                );
                assert!(shape_matches, "wrong result for {name}");
            }
            assert_eq!(serde_json::to_value(parsed)?, raw);
        }
    }
    Ok(())
}
