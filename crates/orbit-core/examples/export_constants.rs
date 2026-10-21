//! Regenerate contracts/constants.json explicitly; tests only compare it.
use orbit_core::limits;
use serde_json::Value;
use std::{collections::BTreeSet, error::Error, path::Path};

pub fn render_registry(source: &str) -> Result<String, Box<dyn Error>> {
    let mut entries: Vec<Value> = serde_json::from_str(source)?;
    let mut rust_names = BTreeSet::new();
    for limit in limits::registry() {
        if !rust_names.insert(limit.name) {
            return Err(format!("Nombre Rust repetido: {}", limit.name).into());
        }
        let entry = entries
            .iter_mut()
            .find(|entry| entry["name"] == limit.name)
            .ok_or_else(|| format!("Falta {} en contracts/constants.json", limit.name))?;
        if entry["source"] != "rust" {
            return Err(format!("{} debe tener fuente rust", limit.name).into());
        }
        entry["value"] = Value::from(limit.value);
        entry["unit"] = Value::from(limit.unit);
        entry["resource"] = Value::from(limit.resource);
    }
    for entry in &entries {
        if entry["source"] == "rust"
            && !rust_names.contains(entry["name"].as_str().ok_or("Nombre ausente")?)
        {
            return Err(format!("{} ya no está en el registro Rust", entry["name"]).into());
        }
    }
    Ok(format!("{}\n", serde_json::to_string_pretty(&entries)?))
}

// The integration test imports render_registry from this example.
#[allow(dead_code)]
fn main() -> Result<(), Box<dyn Error>> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../contracts/constants.json");
    let source = std::fs::read_to_string(&path)?;
    std::fs::write(&path, render_registry(&source)?)?;
    Ok(())
}
