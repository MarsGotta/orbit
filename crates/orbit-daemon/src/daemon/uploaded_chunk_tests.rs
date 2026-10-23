use super::*;
use crate::daemon::audio::tests::{fixture, pending_work};
use crate::daemon::capture_lane::test_support;
use base64::{Engine, engine::general_purpose::STANDARD};
use orbit_store::ChunkMetadata;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn chunk(recording: &str) -> ChunkMetadata {
    ChunkMetadata {
        id: format!("{recording}-mic_local-0"),
        recording_id: recording.into(),
        track: "mic_local".into(),
        capture_epoch_id: None,
        sequence: 0,
        sample_rate: 8000,
        channels: 1,
        frames: 1,
        monotonic_ns: 0,
        first_sample: 0,
        started_at_ms: 1,
    }
}

async fn confirm(runtime: &mut Runtime, id: &str) -> Result<()> {
    let (events, mut outcomes) = mpsc::channel(8);
    runtime.work_events = events;
    let generation = runtime.state.generation();
    let ticket = pending_work(&mut runtime.transfers.worker, generation, ())?;
    runtime
        .work_event(WorkResult::Store(crate::daemon::StoreWork::Uploaded {
            ticket,
            job_id: format!("upload-{id}"),
            result: Ok(()),
        }))
        .await?;
    while runtime.transfers.worker.active().is_some() {
        runtime
            .work_event(outcomes.recv().await.context("settlement absent")?)
            .await?;
    }
    Ok(())
}

#[tokio::test]
async fn confirmed_audio_leaves_disk_and_chunks_but_closure_and_resume_keep_their_facts()
-> Result<()> {
    for live in [false, true] {
        let directory = tempfile::tempdir()?;
        let (mut runtime, _) = fixture(directory.path())?;
        let metadata = chunk("confirmed");
        if live {
            runtime.state = decide(
                &runtime.state,
                Event::StartMeeting {
                    id: "confirmed".into(),
                    title: "Prueba".into(),
                    audio: orbit_core::protocol::MeetingAudio::MicrophoneOnly,
                },
                1,
            )?
            .state;
        }
        runtime.fixture_store.seal_chunk(&metadata, b"synthetic")?;
        runtime
            .fixture_store
            .update_job(&format!("upload-{}", metadata.id), JobState::Running)?;
        confirm(&mut runtime, &metadata.id).await?;
        assert!(
            !runtime
                .paths
                .store
                .join(format!("chunks/{}.orbit", metadata.id))
                .exists()
        );
        assert!(runtime.fixture_store.chunk_receipt(&metadata.id)?.is_none());
        assert_eq!(runtime.fixture_store.diagnostics()?.chunk_count, 0);
        assert_eq!(
            runtime
                .reader
                .latest_chunk_receipt("confirmed", "mic_local")?
                .context("resume frontier lost")?
                .metadata,
            metadata
        );
        runtime.queue_capture_finalize("confirmed", 1, 2, 0).await?;
        let closure = runtime
            .reader
            .get_job("finalize-confirmed")?
            .context("closure lost")?;
        assert_eq!(closure.payload["manifest"]["tracks"][0]["chunk_count"], 1);
        assert_eq!(closure.payload["manifest"]["tracks"][0]["total_frames"], 1);
        assert_eq!(runtime.reader.finalize_counts("confirmed")?, (1, 1));
    }
    Ok(())
}

