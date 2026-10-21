use orbit_core::wire::hub::HubRequest;
use orbit_core::wire::native::NativeCommand;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::error::Error;
use std::io;

const HUB_FIXTURE: &str = include_str!("../../../contracts/fixtures/hub-frame-v1.json");
const NATIVE_FIXTURE: &str = include_str!("../../../contracts/fixtures/native-frame-v1.json");

fn fixture_frames(source: &str) -> Result<Vec<Value>, Box<dyn Error>> {
    let document: Value = serde_json::from_str(source)?;
    document
        .get("frames")
        .and_then(Value::as_array)
        .cloned()
        .ok_or_else(|| io::Error::other("wire fixture does not contain a frames array").into())
}

fn assert_typed_round_trip<T>(source: &str) -> Result<(), Box<dyn Error>>
where
    T: DeserializeOwned + Serialize,
{
    for raw in fixture_frames(source)? {
        let frame: T = serde_json::from_value(raw.clone())?;
        assert_eq!(serde_json::to_value(frame)?, raw);
    }
    Ok(())
}

#[test]
fn shared_wire_fixtures_round_trip_through_typed_frames() -> Result<(), Box<dyn Error>> {
    assert_typed_round_trip::<HubRequest>(HUB_FIXTURE)?;
    assert_typed_round_trip::<NativeCommand>(NATIVE_FIXTURE)?;
    Ok(())
}

#[test]
fn playback_clip_and_capture_stop_are_native_commands() -> Result<(), Box<dyn Error>> {
    let clip = NativeCommand::from_parts(
        "clip-1".to_owned(),
        "playback_clip",
        serde_json::json!({"playback_id":"play-1","mime_type":"audio/mpeg", "provider":"fixture", "audio_base64":"AQI="}),
    )?;
    assert_eq!(serde_json::to_value(clip)?["command"], "playback_clip");
    let stop =
        NativeCommand::from_parts("stop-1".to_owned(), "capture_stop", serde_json::json!({}))?;
    assert_eq!(serde_json::to_value(stop)?["command"], "capture_stop");
    for retired in ["perform_action", "calendar_select"] {
        assert!(
            NativeCommand::from_parts("retired-1".to_owned(), retired, serde_json::json!({}))
                .is_err(),
            "{retired} must not be issued to the native helper"
        );
    }
    Ok(())
}
