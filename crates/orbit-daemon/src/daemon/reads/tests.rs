use super::*;
use crate::daemon::audio::tests::fixture;
use orbit_core::computer_control::MAX_CONTROL_HISTORY;
use serde_json::{Value, json};

/// The dispatcher with no agenda revision and the epoch as the hour:
/// neither matters to the control reads.
fn read(runtime: &mut Runtime, request: ReadRequest) -> Knowledge<Page<serde_json::Value>> {
    dispatch(runtime, request, 0)
}

#[tokio::test]
async fn transcript_rejects_a_cursor_of_another_shape() -> anyhow::Result<()> {
    let directory = tempfile::tempdir()?;
    let (runtime, _) = fixture(directory.path())?;
    // The IPC loop answers the notes reads with `notes_read`.
    let answer = runtime
        .notes_read(&ReadRequest::NoteTranscript {
            id: "00000000-0000-4000-8000-000000000209".into(),
            cursor: Some(Cursor::Id {
                id: "foreign".into(),
            }),
        })
        .ok_or_else(|| anyhow::anyhow!("NoteTranscript is not a notes read"))?
        .await;
    assert!(
        matches!(
            answer,
            Knowledge::Absent {
                refusal: Refusal::InvalidParameter { .. }
            }
        ),
        "a foreign cursor was not refused: {answer:?}"
    );
    Ok(())
}

#[test]
fn the_whole_control_history_reads_in_one_page() -> anyhow::Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _) = fixture(directory.path())?;
    let history: Vec<Value> = (0..orbit_core::computer_control::MAX_CONTROL_HISTORY)
        .map(|index| {
            json!({
                "operation_id": format!("op-{index:03}"),
                "requested": "click",
                "application": "Fixture",
                "channel": "local",
                "at_ms": 1_000 + index as i64,
                "outcome": "performed",
            })
        })
        .collect();
    std::fs::write(
        runtime.paths.root.join("computer-control.json"),
        serde_json::to_vec(&json!({"schema": 1, "history": history}))?,
    )?;
    let answer = read(&mut runtime, ReadRequest::ControlHistory { cursor: None });
    let Knowledge::Present { value, .. } = answer else {
        anyhow::bail!("the whole local record must read in one page, got {answer:?}");
    };
    assert_eq!(
        value.items.len(),
        orbit_core::computer_control::MAX_CONTROL_HISTORY
    );
    assert_eq!(value.next, None);
    assert_eq!(
        value.total,
        orbit_core::computer_control::MAX_CONTROL_HISTORY as u64
    );
    assert_eq!(value.items[0]["operation_id"], json!("op-000"));
    Ok(())
}

#[test]
fn maximal_valid_history_stays_within_the_read_ceiling() -> anyhow::Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _) = fixture(directory.path())?;
    let history: Vec<Value> = (0..MAX_CONTROL_HISTORY)
        .map(|index| {
            json!({
                "operation_id": format!("{index:03}{}", "i".repeat(125)),
                "requested": "r".repeat(128),
                "application": "a".repeat(256),
                "channel": "c".repeat(64),
                "at_ms": 1_000 + index as i64,
                "outcome": "performed",
            })
        })
        .collect();
    std::fs::write(
        runtime.paths.root.join("computer-control.json"),
        serde_json::to_vec(&json!({"schema": 1, "history": history}))?,
    )?;
    let first = read(&mut runtime, ReadRequest::ControlHistory { cursor: None });
    let Knowledge::Present { value, .. } = &first else {
        anyhow::bail!("maximal record must page, got {first:?}");
    };
    let ceiling = ReadRequest::ControlHistory { cursor: None }
        .id()
        .max_bytes();
    assert!(serde_json::to_vec(&first)?.len() <= ceiling);
    assert_eq!(value.total, MAX_CONTROL_HISTORY as u64);
    assert!(value.next.is_some());
    let mut seen = value.items.len();
    let mut cursor = value.next.clone();
    while let Some(next) = cursor {
        let answer = read(
            &mut runtime,
            ReadRequest::ControlHistory { cursor: Some(next) },
        );
        let Knowledge::Present { value, .. } = &answer else {
            anyhow::bail!("continuation must be present: {answer:?}");
        };
        assert!(serde_json::to_vec(&answer)?.len() <= ceiling);
        seen += value.items.len();
        cursor = value.next.clone();
    }
    assert_eq!(seen, MAX_CONTROL_HISTORY);
    Ok(())
}

