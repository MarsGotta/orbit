#![allow(clippy::unwrap_used)]
use orbit_core::CapturePhase;
use orbit_core::refusal::{
    DeviceReplyRetry, EngineCause, Expected, HubSessionField, Operation, Parameter, Permission,
    Refusal, Resource, Retry, Service, UpdateCause, UpdateSizeSource,
};
use serde_json::json;

fn too_large_resources() -> [Resource; 15] {
    [
        Resource::Subscriptions,
        Resource::IpcResponse,
        Resource::IpcRequest,
        Resource::HubResponse,
        Resource::Message,
        Resource::Recording,
        Resource::Session,
        Resource::Audio,
        Resource::Calendar,
        Resource::Source,
        Resource::Capture,
        Resource::ComputerAction,
        Resource::Storage,
        Resource::Folder,
        Resource::File,
    ]
}

fn expected_too_large_retry(resource: Resource) -> Retry {
    match resource {
        Resource::IpcResponse => Retry::CheckOutcome,
        Resource::Subscriptions
        | Resource::IpcRequest
        | Resource::HubResponse
        | Resource::Message
        | Resource::Recording
        | Resource::Session
        | Resource::Audio
        | Resource::Calendar
        | Resource::Source
        | Resource::Capture
        | Resource::ComputerAction
        | Resource::Storage
        | Resource::Folder
        | Resource::File => Retry::AfterFix,
    }
}

fn cases() -> Vec<(Refusal, Retry)> {
    vec![
        (
            Refusal::PrivacyStopped { since_ms: Some(12) },
            Retry::AfterFix,
        ),
        (
            Refusal::PermissionDenied {
                permission: Permission::Microphone,
            },
            Retry::AfterFix,
        ),
        (
            Refusal::PermissionNotGranted {
                permission: Permission::Accessibility,
            },
            Retry::AfterFix,
        ),
        (
            Refusal::PermissionUnknown {
                permission: Permission::Calendar,
            },
            Retry::AfterFix,
        ),
        (Refusal::HelperNotRunning, Retry::AfterFix),
        (Refusal::CalendarBackendUnavailable, Retry::AfterFix),
        (
            Refusal::DeviceReplyInvalid {
                field: "result.windows".into(),
                expected: "lista".into(),
                device_version: "0.3.13".into(),
                retry: DeviceReplyRetry::AfterFix,
            },
            Retry::AfterFix,
        ),
        (
            Refusal::DeviceReplyTimeout {
                expected: "respuesta de perform_action".into(),
                deadline_ms: 30_000,
            },
            Retry::CheckOutcome,
        ),
        (Refusal::NotPaired, Retry::AfterFix),
        (
            Refusal::MacDisconnected {
                last_seen_ms: Some(13),
            },
            Retry::CheckOutcome,
        ),
        (
            Refusal::HubUnreachable { since_ms: Some(14) },
            Retry::CheckOutcome,
        ),
        (
            Refusal::HubSessionsUnpaged {
                delivered: 128,
                total: 500,
            },
            Retry::AfterFix,
        ),
        (
            Refusal::HubSessionInvalidRow {
                position: 1,
                rows: 128,
                field: HubSessionField::Id,
                bytes: 0,
                max_bytes: 121,
            },
            Retry::AfterFix,
        ),
        (
            Refusal::HubSessionsCursorMismatch { rows: 128 },
            Retry::AfterFix,
        ),
        (
            Refusal::EngineUnavailable {
                cause: EngineCause::NotRunning,
            },
            Retry::AfterFix,
        ),
        (
            Refusal::Busy {
                operation: Operation::Chat,
                retry_after_ms: Some(25),
            },
            Retry::AfterMs(25),
        ),
        (
            Refusal::PendingRecording {
                id: "recording-307".into(),
                title: "Reunión pendiente".into(),
                phase: CapturePhase::Interrupted,
                sealed_chunks: 2,
                gaps: 1,
                expires_at_ms: 61_000,
            },
            Retry::AfterFix,
        ),
        (
            Refusal::WorkInFlight {
                in_flight: 1,
                max: 1,
            },
            Retry::CheckOutcome,
        ),
        (
            Refusal::PhysicalWorkFinishing {
                in_flight: 1,
                max: 1,
            },
            Retry::CheckOutcome,
        ),
        (
            Refusal::TooLarge {
                resource: Resource::IpcResponse,
                bytes: 1_048_577,
                max_bytes: 1_048_576,
            },
            Retry::CheckOutcome,
        ),
        (
            Refusal::TooMany {
                resource: Resource::Subscriptions,
                count: 5,
                max: 4,
            },
            Retry::AfterFix,
        ),
        (
            Refusal::InvalidParameter {
                parameter: Parameter::Text,
                expected: Expected::NonEmpty,
            },
            Retry::AfterFix,
        ),
        (
            Refusal::RevisionChanged {
                expected_revision: 6,
                actual_revision: 7,
                expected_state_sha256: "a".repeat(64),
                actual_state_sha256: "b".repeat(64),
            },
            Retry::AfterFix,
        ),
        (
            Refusal::Timeout {
                operation: Operation::Chat,
                waited_ms: 10,
                deadline_ms: 8,
            },
            Retry::CheckOutcome,
        ),
        (
            Refusal::Expired {
                resource: Resource::Recording,
                expired_at_ms: Some(15),
            },
            Retry::AfterFix,
        ),
        (
            Refusal::NotFound {
                resource: Resource::Session,
            },
            Retry::AfterFix,
        ),
        (
            Refusal::OutcomeUnknown {
                operation_id: "op-1".into(),
            },
            Retry::CheckOutcome,
        ),
        (
            Refusal::WorkExpired {
                operation: Operation::Calendar,
                started_generation: 4,
                current_generation: 5,
            },
            Retry::CheckOutcome,
        ),
        (
            Refusal::UpstreamFailed {
                service: Service::Hub,
                status: Some(503),
            },
            Retry::CheckOutcome,
        ),
        (
            Refusal::StorageFailed {
                operation: Operation::Storage,
                cause: None,
            },
            Retry::CheckOutcome,
        ),
        (
            Refusal::Legacy {
                text: "negativa antigua".into(),
            },
            Retry::Never,
        ),
        (
            Refusal::Update {
                cause: UpdateCause::TooLarge {
                    file: "latest.txt".into(),
                    bytes: 8193,
                    max_bytes: 8192,
                    source: UpdateSizeSource::Observed,
                },
            },
            Retry::AfterFix,
        ),
    ]
}