#[tokio::test]
async fn failed_deletion_is_logged_as_cleanup_and_retried_without_failing_the_upload() -> Result<()>
{
    let directory = tempfile::tempdir()?;
    let (mut runtime, _) = fixture(directory.path())?;
    let metadata = chunk("delete-failure");
    runtime.fixture_store.seal_chunk(&metadata, b"synthetic")?;
    let path = runtime
        .paths
        .store
        .join(format!("chunks/{}.orbit", metadata.id));
    let saved = directory.path().join("synthetic-original");
    std::fs::rename(&path, &saved)?;
    std::fs::create_dir(&path)?;
    confirm(&mut runtime, &metadata.id).await?;
    assert_eq!(
        runtime
            .reader
            .get_job(&format!("upload-{}", metadata.id))?
            .context("job lost")?
            .state,
        JobState::Succeeded
    );
    assert!(runtime.reader.chunk_receipt(&metadata.id)?.is_some());
    let log_path = runtime.paths.root.join("logs/daemon.log");
    assert!(log_path.exists(), "failed cleanup produced no diagnostic");
    let log = std::fs::read_to_string(log_path)?;
    assert!(log.contains("uploaded_chunk_delete_failed"), "{log}");
    assert!(!log.contains("upload_failed"), "{log}");
    assert!(log.contains("pending_chunks"), "{log}");
    std::fs::remove_dir(&path)?;
    let bytes = std::fs::metadata(&saved)?.len();
    std::fs::rename(&saved, &path)?;
    let (events, mut outcomes) = mpsc::channel(8);
    runtime.work_events = events;
    runtime.schedule_upload().await?;
    runtime.store_actor.run(|_| Ok(())).await?;
    assert!(!path.exists());
    assert!(runtime.reader.chunk_receipt(&metadata.id)?.is_none());
    let log = std::fs::read_to_string(runtime.paths.root.join("logs/daemon.log"))?;
    let deleted = log
        .lines()
        .map(serde_json::from_str::<Value>)
        .collect::<std::result::Result<Vec<_>, _>>()?
        .into_iter()
        .find(|line| line["event"] == "uploaded_chunk_deleted")
        .context("deletion counts absent")?;
    assert_eq!(deleted["fields"]["chunks"], "1");
    assert_eq!(deleted["fields"]["bytes"], bytes.to_string());
    runtime
        .work_event(outcomes.recv().await.context("cleanup outcome absent")?)
        .await?;
    Ok(())
}

/// The chunks go up from the capture task: it reads the job, sends it and
/// settles it on its own thread.
#[tokio::test]
async fn an_ack_with_wrong_identity_hash_or_durability_keeps_the_audio() -> Result<()> {
    for invalid in ["chunk_id", "sha256", "durable"] {
        let directory = tempfile::tempdir()?;
        let (mut runtime, _) = fixture(directory.path())?;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        runtime.settings.hub_url = Some(format!("http://{}", listener.local_addr()?));
        runtime.link.token = Some("synthetic-not-a-credential".into());
        let metadata = chunk("invalid");
        let receipt = runtime.fixture_store.seal_chunk(&metadata, b"synthetic")?;
        let (mut task, _bridge) = test_support::spawn_task(&runtime)?;
        let (mut stream, _) = listener.accept().await?;
        read_request(&mut stream).await?;
        let mut ack = json!({"durable":true, "chunk_id":metadata.id, "sha256":receipt.sha256});
        ack[invalid] = Value::Null;
        let body = ack.to_string();
        stream
            .write_all(
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .as_bytes(),
            )
            .await?;
        let job = format!("upload-{}", metadata.id);
        test_support::job_reaches(&runtime, &job, JobState::RetryWait).await?;
        task.stop();
        assert_eq!(
            runtime.fixture_store.read_chunk(&metadata.id)?,
            b"synthetic",
            "{invalid}"
        );
    }
    Ok(())
}

/// A request without its `x-orbit-op` line: the op names one attempt.
fn without_op(request: &[u8]) -> Vec<u8> {
    let text = String::from_utf8_lossy(request);
    text.split("\r\n")
        .filter(|line| !line.to_ascii_lowercase().starts_with("x-orbit-op:"))
        .collect::<Vec<_>>()
        .join("\r\n")
        .into_bytes()
}

async fn read_request(stream: &mut tokio::net::TcpStream) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    let mut byte = [0];
    while !bytes.ends_with(b"\r\n\r\n") {
        stream.read_exact(&mut byte).await?;
        bytes.push(byte[0]);
    }
    let headers = String::from_utf8(bytes.clone())?;
    let length: usize = headers
        .lines()
        .find_map(|line| {
            line.to_lowercase()
                .strip_prefix("content-length: ")
                .map(str::to_owned)
        })
        .context("content length missing")?
        .parse()?;
    let header_len = bytes.len();
    bytes.resize(header_len + length, 0);
    stream.read_exact(&mut bytes[header_len..]).await?;
    Ok(bytes)
}

