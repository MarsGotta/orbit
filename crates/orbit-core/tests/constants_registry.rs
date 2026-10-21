#![allow(clippy::expect_used)]
#[path = "../examples/export_constants.rs"]
mod export_constants;

use std::collections::BTreeSet;

#[test]
fn registry_matches_rust() {
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../contracts/constants.json");
    let source = std::fs::read_to_string(&path).expect("registry is checked in");
    let entries: Vec<serde_json::Value> = serde_json::from_str(&source).expect("registry parses");
    let names = entries
        .iter()
        .map(|entry| entry["name"].as_str().expect("name"))
        .collect::<BTreeSet<_>>();
    assert_eq!(names.len(), entries.len(), "each resource needs one name");
    assert!(entries.iter().all(|entry| {
        let valid_value = if entry["unit"] == "path" {
            entry["value"]
                .as_str()
                .is_some_and(|value| !value.is_empty())
        } else {
            entry["value"].as_u64().is_some_and(|value| value > 0)
        };
        valid_value
            && entry["resource"]
                .as_str()
                .is_some_and(|resource| !resource.is_empty())
            && entry["unit"].as_str().is_some_and(|unit| !unit.is_empty())
            && entry["copies"].is_array()
    }));
    assert_eq!(
        source,
        export_constants::render_registry(&source).expect("registry renders"),
        "contracts/constants.json difiere del registro Rust; ejecuta cargo run -p orbit-core --example export_constants --locked"
    );
}
