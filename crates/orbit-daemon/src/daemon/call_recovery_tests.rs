//! Recovery owns only durable originals, never microphone or prompt authority.
use super::*;
use orbit_core::wire::sha256;
use orbit_store::ChunkMetadata;

async fn reopen(directory: &std::path::Path) -> Result<(Runtime, mpsc::Receiver<Value>)> {
    let (mut runtime, receiver) = fixture(directory)?;
    runtime.state = runtime
        .fixture_store
        .load_snapshot::<State>()?
        .context("snapshot absent")?
        .recovered();
    crate::daemon::recover_store(&mut runtime.fixture_store).await?;
    runtime.fixture_store.recover_upload_jobs()?;
    runtime.fixture_store.save_snapshot(&runtime.state)?;
    runtime.recover_call_closures(crate::daemon::now()).await?;
    runtime.connect();
    Ok((runtime, receiver))
}

#[tokio::test]
async fn restart_closes_the_original_without_recapture_or_prompt_replay() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    let stream = start_call(&mut runtime, &mut receiver).await?;
    for sequence in 0..10 {
        runtime
            .native_event(call_frame(&stream, sequence, false))
            .await?;
    }
    settle_seal_receipts(&mut runtime).await?;
    let receipt = runtime
        .fixture_store
        .list_chunks("synthetic-call", None, 10)?
        .remove(0);
    let original = runtime.fixture_store.read_chunk(&receipt.metadata.id)?;
    assert_eq!(receipt.metadata.frames, 8000);
    drop(runtime);

    let (mut recovered, mut native) = reopen(directory.path()).await?;
    // The restart invents no stop; nothing recaptures either.
    assert!(!recovered.state.privacy_stopped());
    assert!(recovered.state.durable.call.is_none() && recovered.capture.call_runtime.is_none());
    assert!(recovered.capture.mic_stream.is_none() && recovered.capture.buffer.voice.is_none());
    // Review (c): the notice says what is true after a restart nobody stopped.
    assert!(
        crate::daemon::turn::said::has(
            &recovered.state,
            orbit_core::notice::NoticeCode::CallsRecovered
        ),
        "{:?}",
        recovered.state.durable.notice_log
    );
    assert!(
        !crate::daemon::turn::said::has(
            &recovered.state,
            orbit_core::notice::NoticeCode::PrivacyStopKept
        ),
        "{:?}",
        recovered.state.durable.notice_log
    );
    assert!(
        recovered.state.durable.active_request.is_none()
            && recovered.conversation.voice_task.active().is_none()
    );
    assert!(
        native.try_recv().is_err(),
        "recovery must not command a native producer"
    );
    let closure = recovered
        .fixture_store
        .get_job("finalize-synthetic-call")?
        .context("restart lost the original's durable closure")?;
    assert_eq!(
        closure.payload["manifest"]["tracks"][0]["total_frames"],
        8000
    );
    assert_eq!(closure.payload["manifest"]["gaps"], 1);
    assert_eq!(
        recovered.fixture_store.read_chunk(&receipt.metadata.id)?,
        original
    );

    recovered.apply(Event::CallStop).await?;
    recovered.run_maintenance().await;
    assert!(
        recovered.work.worker.active().is_none(),
        "privacy must still gate uploads"
    );
    recovered.apply(Event::LocalRearm).await?;
    recovered.apply(Event::CallStop).await?;
    assert!(native.try_recv().is_err());
    assert_eq!(
        recovered
            .fixture_store
            .get_job("finalize-synthetic-call")?
            .context("closure absent")?
            .payload,
        closure.payload
    );
    // The recovered closure follows the ordinary upload -> finalize path
    // after explicit rearm, with synthetic receipts and no provider call.
    recovered.fixture_store.update_job(
        &format!("upload-{}", receipt.metadata.id),
        JobState::Succeeded,
    )?;
    recovered.settings.hub_url = Some("http://127.0.0.1:9".into());
    recovered.token = Some("synthetic-not-a-credential".into());
    assert!(recovered.schedule_finalize().await?);
    let ticket = recovered
        .work
        .worker
        .ticket()
        .cloned()
        .context("finalize ticket")?;
    recovered
        .work
        .worker
        .task_handle()
        .context("finalize not eligible")?
        .abort();
    let before = recovered.features.notes.head.changed_at("synthetic-call");
    recovered
        .work_event(crate::daemon::WorkResult::Store(
            crate::daemon::StoreWork::Uploaded {
                ticket,
                job_id: "finalize-synthetic-call".into(),
                result: Ok(()),
            },
        ))
        .await?;
    assert_eq!(
        recovered
            .fixture_store
            .get_job("finalize-synthetic-call")?
            .context("closure absent")?
            .state,
        JobState::Succeeded
    );
    assert!(
        recovered.features.notes.head.changed_at("synthetic-call") > before,
        "the uploaded closure did not tell the app to read its note again"
    );
    assert!(
        recovered
            .fixture_store
            .list_jobs(100)?
            .iter()
            .all(|job| job.kind != "chat_send" && job.payload["recording_kind"] != "voice_turn")
    );
    Ok(())
}

