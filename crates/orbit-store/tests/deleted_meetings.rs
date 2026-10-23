//! «Borrar reunión» and «Terminar sin guardar» on this Mac: what it keeps of
//! one meeting leaves it, by the meeting's identity and nothing else. Another
//! meeting, a voice note and their audio stay.
use orbit_core::note_outbox::{NoteOrder, QueuedOrder};
use orbit_store::{ChunkMetadata, DeletionScope, Error, Job, JobState, PendingPrivateNotes, Store};
use serde_json::json;
use std::error::Error as StdError;

type TestResult = Result<(), Box<dyn StdError>>;
const KEY: [u8; 32] = [0x31; 32];
const MEETING: &str = "4f1c7a2e-9b3d-4e8f-a6c5-2d1e0f9a8b7c";
const OTHER: &str = "8e2d6b1f-3c4a-4d5e-9f6a-7b8c9d0e1f2a";
const VOICE: &str = "1a2b3c4d-5e6f-4a7b-8c9d-0e1f2a3b4c5d";

fn chunk(recording_id: &str, track: &str, sequence: u64) -> ChunkMetadata {
    ChunkMetadata {
        id: format!("{recording_id}-{track}-{sequence}"),
        recording_id: recording_id.into(),
        track: track.into(),
        capture_epoch_id: Some(format!("epoch-{recording_id}")),
        sequence,
        sample_rate: 16_000,
        channels: 1,
        frames: 16_000,
        monotonic_ns: sequence,
        first_sample: sequence * 16_000,
        started_at_ms: 1,
    }
}

fn job(id: String, kind: &str, payload: serde_json::Value) -> Job {
    Job {
        id,
        kind: kind.into(),
        state: JobState::Succeeded,
        created_at: 1,
        payload,
    }
}

/// A meeting with what this Mac keeps of one: three chunks of audio (one the
/// hub confirmed, whose file already left), its manifest, capture epoch and
/// closure, an order and its private notes waiting for the hub.
fn recorded(store: &mut Store, recording_id: &str, kind: &str) -> TestResult {
    store.enqueue_job(&job(
        format!("recording-{recording_id}"),
        "recording_manifest",
        json!({"recording_id": recording_id, "title": "Revisión de Lumen",
            "recording_kind": kind, "upload_allowed": kind != "voice_turn"}),
    ))?;
    store.enqueue_job(&job(
        format!("capture-epoch-epoch-{recording_id}"),
        "capture_epoch",
        json!({"recording_id": recording_id, "capture_epoch_id": format!("epoch-{recording_id}")}),
    ))?;
    for sequence in 0..3 {
        store.seal_chunk(
            &chunk(recording_id, "mic_local", sequence),
            b"audio sintetico",
        )?;
    }
    if kind == "meeting" {
        let confirmed = format!("{recording_id}-mic_local-0");
        store.update_job(&format!("upload-{confirmed}"), JobState::Succeeded)?;
        assert!(store.prune_uploaded_chunk(&confirmed)?.is_some());
        store.enqueue_job(&Job {
            state: JobState::Queued,
            ..job(
                format!("finalize-{recording_id}"),
                "recording_finalize",
                json!({"recording_id": recording_id, "manifest": {"tracks": []}}),
            )
        })?;
        store.queue_note_order(
            "title",
            QueuedOrder::Meeting {
                recording_id: recording_id.into(),
                order: NoteOrder::Title {
                    title: "Lumen".into(),
                },
            },
            1,
        )?;
        store.save_private_notes(
            &PendingPrivateNotes {
                recording_id: recording_id.into(),
                markdown: "- Ojo con el plazo".into(),
                base_revision: 0,
            },
            1,
        )?;
    }
    Ok(())
}

fn jobs_of(store: &Store, recording_id: &str) -> Result<usize, Error> {
    Ok(store
        .list_jobs(500)?
        .into_iter()
        .filter(|job| {
            job.payload["recording_id"] == recording_id
                || job.payload["metadata"]["recording_id"] == recording_id
        })
        .count())
}

fn files(directory: &std::path::Path, recording_id: &str) -> Result<usize, std::io::Error> {
    Ok(std::fs::read_dir(directory.join("chunks"))?
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with(recording_id)
        })
        .count())
}