#[test]
fn history_cursor_does_not_repeat_a_row_after_a_new_action() -> anyhow::Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _) = fixture(directory.path())?;
    let mut history: Vec<Value> = (0..MAX_CONTROL_HISTORY)
        .map(|index| {
            json!({
                "operation_id": format!("op-{index:03}"), "requested": "r".repeat(128),
                "application": "a".repeat(256), "channel": "c".repeat(64),
                "at_ms": 1_000 + index as i64, "outcome": "performed",
            })
        })
        .collect();
    let file = runtime.paths.root.join("computer-control.json");
    std::fs::write(
        &file,
        serde_json::to_vec(&json!({"schema":1,"history":history}))?,
    )?;
    let Knowledge::Present { value: first, .. } =
        read(&mut runtime, ReadRequest::ControlHistory { cursor: None })
    else {
        anyhow::bail!("first history page absent");
    };
    let seen: std::collections::HashSet<_> = first
        .items
        .iter()
        .map(|item| item["operation_id"].as_str().unwrap_or("").to_owned())
        .collect();
    let cursor = first
        .next
        .ok_or_else(|| anyhow::anyhow!("maximal rows did not page"))?;
    history.insert(
        0,
        json!({
            "operation_id":"new-action", "requested":"click", "application":"Fixture",
            "channel":"local", "at_ms": 2_000, "outcome":"performed",
        }),
    );
    history.truncate(MAX_CONTROL_HISTORY);
    std::fs::write(
        &file,
        serde_json::to_vec(&json!({"schema":1,"history":history}))?,
    )?;
    let Knowledge::Present { value: second, .. } = read(
        &mut runtime,
        ReadRequest::ControlHistory {
            cursor: Some(cursor),
        },
    ) else {
        anyhow::bail!("continuation absent");
    };
    assert!(!second.items.is_empty());
    assert!(
        second
            .items
            .iter()
            .all(|item| !seen.contains(item["operation_id"].as_str().unwrap_or("")))
    );
    Ok(())
}

#[tokio::test]
async fn cursors_of_other_shapes_and_missing_offsets_are_refused() -> anyhow::Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _) = fixture(directory.path())?;
    {
        let fixture_value = serde_json::json!(orbit_core::PermissionState::Granted);
        crate::daemon::authority_fixture::set(
            &mut runtime.state,
            "capabilities.accessibility_state",
            fixture_value,
        );
    }
    {
        let fixture_value = serde_json::json!(orbit_core::PermissionState::Granted);
        crate::daemon::authority_fixture::set(
            &mut runtime.state,
            "capabilities.screen_state",
            fixture_value,
        );
    }
    runtime.features.computer.declared_tools = Some(0);
    let generation = runtime.state.generation();
    runtime
        .features
        .computer
        .work
        .attach_test(generation, (), tokio::spawn(async {}));
    let invalid = Knowledge::Absent {
        refusal: Refusal::InvalidParameter {
            parameter: Parameter::Identifier,
            expected: Expected::ValidIdentifier,
        },
    };
    for request in [
        ReadRequest::ControlHistory {
            cursor: Some(Cursor::Name {
                name: "wrong".into(),
            }),
        },
        ReadRequest::ControlHistory {
            cursor: Some(Cursor::Index { index: 1 }),
        },
        ReadRequest::ControlHistory {
            cursor: Some(Cursor::Id {
                id: "missing".into(),
            }),
        },
        ReadRequest::EngineTools {
            cursor: Some(Cursor::Name {
                name: "wrong".into(),
            }),
        },
        ReadRequest::EngineTools {
            cursor: Some(Cursor::Index { index: 1 }),
        },
    ] {
        assert_eq!(read(&mut runtime, request), invalid);
    }
    Ok(())
}

