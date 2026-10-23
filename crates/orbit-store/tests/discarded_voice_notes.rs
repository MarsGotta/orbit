//! «Descartar» takes a voice note's audio off this Mac, and only a voice
//! note's: a meeting's chunks are never deleted this way.
use orbit_store::{ChunkMetadata, Error, Job, JobState, Store};
use serde_json::json;
use std::error::Error as StdError;

type TestResult = Result<(), Box<dyn StdError>>;
const KEY: [u8; 32] = [0x24; 32];

fn chunk(recording_id: &str) -> ChunkMetadata {
    ChunkMetadata {
        id: format!("{recording_id}-mic_local-0"),
        recording_id: recording_id.into(),
        track: "mic_local".into(),
        capture_epoch_id: None,
        sequence: 0,
        sample_rate: 16_000,
        channels: 1,
        frames: 16_000,
        monotonic_ns: 0,
        first_sample: 0,
        started_at_ms: 1,
    }
}

fn manifest(store: &mut Store, recording_id: &str, kind: &str) -> TestResult {
    store.enqueue_job(&Job {
        id: format!("recording-{recording_id}"),
        kind: "recording_manifest".into(),
        payload: json!({"recording_id": recording_id, "recording_kind": kind,
            "upload_allowed": kind != "voice_turn"}),
        state: JobState::Succeeded,
        created_at: 1,
    })?;
    Ok(())
}

#[test]
fn a_discarded_voice_note_leaves_no_audio_and_no_job() -> TestResult {
    let directory = tempfile::tempdir()?;
    let mut store = Store::open(directory.path(), KEY)?;
    manifest(&mut store, "nota-sintetica", "voice_turn")?;
    store.seal_chunk(&chunk("nota-sintetica"), b"synthetic wav")?;
    store.update_job("upload-nota-sintetica-mic_local-0", JobState::Cancelled)?;

    assert_eq!(store.discard_voice_note("nota-sintetica")?, 1);
    assert!(store.list_chunks("nota-sintetica", None, 10)?.is_empty());
    assert!(
        !directory
            .path()
            .join("chunks/nota-sintetica-mic_local-0.orbit")
            .exists()
    );
    assert!(store.get_job("recording-nota-sintetica")?.is_none());
    assert!(
        store
            .get_job("upload-nota-sintetica-mic_local-0")?
            .is_none()
    );
    // Nothing is left for recovery to bring back.
    drop(store);
    let mut store = Store::open(directory.path(), KEY)?;
    let mut scan = store.begin_recovery()?;
    let report = store.recover_next(&mut scan, 10)?;
    assert_eq!((report.recovered, report.missing), (0, 0));
    assert!(store.list_chunks("nota-sintetica", None, 10)?.is_empty());
    // Discarding again finds nothing, which is not an error.
    assert_eq!(store.discard_voice_note("nota-sintetica")?, 0);
    Ok(())
}

#[test]
fn a_meeting_is_never_discarded_as_a_voice_note() -> TestResult {
    let directory = tempfile::tempdir()?;
    let mut store = Store::open(directory.path(), KEY)?;
    manifest(&mut store, "reunion-sintetica", "meeting")?;
    store.seal_chunk(&chunk("reunion-sintetica"), b"synthetic meeting")?;
    assert!(matches!(
        store.discard_voice_note("reunion-sintetica"),
        Err(Error::Invalid(_))
    ));
    assert_eq!(store.list_chunks("reunion-sintetica", None, 10)?.len(), 1);
    assert!(store.read_chunk("reunion-sintetica-mic_local-0").is_ok());
    Ok(())
}
