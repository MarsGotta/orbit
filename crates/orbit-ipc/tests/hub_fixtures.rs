use orbit_core::wire::hub::{HubEvent, HubEventKind, HubRequest};
use serde_json::Value;
use std::{collections::BTreeSet, error::Error, fs, path::Path};

fn request_name(frame: &HubRequest) -> Option<&str> {
    match frame {
        HubRequest::ChatSend { .. } => Some("chat.send"),
        HubRequest::ChatInterrupt { .. } => Some("chat.interrupt"),
        HubRequest::ApprovalRespond { .. } => Some("approval.respond"),
        HubRequest::ClarifyRespond { .. } => Some("clarify.respond"),
        HubRequest::Ping { .. } => Some("ping"),
        HubRequest::DeviceReply { .. } => Some("device.reply"),
        HubRequest::Unrecognized { .. } => None,
    }
}
fn event_name(frame: &HubEventKind) -> Option<&str> {
    match frame {
        HubEventKind::Ready(_) => Some("ready"),
        HubEventKind::Pong(_) => Some("pong"),
        HubEventKind::ChatInterrupted(_) => Some("chat.interrupted"),
        HubEventKind::ChatSession(_) => Some("chat.session"),
        HubEventKind::ChatDelta(_) => Some("chat.delta"),
        HubEventKind::ChatCompleted(_) => Some("chat.completed"),
        HubEventKind::ChatStatus(_) => Some("chat.status"),
        HubEventKind::ApprovalRequired(_) => Some("approval.required"),
        HubEventKind::ClarifyRequired(_) => Some("clarify.required"),
        HubEventKind::InteractionResolved(_) => Some("interaction.resolved"),
        HubEventKind::InteractionExpired(_) => Some("interaction.expired"),
        HubEventKind::DeviceRequest(_) => Some("device.request"),
        HubEventKind::StateSnapshot(_) => Some("state.snapshot"),
        HubEventKind::Error(_) => Some("error"),
        HubEventKind::Revoked(_) => Some("revoked"),
        HubEventKind::RecordingChanged(_) => Some("recording.changed"),
        HubEventKind::Unrecognized { .. } => None,
    }
}
fn fixture_names(directory: &Path) -> Result<BTreeSet<String>, Box<dyn Error>> {
    fs::read_dir(directory)?
        .map(|entry| {
            let entry = entry?;
            let path = entry.path();
            if path
                .extension()
                .is_some_and(|extension| extension == "json")
            {
                Ok(path
                    .file_stem()
                    .ok_or("fixture without name")?
                    .to_string_lossy()
                    .into_owned())
            } else {
                Err(format!("unexpected fixture file: {}", path.display()).into())
            }
        })
        .collect()
}
#[test]
fn every_hub_variant_has_one_consumed_fixture() -> Result<(), Box<dyn Error>> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../contracts/fixtures/hub");
    let requests = root.join("requests");
    let events = root.join("events");
    let expected_requests: BTreeSet<String> = [
        "chat.send",
        "chat.interrupt",
        "approval.respond",
        "clarify.respond",
        "ping",
        "device.reply",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    let expected_events: BTreeSet<String> = [
        "ready",
        "pong",
        "chat.interrupted",
        "chat.session",
        "chat.delta",
        "chat.completed",
        "chat.status",
        "approval.required",
        "clarify.required",
        "interaction.resolved",
        "interaction.expired",
        "device.request",
        "state.snapshot",
        "error",
        "revoked",
        "recording.changed",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    assert_eq!(fixture_names(&requests)?, expected_requests);
    assert_eq!(fixture_names(&events)?, expected_events);
    for name in expected_requests {
        let raw: Value = serde_json::from_slice(&fs::read(requests.join(format!("{name}.json")))?)?;
        let request = HubRequest::decode(raw.clone())?;
        assert_eq!(request_name(&request), Some(name.as_str()));
        assert_eq!(serde_json::to_value(request)?, raw);
    }
    for name in expected_events {
        let raw: Value = serde_json::from_slice(&fs::read(events.join(format!("{name}.json")))?)?;
        let frame = HubEvent::decode(raw.clone())?;
        assert_eq!(event_name(&frame.kind), Some(name.as_str()));
        assert_eq!(serde_json::to_value(frame)?, raw);
    }
    Ok(())
}