#[test]
fn an_unreadable_record_refuses_with_its_cause() -> anyhow::Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _) = fixture(directory.path())?;
    std::fs::write(
        runtime.paths.root.join("computer-control.json"),
        b"not json at all",
    )?;
    let answer = read(&mut runtime, ReadRequest::ControlHistory { cursor: None });
    // The unreadable record keeps its own cause (`StorageFailed`); the exact
    // parser text is not the contract.
    assert!(
        matches!(
            &answer,
            Knowledge::Absent {
                refusal: Refusal::StorageFailed {
                    operation: Operation::Storage,
                    cause: Some(_),
                },
            }
        ),
        "{answer:?}"
    );
    Ok(())
}

#[test]
fn without_reported_permissions_the_catalogue_names_the_cause() -> anyhow::Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _) = fixture(directory.path())?;
    {
        let fixture_value = serde_json::json!(orbit_core::PermissionState::Unknown);
        crate::daemon::authority_fixture::set(
            &mut runtime.state,
            "capabilities.accessibility_state",
            fixture_value,
        );
    }
    {
        let fixture_value = serde_json::json!(orbit_core::PermissionState::Unknown);
        crate::daemon::authority_fixture::set(
            &mut runtime.state,
            "capabilities.screen_state",
            fixture_value,
        );
    }
    assert!(!runtime.engine_is_ready());
    let answer = read(&mut runtime, ReadRequest::EngineTools { cursor: None });
    assert_eq!(
        answer,
        Knowledge::Absent {
            refusal: Refusal::PermissionUnknown {
                permission: orbit_core::refusal::Permission::ScreenAndAccessibility,
            },
        }
    );
    Ok(())
}

#[tokio::test]
async fn oversized_agenda_reply_names_the_ipc_envelope_and_its_exact_bytes() -> anyhow::Result<()> {
    use crate::daemon::calendar::tests::{OCTOBER_MS, ack, event, next_command, page, seed};
    use orbit_core::{
        calendar::CalendarWindow, calendar_control::CalendarCommand, protocol::Command,
    };
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut native) = fixture(directory.path())?;
    seed(&mut runtime, &mut native).await?;
    let end_ms = OCTOBER_MS + 86_400_000;
    let mut event = event(0, OCTOBER_MS, "calendar-0000");
    event["invitees_status"] = json!("provided");
    event["invitees"] = json!((0..1000).map(|index| json!({
        "name":format!("Invitado {index}"), "participant_url":format!("mailto:guest{index}@example.invalid"),
        "role":"required", "response_status":"accepted", "participant_type":"person", "is_current_user":false
    })).collect::<Vec<_>>());
    runtime
        .command(Command::Calendar {
            request: CalendarCommand::Read {
                window: CalendarWindow {
                    start_unix_ms: OCTOBER_MS,
                    end_unix_ms: end_ms,
                },
            },
        })
        .await?;
    let command = next_command(&mut native).await?;
    assert_eq!(command["command"], "calendar_snapshot");
    let response = page(
        &command["calendar_query"],
        vec![event],
        1,
        "44444444-4444-4444-8444-444444444444",
    );
    ack(&mut runtime, &command, response).await?;
    let snapshot = runtime.features.calendar.complete.as_ref().ok_or_else(|| {
        anyhow::anyhow!(
            "valid native event was not admitted: {:?}",
            runtime.features.calendar.read_failure
        )
    })?;
    assert_eq!(snapshot.events.len(), 1);
    let expected = Knowledge::Present {
        value: Page {
            items: vec![serde_json::to_value(&snapshot.events[0])?],
            next: None,
            total: 1,
            revision: 7,
            store_version: None,
        },
        as_of_ms: Some(snapshot.observed_at_unix_ms),
    };
    let bytes = serde_json::to_vec(&expected)?.len();
    assert!(bytes > ReadId::AgendaRange.max_bytes());
    let answer = dispatch(
        &mut runtime,
        ReadRequest::AgendaRange {
            start_ms: OCTOBER_MS,
            end_ms,
            cursor: None,
        },
        7,
    );
    assert_eq!(
        answer,
        Knowledge::Absent {
            refusal: Refusal::TooLarge {
                resource: Resource::IpcResponse,
                bytes: bytes as u64,
                max_bytes: ReadId::AgendaRange.max_bytes() as u64
            }
        }
    );
    Ok(())
}
