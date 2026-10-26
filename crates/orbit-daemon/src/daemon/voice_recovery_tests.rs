//! Persisted-state loading uses invented notes and encrypted synthetic PCM.
use super::*;

#[tokio::test]
async fn loading_saved_state_removes_old_noise_audio_and_preserves_other_notes() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut helper) = crate::daemon::audio::tests::fixture(directory.path())?;
    runtime.link.token = Some("synthetic-token".into());
    crate::daemon::authority_fixture::set(
        &mut runtime.state,
        "connection",
        json!(Connection::Offline),
    );
    let mut ids = Vec::new();
    for _ in 0..4 {
        crate::daemon::offline_voice_note_tests::record(
            &mut runtime,
            &mut helper,
            8_000,
            Command::VoiceEnd,
        )
        .await?;
        ids.push(
            runtime
                .state
                .durable
                .assistant
                .turn
                .as_ref()
                .context("note")?
                .user
                .id
                .clone(),
        );
    }
    let mut saved = serde_json::to_value(&runtime.state.durable)?;
    saved["pending_voice"][0]["voice"]["failure"]["code"] =
        json!("voice_transcription_joined_next_turn");
    saved["pending_voice"][1]["voice"]["failure"]["code"] = json!("transcription_provider_refused");
    saved["pending_voice"][2]["voice"]["failure"]["code"] = json!("voice_transcription_empty");
    runtime.fixture_store.save_snapshot(&saved)?;
    let loaded = load_saved(&mut runtime.fixture_store, &runtime.settings)?;
    for id in &ids[..3] {
        assert!(
            runtime.fixture_store.list_chunks(id, None, 2)?.is_empty(),
            "old noise audio survived load"
        );
        assert!(
            runtime
                .fixture_store
                .get_job(&format!("recording-{id}"))?
                .is_none()
        );
    }
    assert!(
        !runtime
            .fixture_store
            .list_chunks(&ids[3], None, 2)?
            .is_empty()
    );
    assert!(loaded.state.durable.assistant.pending_voice.is_empty());
    assert_eq!(
        loaded
            .state
            .durable
            .assistant
            .turn
            .as_ref()
            .context("waiting note")?
            .user
            .id,
        ids[3]
    );
    let written: orbit_core::DurableState =
        runtime.fixture_store.load_snapshot()?.context("snapshot")?;
    assert!(
        written.assistant.pending_voice.is_empty(),
        "cleanup must be durable before startup continues"
    );
    Ok(())
}