#[tokio::test]
async fn call_stop_snapshot_cannot_orphan_original_before_drain_after_restart() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    let stream = start_call(&mut runtime, &mut receiver).await?;
    for sequence in 0..10 {
        runtime
            .native_event(call_frame(&stream, sequence, false))
            .await?;
    }
    runtime.apply(Event::CallStop).await?;
    assert!(
        runtime
            .fixture_store
            .load_snapshot::<State>()?
            .context("snapshot absent")?
            .durable
            .call
            .is_none()
    );
    assert!(runtime.audio_is_draining());
    assert!(
        runtime
            .fixture_store
            .get_job("finalize-synthetic-call")?
            .is_none()
    );
    drop(runtime); // No native drain ACK; only the already durable chunk survives.
    let (runtime, mut native) = reopen(directory.path()).await?;
    assert!(!runtime.state.privacy_stopped() && runtime.capture.call_runtime.is_none());
    assert!(
        runtime
            .fixture_store
            .get_job("finalize-synthetic-call")?
            .is_some()
    );
    assert!(native.try_recv().is_err());
    Ok(())
}

fn manifest(runtime: &mut Runtime, identity: &str) -> Result<()> {
    runtime.fixture_store.enqueue_job(&Job {
        id: format!("recording-{identity}"),
        kind: "recording_manifest".into(),
        state: JobState::Succeeded,
        created_at: 10,
        payload: json!({"recording_id":identity,"recording_kind":"voice_call",
            "started_at_ms":10,"expires_at_ms":60010}),
    })?;
    Ok(())
}

#[tokio::test]
async fn recovery_pages_to_eof_and_preserves_all_existing_closure_states() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut native) = fixture(directory.path())?;
    runtime.state = runtime.state.recovered();
    // More than two pages; do not infer EOF from a limit or a short byte page.
    for index in 0..205 {
        manifest(&mut runtime, &format!("call-{index:03}"))?;
    }
    let mut unchanged = Vec::new();
    for (index, state) in [
        JobState::Running,
        JobState::Unknown,
        JobState::Blocked,
        JobState::Succeeded,
    ]
    .into_iter()
    .enumerate()
    {
        let identity = format!("call-{index:03}");
        runtime.queue_capture_finalize(&identity, 10, 20, 0).await?;
        let id = format!("finalize-{identity}");
        runtime.fixture_store.update_job(&id, state)?;
        unchanged.push(
            runtime
                .fixture_store
                .get_job(&id)?
                .context("closure absent")?,
        );
    }
    assert_eq!(runtime.recover_call_closures(30).await?, 201);
    assert!(
        runtime
            .fixture_store
            .get_job("finalize-call-204")?
            .is_some()
    );
    assert_eq!(runtime.recover_call_closures(40).await?, 0);
    for prior in unchanged {
        let after = runtime
            .fixture_store
            .get_job(&prior.id)?
            .context("closure absent")?;
        assert_eq!(after.payload, prior.payload);
        assert_eq!(after.state, prior.state);
        assert!(
            runtime
                .fixture_store
                .get_job(&prior.id.replace("finalize-", "call-recovery-"))?
                .is_none()
        );
    }
    assert!(native.try_recv().is_err());
    Ok(())
}

#[tokio::test]
async fn failed_closure_commit_reuses_the_original_recovery_boundary_after_restart() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _) = fixture(directory.path())?;
    runtime.state = runtime.state.recovered();
    runtime.fixture_store.save_snapshot(&runtime.state)?;
    manifest(&mut runtime, "failed-call")?;
    let fault = rusqlite::Connection::open(runtime.paths.store.join("orbit.sqlite3"))?;
    fault.execute_batch("CREATE TRIGGER fail_recovery_close BEFORE INSERT ON jobs WHEN NEW.kind = 'recording_finalize' BEGIN SELECT RAISE(FAIL, 'synthetic closure write failure'); END;")?;
    assert!(runtime.recover_call_closures(30).await.is_err());
    assert!(
        runtime
            .fixture_store
            .get_job("finalize-failed-call")?
            .is_none()
    );
    let boundary = runtime
        .fixture_store
        .get_job("call-recovery-failed-call")?
        .context("boundary absent")?;
    drop(runtime);
    fault.execute_batch("DROP TRIGGER fail_recovery_close;")?;
    let (mut recovered, mut native) = reopen(directory.path()).await?;
    let closure = recovered
        .fixture_store
        .get_job("finalize-failed-call")?
        .context("closure absent")?;
    assert_eq!(closure.payload["manifest"]["finalized_at_ms"], 30);
    assert_eq!(closure.payload["manifest"]["gaps"], 1);
    assert_eq!(
        recovered
            .fixture_store
            .get_job(&boundary.id)?
            .context("boundary absent")?
            .payload,
        boundary.payload
    );
    assert_eq!(recovered.recover_call_closures(90).await?, 0);
    assert!(native.try_recv().is_err());
    Ok(())
}

