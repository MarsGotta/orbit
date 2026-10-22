//! Generate synthetic hub fixtures through the production Rust wire types.
use orbit_core::wire::bridge::{BridgeRequest, DeviceRequestData, ScreenshotParameters};
use orbit_core::wire::hub::*;
use serde::Serialize;
use serde_json::{Value, json};
use std::{error::Error, fs, path::Path};

fn write<T: Serialize>(directory: &Path, name: &str, frame: &T) -> Result<(), Box<dyn Error>> {
    fs::create_dir_all(directory)?;
    let path = directory.join(format!("{name}.json"));
    fs::write(path, format!("{}\n", serde_json::to_string_pretty(frame)?))?;
    Ok(())
}

fn main() -> Result<(), Box<dyn Error>> {
    let path = std::env::args()
        .nth(1)
        .ok_or("fixture directory required")?;
    let path = Path::new(&path);
    let id = "00000000-0000-4000-8000-000000000901";
    let session = "00000000-0000-4000-8000-000000000902";
    let request = |name: &str, params: Value| -> Result<HubRequest, String> {
        HubRequest::new(id.into(), name, params)
    };
    for (name, params) in [
        (
            "chat.send",
            json!({
                "text":"mensaje sintético","session_id":session,"call_active":false,
                "client_id":"00000000-0000-4000-8000-000000000903",
                "voice":{"duration_ms":4200},
                "meeting":{
                    "recording_id":"00000000-0000-4000-8000-000000000904",
                    "minutes":2,"title":"Reunión sintética"
                },
                "attachments":["00000000-0000-4000-8000-000000000905"]
            }),
        ),
        ("chat.interrupt", json!({"session_id":session,"turn_id":id})),
        (
            "approval.respond",
            json!({"session_id":session,"request_id":"approval-1","approved":false,"turn_id":id}),
        ),
        (
            "clarify.respond",
            json!({"session_id":session,"request_id":"clarify-1","answer":"sí"}),
        ),
        ("ping", json!({})),
        (
            "device.reply",
            json!({"request_id":"device-1","result":{"windows":[]}}),
        ),
    ] {
        write(&path.join("requests"), name, &request(name, params)?)?;
    }
    let event = |kind| HubEvent {
        id: Some(id.into()),
        kind,
    };
    let empty = EmptyData {};
    let required = RequiredData {
        session_id: session.into(),
        request_id: "approval-1".into(),
        description: Some("Permiso sintético".into()),
        question: None,
        kind: Some("approval".into()),
        state: Some("pending".into()),
        expires_at: None,
        choices: None,
        questions: None,
    };
    let interaction = InteractionData {
        session_id: session.into(),
        request_id: "approval-1".into(),
        status: Some("ok".into()),
        applied: Some(true),
        resolved: Some(1),
        remaining: None,
        stale_interaction: None,
        turn_id: None,
    };
    let frames = [
        (
            "ready",
            event(HubEventKind::Ready(ReadyData {
                assistant_name: Some("Atlas".into()),
                agents: Some(vec![
                    HubAgent {
                        id: "atlas".into(),
                        name: "Atlas".into(),
                        kind: "hermes".into(),
                        capabilities: vec!["images".into(), "voice".into(), "resume".into()],
                    },
                    HubAgent {
                        id: "vega".into(),
                        name: "Vega".into(),
                        kind: "hermes".into(),
                        capabilities: vec!["resume".into()],
                    },
                ]),
                default_agent_id: Some("atlas".into()),
                attachments: Some(orbit_core::attachments::AttachmentLimits {
                    max_file_bytes: 20 * 1024 * 1024,
                    max_message_files: 10,
                    max_message_bytes: 32 * 1024 * 1024,
                    max_session_bytes: 256 * 1024 * 1024,
                }),
                device_id: Some("device-1".into()),
                hub_protocol: Some(1),
                hub_version: Some("0.3.12".into()),
                device_version: None,
            })),
        ),
        ("pong", event(HubEventKind::Pong(empty.clone()))),
        (
            "chat.interrupted",
            event(HubEventKind::ChatInterrupted(InterruptedData {
                session_id: session.into(),
                status: "requested".into(),
            })),
        ),
        (
            "chat.session",
            event(HubEventKind::ChatSession(SessionData {
                session_id: session.into(),
            })),
        ),
        (
            "chat.delta",
            event(HubEventKind::ChatDelta(DeltaData {
                session_id: Some(session.into()),
                text: "parcial".into(),
            })),
        ),
        (
            "chat.completed",
            event(HubEventKind::ChatCompleted(CompletedData {
                session_id: Some(session.into()),
                text: "final".into(),
                status: Some("completed".into()),
                persisted: Some(true),
                warning: None,
            })),
        ),
        (
            "chat.status",
            event(HubEventKind::ChatStatus(StatusData {
                session_id: session.into(),
                kind: "status".into(),
                text: "procesando".into(),
            })),
        ),
        (
            "approval.required",
            event(HubEventKind::ApprovalRequired(required.clone())),
        ),
        (
            "clarify.required",
            event(HubEventKind::ClarifyRequired(RequiredData {
                request_id: "clarify-1".into(),
                description: None,
                question: Some("¿Confirmas?".into()),
                kind: Some("clarify".into()),
                ..required
            })),
        ),
        (
            "interaction.resolved",
            event(HubEventKind::InteractionResolved(interaction.clone())),
        ),
        (
            "interaction.expired",
            event(HubEventKind::InteractionExpired(InteractionData {
                status: Some("expired".into()),
                applied: Some(false),
                resolved: None,
                ..interaction
            })),
        ),
        (
            "device.request",
            event(HubEventKind::DeviceRequest(DeviceRequestData {
                request_id: "device-1".into(),
                request: BridgeRequest::Screenshot {
                    parameters: ScreenshotParameters::default(),
                },
            })),
        ),
        (
            "state.snapshot",
            event(HubEventKind::StateSnapshot(SnapshotData {
                revision: 1,
                sessions: vec![],
                omitted: Some(false),
            })),
        ),
        (
            "error",
            event(HubEventKind::Error(ErrorData {
                code: Some("invalid_request".into()),
                message: "Mensaje fuera de contrato".into(),
                session_id: None,
                status: None,
                refusal: None,
            })),
        ),
        ("revoked", event(HubEventKind::Revoked(empty))),
        (
            "recording.changed",
            event(HubEventKind::RecordingChanged(RecordingChangedData {
                recording_id: "00000000-0000-4000-8000-000000000903".into(),
            })),
        ),
    ];
    for (name, frame) in frames {
        write(&path.join("events"), name, &frame)?;
    }
    Ok(())
}
