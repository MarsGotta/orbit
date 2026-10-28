use orbit_app_core::{
    AppSnapshot, PresentationCopy, PresentationText,
    features::{FeatureInput, FrameRevision},
    topics::TopicCache,
};

fn render(text: &PresentationText) -> String {
    match text {
        PresentationText::UpdateBlocked(reason) => format!("Translated: {reason:?}"),
        _ => "Translated".into(),
    }
}

#[test]
fn presentation_copy_reaches_bootstrap_projection_without_changing_update_safety()
-> Result<(), Box<dyn std::error::Error>> {
    let snapshot = AppSnapshot::default();
    let topics = TopicCache::default();
    let input = FeatureInput::new(&snapshot, &topics).with_copy(PresentationCopy { render });
    let frames = orbit_app_core::features::project_all(
        &input,
        FrameRevision {
            epoch: 1,
            counter: 1,
        },
    );
    let local = frames
        .iter()
        .find_map(|frame| match frame {
            orbit_app_core::features::FeatureFrame::Local { view, .. } => Some(view),
            _ => None,
        })
        .ok_or("local frame missing")?;
    assert!(
        local
            .update
            .install_blocked
            .as_ref()
            .is_some_and(|text| text.starts_with("Translated:"))
    );
    Ok(())
}

#[path = "support/fake_daemon.rs"]
mod fake_daemon;

#[tokio::test]
async fn changing_copy_republishes_frames() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let paths = orbit_ipc::Paths::resolve(Some(directory.path().into()))?;
    let client = orbit_app_core::DaemonClient::connect(paths, fake_daemon::idle());
    let mut changes = client.watch_frames();
    client.set_presentation_copy(PresentationCopy { render });
    assert!(changes.has_changed()?);
    changes.borrow_and_update();
    let first = client.take_frames();
    assert_eq!(first.len(), client.bootstrap().frames.len());
    let first_revision = first[0].revision();
    client.set_presentation_copy(PresentationCopy {
        render: |_| "Second language".into(),
    });
    assert!(changes.has_changed()?);
    let second = client.take_frames();
    assert!(
        second
            .iter()
            .all(|frame| frame.revision().counter > first_revision.counter)
    );
    assert!(second.iter().any(|frame| matches!(frame,
        orbit_app_core::features::FeatureFrame::Local { view, .. }
        if view.update.install_blocked.as_deref() == Some("Second language"))));
    Ok(())
}

#[test]
fn app_errors_serialize_the_operation_code_for_read_and_command_callers()
-> Result<(), Box<dyn std::error::Error>> {
    let error = orbit_app_core::AppError::note_unavailable();
    let encoded = serde_json::to_value(error)?;
    assert_eq!(encoded["refusal"]["cause"]["code"], "note_unavailable");
    assert_eq!(encoded["refusal"]["cause"]["kind"], "app_operation");
    Ok(())
}

#[test]
fn retained_read_facts_reword_on_projection_without_rewriting_the_source()
-> Result<(), Box<dyn std::error::Error>> {
    let encoded = orbit_core::refusal::Refusal::Internal {
        cause: orbit_core::refusal::InternalCause::AppOperation {
            code: "read_turn_limit".into(),
            error: serde_json::json!({"max": 64, "count": 211}).to_string(),
        },
    }
    .to_string();
    let mut snapshot = AppSnapshot::default();
    snapshot.notes.as_mut().ok_or("notes missing")?.open_read =
        Some(orbit_app_core::NoteReadSnapshot {
            recording_id: "recording-1".into(),
            revision: 1,
            state: orbit_app_core::NoteReadState::Failed,
            detail: Some(encoded.clone()),
            read_at_ms: None,
        });
    let topics = TopicCache::default();
    let copies = [
        PresentationCopy {
            render: |text| match text {
                PresentationText::ReadFailure { facts, .. } => {
                    format!("First: {} pages, {} turns", facts["max"], facts["count"])
                }
                other => format!("{other:?}"),
            },
        },
        PresentationCopy {
            render: |text| match text {
                PresentationText::ReadFailure { facts, .. } => {
                    format!("Second: {} pages, {} turns", facts["max"], facts["count"])
                }
                other => format!("{other:?}"),
            },
        },
    ];
    for (copy, prefix) in copies.into_iter().zip(["First:", "Second:"]) {
        let view = orbit_app_core::features::notes::project(
            &FeatureInput::new(&snapshot, &topics).with_copy(copy),
        )
        .ok_or("notes view missing")?;
        let detail = view
            .open_read
            .and_then(|read| read.detail)
            .ok_or("read detail missing")?;
        assert!(detail.starts_with(prefix));
        assert!(detail.contains("64"));
        assert!(detail.contains("211"));
    }
    assert_eq!(
        snapshot
            .notes
            .and_then(|notes| notes.open_read)
            .and_then(|read| read.detail),
        Some(encoded)
    );
    Ok(())
}