#[tokio::test]
async fn recovery_rejects_live_capture_and_conflicting_authenticated_lifecycle() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut native) = fixture(directory.path())?;
    assert_eq!(runtime.recover_call_closures(30).await?, 0, "fresh install");
    manifest(&mut runtime, "conflict")?;
    assert!(
        runtime.recover_call_closures(30).await.is_err(),
        "a helper is already connected: recovery belongs to startup, before it"
    );
    runtime.state = runtime.state.recovered();
    runtime.capture.mic_stream = Some("not-authority".into());
    assert!(runtime.recover_call_closures(30).await.is_err());
    runtime.capture.mic_stream = None;
    runtime
        .capture
        .pending_starts
        .push("uncertain-start".into());
    assert!(runtime.recover_call_closures(30).await.is_err());
    runtime.capture.pending_starts.clear();
    runtime
        .capture
        .buffer
        .begin_recording("live-buffer".into())?;
    runtime
        .capture
        .buffer
        .ingest(&frame("old-stream", 0, 100), 10)?;
    assert!(runtime.capture.buffer.has_pending_audio());
    assert!(runtime.recover_call_closures(30).await.is_err());
    runtime.capture.buffer = CaptureBuffer::default();
    runtime.fixture_store.enqueue_job(&Job {
        id: "finalize-conflict".into(),
        kind: "recording_finalize".into(),
        state: JobState::Running,
        created_at: 30,
        payload: json!({"recording_id":"different"}),
    })?;
    assert!(runtime.recover_call_closures(30).await.is_err());
    assert!(
        runtime
            .fixture_store
            .get_job("call-recovery-conflict")?
            .is_none()
    );
    assert!(native.try_recv().is_err());
    Ok(())
}

#[tokio::test]
async fn manifest_before_first_native_ack_recovers_an_explicitly_empty_original() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    request_call(&mut runtime, &mut receiver).await?;
    assert!(
        runtime
            .fixture_store
            .list_chunks("synthetic-call", None, 10)?
            .is_empty()
    );
    drop(runtime);
    let (runtime, mut native) = reopen(directory.path()).await?;
    let closure = runtime
        .fixture_store
        .get_job("finalize-synthetic-call")?
        .context("closure absent")?;
    assert_eq!(closure.payload["manifest"]["tracks"], json!([]));
    assert_eq!(closure.payload["manifest"]["gaps"], 1);
    assert!(
        runtime
            .fixture_store
            .get_job("call-processing-synthetic-call")?
            .is_none()
    );
    assert!(runtime.state.durable.call.is_none() && native.try_recv().is_err());
    Ok(())
}

#[tokio::test]
async fn failed_recovery_snapshot_cannot_restore_capture_or_rewrite_a_sealed_closure() -> Result<()>
{
    let directory = tempfile::tempdir()?;
    let (mut runtime, _) = fixture(directory.path())?;
    runtime.state = runtime.state.recovered();
    runtime.fixture_store.save_snapshot(&runtime.state)?;
    manifest(&mut runtime, "snapshot-fault")?;
    let fault = rusqlite::Connection::open(runtime.paths.store.join("orbit.sqlite3"))?;
    fault.execute_batch("CREATE TRIGGER fail_recovery_snapshot BEFORE UPDATE ON settings BEGIN SELECT RAISE(FAIL, 'synthetic recovery snapshot failure'); END;")?;
    assert!(runtime.recover_call_closures(30).await.is_err());
    let closure = runtime
        .fixture_store
        .get_job("finalize-snapshot-fault")?
        .context("closure absent")?;
    assert!(!runtime.state.privacy_stopped() && runtime.state.durable.call.is_none());
    drop(runtime);
    fault.execute_batch("DROP TRIGGER fail_recovery_snapshot;")?;
    let (runtime, mut native) = reopen(directory.path()).await?;
    assert_eq!(
        runtime
            .fixture_store
            .get_job(&closure.id)?
            .context("closure absent")?
            .payload,
        closure.payload
    );
    assert!(!runtime.state.privacy_stopped() && runtime.state.durable.call.is_none());
    assert!(native.try_recv().is_err());
    Ok(())
}