#[test]
fn storage_failure_preserves_its_cause_and_retry_on_the_wire() {
    let refusal: Refusal = serde_json::from_value(serde_json::json!({
        "code": "storage_failed",
        "operation": "storage",
        "cause": "registro de notas dañado"
    }))
    .unwrap();
    let wire = serde_json::to_value(refusal).unwrap();
    assert_eq!(wire["code"], "storage_failed");
    assert_eq!(wire["cause"], "registro de notas dañado");
    assert_eq!(wire["retry"]["kind"], "check_outcome");
}

#[test]
fn each_family_serializes_cause_numbers_and_retry() {
    for (refusal, expected_retry) in cases() {
        assert_eq!(refusal.retry(), expected_retry);
        let value = serde_json::to_value(&refusal).unwrap();
        assert_eq!(
            value["retry"],
            serde_json::to_value(expected_retry).unwrap()
        );
        assert!(value["code"].as_str().is_some());
        assert_eq!(serde_json::from_value::<Refusal>(value).unwrap(), refusal);
    }
    let unknown = Refusal::OutcomeUnknown {
        operation_id: "op-9".into(),
    };
    assert_eq!(unknown.retry(), Retry::CheckOutcome);
    assert!(unknown.to_string().contains("op-9"));
    let sized = Refusal::TooLarge {
        resource: Resource::IpcResponse,
        bytes: 123,
        max_bytes: 100,
    };
    assert!(sized.to_string().contains("123"));
    assert!(sized.to_string().contains("100"));
}

#[test]
fn resource_sizes_keep_the_existing_retry_policy() {
    for resource in too_large_resources() {
        let refusal = Refusal::TooLarge {
            resource,
            bytes: 101,
            max_bytes: 100,
        };
        assert_eq!(refusal.retry(), expected_too_large_retry(resource));
    }
}