#[tokio::test]
async fn lost_response_preserves_audio_and_retry_sends_the_same_chunk_before_deleting_it()
-> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _) = fixture(directory.path())?;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    runtime.settings.hub_url = Some(format!("http://{}", listener.local_addr()?));
    runtime.link.token = Some("synthetic-not-a-credential".into());
    let metadata = chunk("lost");
    let receipt = runtime.fixture_store.seal_chunk(&metadata, b"synthetic")?;
    let job = format!("upload-{}", metadata.id);
    let (mut task, _bridge) = test_support::spawn_task(&runtime)?;
    let (mut first, _) = listener.accept().await?;
    let original = read_request(&mut first).await?;
    first.shutdown().await?;
    drop(first);
    test_support::job_reaches(&runtime, &job, JobState::RetryWait).await?;
    assert_eq!(
        runtime.fixture_store.read_chunk(&metadata.id)?,
        b"synthetic"
    );
    // The retry, once the gate's backoff has passed, sends the same chunk:
    // the same body and headers, under the op of its own attempt.
    let (mut second, _) = tokio::time::timeout(test_support::TASK_JAM, listener.accept()).await??;
    assert_eq!(
        without_op(&read_request(&mut second).await?),
        without_op(&original)
    );
    let body = json!({"durable":true,"chunk_id":metadata.id,"sha256":receipt.sha256}).to_string();
    second
        .write_all(
            format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .as_bytes(),
        )
        .await?;
    test_support::job_reaches(&runtime, &job, JobState::Succeeded).await?;
    // A storage barrier: the pruning is part of the same writer operation.
    runtime.store_actor.run(|_| Ok(())).await?;
    task.stop();
    assert!(
        !runtime
            .paths
            .store
            .join(format!("chunks/{}.orbit", metadata.id))
            .exists()
    );
    assert!(runtime.reader.chunk_receipt(&metadata.id)?.is_none());
    Ok(())
}

#[tokio::test]
async fn a_reopened_store_prunes_old_confirmations_without_uploading_or_removing_private_audio()
-> Result<()> {
    let directory = tempfile::tempdir()?;
    let (runtime, _) = fixture(directory.path())?;
    let paths = runtime.paths.clone();
    drop(runtime);
    let mut store = Store::open_existing(&paths.store, [0x42; 32])?;
    for (name, state) in [
        ("old-a", JobState::Succeeded),
        ("old-b", JobState::Succeeded),
        ("pending", JobState::Queued),
        ("uncertain", JobState::Unknown),
        ("private", JobState::Cancelled),
        ("voice", JobState::Cancelled),
    ] {
        let metadata = chunk(name);
        store.seal_chunk(&metadata, b"synthetic")?;
        store.update_job(&format!("upload-{}", metadata.id), state)?;
    }
    drop(store);
    let (mut runtime, _) = fixture(directory.path())?;
    let (events, mut outcomes) = mpsc::channel(8);
    runtime.work_events = events;
    crate::daemon::authority_fixture::set(
        &mut runtime.state,
        "connection",
        json!(Connection::Offline),
    );
    runtime.run_maintenance().await;
    // A storage barrier waits for the admitted cleanup, without a clock race.
    runtime.store_actor.run(|_| Ok(())).await?;
    assert!(runtime.reader.chunk_receipt(&chunk("old-a").id)?.is_none());
    assert!(runtime.reader.chunk_receipt(&chunk("old-b").id)?.is_some());
    assert!(
        !paths
            .store
            .join(format!("chunks/{}.orbit", chunk("old-a").id))
            .exists()
    );
    for name in ["pending", "uncertain", "private", "voice"] {
        assert_eq!(
            runtime.fixture_store.read_chunk(&chunk(name).id)?,
            b"synthetic",
            "{name}"
        );
    }
    assert_eq!(
        runtime.voice_note_audio("voice")?.bytes_base64,
        STANDARD.encode(b"synthetic")
    );
    assert_eq!(
        runtime
            .reader
            .get_job(&format!("upload-{}", chunk("uncertain").id))?
            .context("uncertain job lost")?
            .state,
        JobState::Unknown
    );
    runtime
        .work_event(
            outcomes
                .recv()
                .await
                .context("first cleanup outcome absent")?,
        )
        .await?;
    runtime.run_maintenance().await;
    runtime.store_actor.run(|_| Ok(())).await?;
    assert!(runtime.reader.chunk_receipt(&chunk("old-b").id)?.is_none());
    runtime
        .work_event(
            outcomes
                .recv()
                .await
                .context("second cleanup outcome absent")?,
        )
        .await?;
    Ok(())
}