// The shared jobs table also holds padded domain rows. A store from an older
// Orbit still holds those of the retired source-reading and voice-memory
// verticals; nothing reads them now, and recovery must step over
// them. Every byte is synthetic.

const DOMAIN_DEVICE: &str = "11111111-1111-4111-8111-111111111111";
const DOMAIN_RECORDING: &str = "33333333-3333-4333-8333-333333333333";

/// Rows of the retired verticals' kinds as an older store holds them: padded
/// past the queue's payload limit, which only a domain row may be.
fn seed_retired_domain_rows(runtime: &Runtime) -> Result<()> {
    let database = rusqlite::Connection::open(runtime.paths.store.join("orbit.sqlite3"))?;
    for (id, kind) in [
        ("source-reading-registry", "source_reading_head"),
        ("voice-artifact-retired", "voice_memory_artifact"),
    ] {
        database.execute(
            "INSERT INTO jobs(id,kind,state,created_at,payload) \
             VALUES(?1,?2,'succeeded',1,zeroblob(2097152))",
            rusqlite::params![id, kind],
        )?;
    }
    Ok(())
}

fn seed_meeting(runtime: &mut Runtime) -> Result<()> {
    let mut segments = Vec::new();
    for index in 0..20 {
        let chunk_id = format!("domain-chunk-{index:03}");
        let chunk = runtime.fixture_store.seal_chunk(
            &ChunkMetadata {
                id: chunk_id.clone(),
                recording_id: DOMAIN_RECORDING.into(),
                track: "mic_local".into(),
                capture_epoch_id: None,
                sequence: index as u64,
                sample_rate: 16000,
                channels: 1,
                frames: 1,
                monotonic_ns: index as u64,
                first_sample: index as u64,
                started_at_ms: 1000 + index as i64,
            },
            chunk_id.as_bytes(),
        )?;
        let sources =
            json!([{"chunk_id":chunk_id,"sequence":index,"sha256":chunk.sha256,"frames":1}]);
        let group_id = format!(
            "group_{}",
            sha256(&serde_json::to_vec(&json!({"device_id":DOMAIN_DEVICE,
                "recording_id":DOMAIN_RECORDING,"track":"mic_local","sources":sources,
                "pipeline_version":1}))?)
        );
        let text = format!("g{index:03}: {}\n", "x".repeat(8 * 1024 - 8));
        segments.push(
            json!({"group_id":group_id,"track":"mic_local","sequence":index,
            "last_sequence":index,"source_chunk_ids":[chunk_id],"sources":sources,
            "timing_precision":"chunk","speaker":"unknown","pipeline_version":1,"frames":1,
            "flush_reason":"legacy","state":"completed","error_code":null,"text":text,
            "chunk_id":chunk_id,"source_sha256":chunk.sha256}),
        );
    }
    let manifest = json!({"tracks":[{"track":"mic_local","chunk_count":20,"last_sequence":19,
        "total_frames":20}],"gaps":0,"started_at_ms":1000,"finalized_at_ms":1900});
    for job in [
        Job {
            id: format!("finalize-{DOMAIN_RECORDING}"),
            kind: "recording_finalize".into(),
            state: JobState::Succeeded,
            created_at: 1900,
            payload: json!({"recording_id":DOMAIN_RECORDING,"manifest":manifest}),
        },
        Job {
            id: format!("transcript-{DOMAIN_RECORDING}"),
            kind: "transcript".into(),
            state: JobState::Succeeded,
            created_at: 1901,
            payload: json!({"recording_id":DOMAIN_RECORDING,"publication":"local_draft",
                "pending_chunks":0,"capture_finalized":true,"capture_complete":false,
                "transcript_ready":true,"manifest":manifest,"missing_tracks":["remote_app"],
                "segments":segments}),
        },
    ] {
        runtime.fixture_store.enqueue_job(&job)?;
    }
    Ok(())
}

/// One consented person with one enrolled voice, leaving the padded voice
/// artifact row in jobs.
#[tokio::test]
async fn startup_recovery_ends_ok_with_domain_rows_present() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _native) = fixture(directory.path())?;
    seed_retired_domain_rows(&runtime)?;
    seed_meeting(&mut runtime)?;
    runtime.state = runtime.state.recovered();
    assert_eq!(runtime.recover_call_closures(30).await?, 0);
    Ok(())
}