#[test]
fn decisions_preserve_actual_text_and_the_call_duration_bound() {
    use orbit_core::{Connection, Event, State, decide};
    let online = decide(&State::default(), Event::Connect(Connection::Online), 0)
        .unwrap()
        .state;
    let text_bytes = orbit_core::MAX_TEXT_BYTES + 1;
    let text = decide(
        &online,
        Event::Submit {
            attachments: Vec::new(),
            id: "synthetic".into(),
            text: "x".repeat(text_bytes),
        },
        0,
    )
    .unwrap_err();
    assert_eq!(
        text.clone(),
        Refusal::InvalidParameter {
            parameter: Parameter::Text,
            expected: Expected::TextLength {
                bytes: text_bytes as u64,
                max_bytes: orbit_core::MAX_TEXT_BYTES as u64
            },
        }
    );
    let text_wire = serde_json::to_value(text.clone()).unwrap();
    assert_eq!(
        text_wire["expected"],
        json!({"kind":"text_length","bytes":text_bytes,"max_bytes":orbit_core::MAX_TEXT_BYTES})
    );
    assert_eq!(serde_json::from_value::<Refusal>(text_wire).unwrap(), text);
    let whitespace = decide(
        &online,
        Event::Submit {
            attachments: Vec::new(),
            id: "blank".into(),
            text: "   ".into(),
        },
        0,
    )
    .unwrap_err();
    assert_eq!(
        whitespace,
        Refusal::InvalidParameter {
            parameter: Parameter::Text,
            expected: Expected::TextLength {
                bytes: 3,
                max_bytes: orbit_core::MAX_TEXT_BYTES as u64
            },
        }
    );
    let ready = decide(&online, Event::HelperReady, 0).unwrap().state;
    let mut with_audio = decide(
        &ready,
        Event::PermissionObserved {
            microphone: orbit_core::PermissionState::Granted,
            screen: orbit_core::PermissionState::Unknown,
            accessibility: orbit_core::PermissionState::Unknown,
            calendar: orbit_core::calendar::CalendarAuthorization::Unknown,
        },
        0,
    )
    .unwrap()
    .state;
    with_audio.set_voice_processing_supported(true);
    let call = decide(
        &with_audio,
        Event::CallStart {
            id: "call".into(),
            duration_ms: 14_400_001,
            origin: orbit_core::CallOrigin::App,
        },
        0,
    )
    .unwrap_err();
    assert_eq!(
        call.clone(),
        Refusal::InvalidParameter {
            parameter: Parameter::Duration,
            expected: Expected::DurationRange {
                actual_ms: 14_400_001,
                min_ms: 60_000,
                max_ms: 14_400_000
            },
        }
    );
    let call_wire = serde_json::to_value(call).unwrap();
    assert_eq!(
        call_wire["expected"],
        json!({"kind":"duration_range","actual_ms":14_400_001,"min_ms":60_000,"max_ms":14_400_000})
    );
}

#[test]
fn title_length_carries_both_numbers_on_the_wire_and_in_text() {
    let refusal = Refusal::InvalidParameter {
        parameter: Parameter::Title,
        expected: Expected::TitleLength {
            characters: 257,
            max_characters: 256,
        },
    };
    let text = refusal.to_string();

    assert!(text.contains("257"), "{text}");
    assert!(text.contains("256"), "{text}");
    let wire = serde_json::to_value(&refusal).unwrap();
    assert_eq!(
        wire["expected"],
        json!({"kind":"title_length","characters":257,"max_characters":256})
    );
    assert_eq!(
        serde_json::from_value::<Refusal>(wire).unwrap(),
        refusal,
        "the title-length refusal must round-trip"
    );
    assert_eq!(refusal.retry(), Retry::AfterFix);
}

#[test]
fn invalid_reply_retry_is_not_silently_rewritten() {
    let invalid = json!({
        "code":"device_reply_invalid",
        "field":"result.windows",
        "expected":"lista",
        "device_version":"0.3.13",
        "retry":{"kind":"never"}
    });
    assert!(serde_json::from_value::<Refusal>(invalid).is_err());
}