#[test]
fn a_deleted_meeting_leaves_nothing_on_this_mac_and_the_others_stay() -> TestResult {
    let directory = tempfile::tempdir()?;
    let mut store = Store::open(directory.path(), KEY)?;
    recorded(&mut store, MEETING, "meeting")?;
    recorded(&mut store, OTHER, "meeting")?;
    recorded(&mut store, VOICE, "voice_turn")?;
    let other_before = (jobs_of(&store, OTHER)?, files(directory.path(), OTHER)?);

    // One chunk a step: two steps take the two left, a third the rest.
    let first = store.delete_recording(MEETING, DeletionScope::Everything, 1)?;
    assert_eq!((first.chunks, first.done), (1, false));
    let mut steps = vec![first];
    while !steps.last().is_some_and(|step| step.done) {
        steps.push(store.delete_recording(MEETING, DeletionScope::Everything, 1)?);
    }
    assert_eq!(steps.iter().map(|step| step.chunks).sum::<u64>(), 2);

    assert!(store.list_chunks(MEETING, None, 10)?.is_empty());
    assert_eq!(files(directory.path(), MEETING)?, 0);
    assert_eq!(jobs_of(&store, MEETING)?, 0, "{:#?}", store.list_jobs(500)?);
    assert!(store.private_notes(MEETING)?.is_none());
    assert!(
        store
            .note_orders()?
            .iter()
            .all(|waiting| waiting.order.lane().key() != format!("meeting:{MEETING}"))
    );
    // Another meeting and a voice note are untouched, audio and all.
    assert_eq!(
        (jobs_of(&store, OTHER)?, files(directory.path(), OTHER)?),
        other_before
    );
    assert!(store.private_notes(OTHER)?.is_some());
    assert_eq!(store.note_orders()?.len(), 1);
    assert_eq!(store.list_chunks(VOICE, None, 10)?.len(), 3);
    assert!(store.read_chunk(&format!("{VOICE}-mic_local-1")).is_ok());
    // Nothing is left for recovery to bring back, and again is no error.
    drop(store);
    let mut store = Store::open(directory.path(), KEY)?;
    let mut scan = store.begin_recovery()?;
    let report = store.recover_next(&mut scan, 100)?;
    assert_eq!(report.missing, 0);
    assert!(store.list_chunks(MEETING, None, 10)?.is_empty());
    let again = store.delete_recording(MEETING, DeletionScope::Everything, 10)?;
    assert_eq!((again.chunks, again.jobs, again.done), (0, 0, true));
    Ok(())
}

/// «Terminar sin guardar» takes the audio at once; what the person wrote
/// and the order to the hub wait until the hub has deleted it.
#[test]
fn a_discard_takes_the_audio_and_keeps_the_order_for_the_hub() -> TestResult {
    let directory = tempfile::tempdir()?;
    let mut store = Store::open(directory.path(), KEY)?;
    recorded(&mut store, MEETING, "meeting")?;
    store.queue_note_order(
        "delete",
        QueuedOrder::Meeting {
            recording_id: MEETING.into(),
            order: NoteOrder::Delete { discarded: true },
        },
        2,
    )?;
    let deleted = store.delete_recording(MEETING, DeletionScope::Audio, 100)?;
    assert!(deleted.done);
    assert_eq!(deleted.chunks, 2);
    assert!(store.list_chunks(MEETING, None, 10)?.is_empty());
    assert_eq!(files(directory.path(), MEETING)?, 0);
    assert_eq!(jobs_of(&store, MEETING)?, 0);
    assert!(store.private_notes(MEETING)?.is_some());
    assert_eq!(store.note_orders()?.len(), 2);
    Ok(())
}

#[test]
fn a_voice_note_or_a_call_is_never_deleted_as_a_meeting() -> TestResult {
    let directory = tempfile::tempdir()?;
    let mut store = Store::open(directory.path(), KEY)?;
    recorded(&mut store, VOICE, "voice_turn")?;
    recorded(&mut store, OTHER, "voice_call")?;
    for recording in [VOICE, OTHER] {
        assert!(matches!(
            store.delete_recording(recording, DeletionScope::Everything, 10),
            Err(Error::Invalid(_))
        ));
        assert_eq!(store.list_chunks(recording, None, 10)?.len(), 3);
    }
    Ok(())
}
