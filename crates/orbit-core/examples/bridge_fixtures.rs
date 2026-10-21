//! Generate bridge fixtures through the production core wire types.
use orbit_core::{
    calendar::{
        CalendarFieldStatus, CalendarInvitee, CalendarOccurrence, CalendarOversizedEvent,
        CalendarReference,
    },
    computer_control::ToolCall,
    device_bridge::Capability,
    refusal::Refusal,
    wire::{bridge::*, hub::*},
};
use serde::Serialize;
use serde_json::json;
use std::{error::Error, fs, path::Path};

fn write<T: Serialize>(path: &Path, frame: &T) -> Result<(), Box<dyn Error>> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, format!("{}\n", serde_json::to_string_pretty(frame)?))?;
    Ok(())
}

fn main() -> Result<(), Box<dyn Error>> {
    let output = std::env::args()
        .nth(1)
        .ok_or("fixture directory required")?;
    let output = Path::new(&output);
    let id = "00000000-0000-4000-8000-000000000405";
    let requests = [
        BridgeRequest::Screenshot {
            parameters: ScreenshotParameters {
                application: Some("Safari".into()),
                title: Some("Correo".into()),
                full_screen: Some(false),
            },
        },
        BridgeRequest::ScreenSources {
            parameters: EmptyParameters::default(),
        },
        BridgeRequest::SelectedText {
            parameters: SelectedTextParameters {
                application: Some("Safari".into()),
                title: Some("Correo".into()),
            },
        },
        BridgeRequest::ControlTools {
            parameters: ControlToolsParameters {
                after_index: Some(1),
            },
        },
        BridgeRequest::PerformAction {
            parameters: PerformActionParameters {
                action: ToolCall {
                    tool: "click".into(),
                    arguments: json!({"x": 1}),
                },
            },
        },
        BridgeRequest::CalendarRead {
            parameters: CalendarReadParameters {
                start_ms: 1_790_200_800_000,
                end_ms: 1_790_287_200_000,
                read_id: None,
                after_ms: None,
                after_index: None,
                after_invitee: None,
            },
        },
    ];
    let results = [
        BridgeResult::Screenshot(ScreenshotResult {
            capture_id: "capture-1".into(),
            application: "Safari".into(),
            title: "Correo".into(),
            width: 100,
            height: 80,
            mime_type: "image/png".into(),
            sha256: "a".repeat(64),
            coordinates: None,
        }),
        BridgeResult::ScreenSources(ScreenSourcesResult {
            windows: vec![WindowSummary {
                application: "Safari".into(),
                title: "Correo".into(),
            }],
        }),
        BridgeResult::SelectedText(SelectedTextResult {
            application: "Safari".into(),
            title: "Correo".into(),
            text: "texto sintético".into(),
        }),
        BridgeResult::ControlTools(ControlToolsResult {
            tools: vec![],
            after_index: 1,
            total: 1,
            next_after_index: None,
        }),
        BridgeResult::PerformAction(PerformActionResult {
            receipt: ActionReceipt {
                operation_id: "operation-1".into(),
                tool: "click".into(),
                state: "performed".into(),
                attempted: true,
                driver_receipt: json!({"ok": true}),
                retry: "no_repetir".into(),
            },
        }),
        BridgeResult::CalendarRead(CalendarReadResult {
            read_id: "read-1".into(),
            requested: CalendarRequested {
                start_unix_ms: 1_790_200_800_000,
                end_unix_ms: 1_790_287_200_000,
            },
            read_at_unix_ms: Some(1_790_200_800_001),
            system_timezone: Some("Europe/Madrid".into()),
            provider_sync_status: "unknown".into(),
            enumeration: "bounded_local_read".into(),
            events_returned: 1,
            events: vec![CalendarBridgeEvent {
                occurrence: CalendarOccurrence {
                    observation_id: "00000000-0000-4000-8000-000000000406".into(),
                    calendar: CalendarReference {
                        calendar_id: "work".into(),
                        source_id: "account".into(),
                    },
                    calendar_item_id: "item-1".into(),
                    event_id: None,
                    original_occurrence_unix_ms: None,
                    start_unix_ms: 1_790_200_830_000,
                    end_unix_ms: 1_790_204_430_000,
                    event_timezone: None,
                    interpreted_timezone: "Europe/Madrid".into(),
                    is_floating: true,
                    is_all_day: false,
                    is_detached: false,
                    title: "Reunión sintética".into(),
                    status: "confirmed".into(),
                    last_modified_unix_ms: None,
                    invitees: Some(vec![CalendarInvitee {
                        name: Some("Invitada sintética".into()),
                        participant_url: "mailto:guest@example.invalid".into(),
                        email: Some("guest@example.invalid".into()),
                        role: "required".into(),
                        response_status: "accepted".into(),
                        participant_type: "person".into(),
                        is_current_user: false,
                    }]),
                    invitees_status: CalendarFieldStatus::Provided,
                    event_url: None,
                    event_url_status: CalendarFieldStatus::NotRequested,
                    location: None,
                    notes: None,
                    notes_left_out: None,
                    organizer: None,
                    availability: None,
                    recurrence: None,
                    can_edit: None,
                    alarms_minutes: None,
                    alarms_not_editable: None,
                },
                invitees_continued: None,
            }],
            omitted: vec![CalendarOmission {
                reason: "read_limit".into(),
                count: 1,
            }],
            oversized: Some(vec![CalendarOversizedEvent {
                calendar: CalendarReference {
                    calendar_id: "work".into(),
                    source_id: "account".into(),
                },
                calendar_item_id: Some("item-oversized".into()),
                start_unix_ms: 1_790_200_840_000,
                end_unix_ms: 1_790_204_440_000,
                field: "title".into(),
                bytes: 100_000,
                limit: 98_304,
            }]),
            next_after_ms: Some(1_790_200_860_000),
            next_after_index: Some(1),
            next_after_invitee: None,
        }),
    ];
    for ((capability, request), result) in Capability::ALL.into_iter().zip(requests).zip(results) {
        let name = capability.wire_name();
        let event = HubEvent {
            id: None,
            kind: HubEventKind::DeviceRequest(DeviceRequestData {
                request_id: id.into(),
                request,
            }),
        };
        write(
            &output.join("requests").join(format!("{name}.json")),
            &event,
        )?;
        for (suffix, result, refusal) in [
            ("result", Some(result), None),
            (
                "refusal",
                None,
                Some(Refusal::PrivacyStopped { since_ms: Some(1) }),
            ),
        ] {
            let reply = HubRequest::DeviceReply {
                v: HUB_PROTOCOL,
                id: id.into(),
                params: Box::new(DeviceReplyParams {
                    request_id: id.into(),
                    result,
                    refusal: refusal.map(|reason| BridgeRefusal {
                        reason,
                        detail: None,
                        retry_in_ms: None,
                        interruption: None,
                    }),
                }),
            };
            write(
                &output.join("replies").join(format!("{name}-{suffix}.json")),
                &reply,
            )?;
        }
    }
    Ok(())
}