#[test]
fn existing_refusal_wire_matches_main() {
    // Wire samples captured on main 3bd3250b, before several refusal
    // producers were retired. Stored legacy
    // refusals remain part of the contract even after producers are retired.
    let mut values: Vec<_> = cases()
        .into_iter()
        .map(|(refusal, _)| serde_json::to_value(refusal).unwrap())
        .collect();
    values.push(
        serde_json::to_value(Refusal::Recovering {
            step: orbit_core::RecoveryStep::Snapshot,
            elapsed_ms: 1234,
        })
        .unwrap(),
    );
    let expected: serde_json::Value = serde_json::from_str(
        r#"[
  {
    "code": "privacy_stopped",
    "retry": {
      "kind": "after_fix"
    },
    "since_ms": 12
  },
  {
    "code": "permission_denied",
    "permission": "microphone",
    "retry": {
      "kind": "after_fix"
    }
  },
  {
    "code": "permission_not_granted",
    "permission": "accessibility",
    "retry": {
      "kind": "after_fix"
    }
  },
  {
    "code": "permission_unknown",
    "permission": "calendar",
    "retry": {
      "kind": "after_fix"
    }
  },
  {
    "code": "helper_not_running",
    "retry": {
      "kind": "after_fix"
    }
  },
  {
    "code": "calendar_backend_unavailable",
    "retry": {
      "kind": "after_fix"
    }
  },
  {
    "code": "device_reply_invalid",
    "device_version": "0.3.13",
    "expected": "lista",
    "field": "result.windows",
    "retry": {
      "kind": "after_fix"
    }
  },
  {
    "code": "device_reply_timeout",
    "deadline_ms": 30000,
    "expected": "respuesta de perform_action",
    "retry": {
      "kind": "check_outcome"
    }
  },
  {
    "code": "not_paired",
    "retry": {
      "kind": "after_fix"
    }
  },
  {
    "code": "mac_disconnected",
    "last_seen_ms": 13,
    "retry": {
      "kind": "check_outcome"
    }
  },
  {
    "code": "hub_unreachable",
    "retry": {
      "kind": "check_outcome"
    },
    "since_ms": 14
  },
  {
    "code": "hub_sessions_unpaged",
    "delivered": 128,
    "retry": {
      "kind": "after_fix"
    },
    "total": 500
  },
  {
    "bytes": 0,
    "code": "hub_session_invalid_row",
    "field": "id",
    "max_bytes": 121,
    "position": 1,
    "retry": {
      "kind": "after_fix"
    },
    "rows": 128
  },
  {
    "code": "hub_sessions_cursor_mismatch",
    "retry": {
      "kind": "after_fix"
    },
    "rows": 128
  },
  {
    "cause": "not_running",
    "code": "engine_unavailable",
    "retry": {
      "kind": "after_fix"
    }
  },
  {
    "code": "busy",
    "operation": "chat",
    "retry": {
      "kind": "after_ms",
      "ms": 25
    },
    "retry_after_ms": 25
  },
  {
    "code": "pending_recording",
    "expires_at_ms": 61000,
    "gaps": 1,
    "id": "recording-307",
    "phase": "interrupted",
    "retry": {
      "kind": "after_fix"
    },
    "sealed_chunks": 2,
    "title": "Reunión pendiente"
  },
  {
    "code": "work_in_flight",
    "in_flight": 1,
    "max": 1,
    "retry": {
      "kind": "check_outcome"
    }
  },
  {
    "code": "physical_work_finishing",
    "in_flight": 1,
    "max": 1,
    "retry": {
      "kind": "check_outcome"
    }
  },
  {
    "bytes": 1048577,
    "code": "too_large",
    "max_bytes": 1048576,
    "resource": "ipc_response",
    "retry": {
      "kind": "check_outcome"
    }
  },
  {
    "code": "too_many",
    "count": 5,
    "max": 4,
    "resource": "subscriptions",
    "retry": {
      "kind": "after_fix"
    }
  },
  {
    "code": "invalid_parameter",
    "expected": {
      "kind": "non_empty"
    },
    "parameter": "text",
    "retry": {
      "kind": "after_fix"
    }
  },
  {
    "actual_revision": 7,
    "actual_state_sha256": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
    "code": "revision_changed",
    "expected_revision": 6,
    "expected_state_sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    "retry": {
      "kind": "after_fix"
    }
  },
  {
    "code": "timeout",
    "deadline_ms": 8,
    "operation": "chat",
    "retry": {
      "kind": "check_outcome"
    },
    "waited_ms": 10
  },
  {
    "code": "expired",
    "expired_at_ms": 15,
    "resource": "recording",
    "retry": {
      "kind": "after_fix"
    }
  },
  {
    "code": "not_found",
    "resource": "session",
    "retry": {
      "kind": "after_fix"
    }
  },
  {
    "code": "outcome_unknown",
    "operation_id": "op-1",
    "retry": {
      "kind": "check_outcome"
    }
  },
  {
    "code": "work_expired",
    "current_generation": 5,
    "operation": "calendar",
    "retry": {
      "kind": "check_outcome"
    },
    "started_generation": 4
  },
  {
    "code": "upstream_failed",
    "retry": {
      "kind": "check_outcome"
    },
    "service": "hub",
    "status": 503
  },
  {
    "code": "storage_failed",
    "operation": "storage",
    "retry": {
      "kind": "check_outcome"
    }
  },
  {
    "code": "legacy",
    "retry": {
      "kind": "never"
    },
    "text": "negativa antigua"
  },
  {
    "cause": {
      "bytes": 8193,
      "file": "latest.txt",
      "kind": "too_large",
      "max_bytes": 8192,
      "source": "observed"
    },
    "code": "update",
    "retry": {
      "kind": "after_fix"
    }
  },
  {
    "code": "recovering",
    "elapsed_ms": 1234,
    "retry": {
      "kind": "never"
    },
    "step": "snapshot"
  }
]"#,
    )
    .unwrap();
    assert_eq!(serde_json::to_value(values).unwrap(), expected);
}

