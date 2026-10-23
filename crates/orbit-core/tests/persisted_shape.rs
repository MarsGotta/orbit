//! The saved state keeps its shape byte for byte. Each fixture was written
//! by Orbit before its state was split by domain (`tests/fixtures/durable`):
//! it must load, and the loaded state must write back exactly the same bytes,
//! keys in the same order, so a snapshot on a Mac reads the same either way.
#![allow(clippy::unwrap_used)]
use orbit_core::{DurableState, State};
use std::path::{Path, PathBuf};

fn fixture(name: &str) -> (PathBuf, Vec<u8>) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/durable")
        .join(name);
    let bytes = std::fs::read(&path).unwrap();
    (path, bytes)
}

fn assert_same_bytes<T: serde::Serialize + serde::de::DeserializeOwned>(name: &str) {
    let (path, saved) = fixture(name);
    let loaded: T = serde_json::from_slice(&saved).unwrap();
    let written = serde_json::to_vec(&loaded).unwrap();
    assert_eq!(
        String::from_utf8_lossy(&written),
        String::from_utf8_lossy(&saved),
        "{} does not write back the bytes it was loaded from",
        path.display()
    );
}

#[test]
fn every_saved_durable_state_writes_back_its_own_bytes() {
    for name in [
        "default.json",
        "live_recording.json",
        "conversation_pending_voice.json",
        "pending_voice_offline_outbox.json",
        "call.json",
        "discard.json",
        "privacy_stop.json",
        "revoked.json",
        "cut_closed.json",
    ] {
        assert_same_bytes::<DurableState>(name);
    }
}

#[test]
fn the_served_state_with_its_views_writes_back_its_own_bytes() {
    assert_same_bytes::<State>("state_with_views.json");
}

#[test]
fn a_saved_state_is_the_durable_part_of_the_served_one() {
    // The served state is the saved one with the views beside it: the same
    // keys, in the same order, before the first view.
    let (_, served) = fixture("state_with_views.json");
    let state: State = serde_json::from_slice(&served).unwrap();
    let durable = serde_json::to_string(&state.durable).unwrap();
    let served = String::from_utf8(served).unwrap();
    assert!(served.starts_with(durable.trim_end_matches('}')));
}

#[test]
fn the_default_state_is_the_saved_default() {
    let (_, saved) = fixture("default.json");
    assert_eq!(serde_json::to_vec(&DurableState::default()).unwrap(), saved);
}