/// The assistant's folders refuse by name: the path asked for, where it led,
/// every folder, the cause and its numbers, and each one can be corrected.
#[test]
fn folder_refusals_carry_the_path_the_folders_and_their_numbers() {
    use orbit_core::assistant_files::{ContentKind, FileCause, FolderCause};
    let cases = [
        (
            Refusal::NoFolders,
            json!({"code":"no_folders","retry":{"kind":"after_fix"}}),
        ),
        (
            Refusal::OutsideFolders {
                path: "/Users/ejemplo/Proyectos/atajo".into(),
                resolved: Some("/Users/ejemplo/Privado/x".into()),
                folders: vec!["/Users/ejemplo/Proyectos".into()],
            },
            json!({"code":"outside_folders","path":"/Users/ejemplo/Proyectos/atajo",
                "resolved":"/Users/ejemplo/Privado/x","folders":["/Users/ejemplo/Proyectos"],
                "retry":{"kind":"after_fix"}}),
        ),
        (
            Refusal::FolderUnavailable {
                path: "/Users/ejemplo/Notas".into(),
                cause: FolderCause::BookmarkUnresolvable {
                    domain: "NSCocoaErrorDomain".into(),
                    code: 4,
                },
            },
            json!({"code":"folder_unavailable","path":"/Users/ejemplo/Notas",
                "cause":{"kind":"bookmark_unresolvable","domain":"NSCocoaErrorDomain","code":4},
                "retry":{"kind":"after_fix"}}),
        ),
        (
            Refusal::FileUnavailable {
                path: "/Users/ejemplo/Notas/a.bin".into(),
                cause: FileCause::NotText,
            },
            json!({"code":"file_unavailable","path":"/Users/ejemplo/Notas/a.bin",
                "cause":{"kind":"not_text"},"retry":{"kind":"after_fix"}}),
        ),
        (
            Refusal::FileTooLarge {
                path: "/Users/ejemplo/Notas/manual.pdf".into(),
                content: ContentKind::Pdf,
                bytes: 70_000_000,
                max_bytes: 67_108_864,
            },
            json!({"code":"file_too_large","path":"/Users/ejemplo/Notas/manual.pdf",
                "content":"pdf","bytes":70_000_000,"max_bytes":67_108_864,
                "retry":{"kind":"after_fix"}}),
        ),
    ];
    for (refusal, wire) in cases {
        assert_eq!(serde_json::to_value(&refusal).unwrap(), wire);
        assert_eq!(serde_json::from_value::<Refusal>(wire).unwrap(), refusal);
    }
}
