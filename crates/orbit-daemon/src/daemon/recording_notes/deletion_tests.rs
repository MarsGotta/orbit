//! «Borrar reunión» and «Terminar sin guardar» at the daemon, through its
//! commands, its helper events and the notes outbox's turns. Every request
//! goes to a synthetic hub on loopback (`fake_hub`); none of this is
//! evidence about the real hub, macOS or its audio.
use super::fake_hub::{self, Hub, lock};
use crate::daemon::{
    Runtime,
    audio::tests::{fixture, settle_seal_receipts},
};
use anyhow::{Context, Result, bail};
use base64::{Engine, engine::general_purpose::STANDARD};
use orbit_core::{
    CapturePhase, Event, notice::NoticeCode, protocol::Command, reads::ReadRequest,
    topics::Knowledge,
};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;

const TODAY: &str = "6f2b7d5a-9c41-4b7e-8d3f-1a2b3c4d5e6f";
const OTHER: &str = "8e2d6b1f-3c4a-4d5e-9f6a-7b8c9d0e1f2a";
const LIVE: &str = "4f1c7a2e-9b3d-4e8f-a6c5-2d1e0f9a8b7c";
const DEVICE: &str = "11111111-1111-4111-8111-111111111111";
const START: i64 = 1_800_000_000_000;

fn hub_with(documents: &[&str]) -> fake_hub::Shared {
    let hub: fake_hub::Shared = Arc::new(Mutex::new(Hub::default()));
    for recording in documents {
        let mut document = fake_hub::document(recording, "Revisión de Lumen", START);
        document["note"] = fake_hub::note(7);
        lock(&hub).documents.insert((*recording).into(), document);
    }
    hub
}

fn pair(runtime: &mut Runtime, url: &str) {
    runtime.settings.hub_url = Some(url.into());
    runtime.settings.device_id = Some(DEVICE.into());
    runtime.link.token = Some("synthetic-not-a-credential".into());
}

fn connection(runtime: &mut Runtime, value: &str) {
    crate::daemon::authority_fixture::set(&mut runtime.state, "connection", json!(value));
}

/// The hub's `ready` frame, through the path production takes: a hub of
/// this version says it deletes recordings, a 0.11 hub says nothing.
async fn ready(runtime: &mut Runtime, deletes: bool) -> Result<()> {
    let mut data = json!({"device_id": DEVICE, "hub_protocol": 1});
    if deletes {
        data["recording_delete"] = json!(true);
    }
    runtime
        .hub_event(crate::hub::HubEvent::Frame(
            json!({"event": "ready", "data": data}),
        ))
        .await
}

/// What the recording topic says is being recorded.
fn recorded(runtime: &Runtime) -> Result<Value> {
    let source = crate::daemon::topics::TopicSource {
        state: &runtime.state,
        runtime: Some(runtime),
        now_ms: 0,
    };
    let mut board = crate::daemon::topics::TopicBoard::new("deletion".into());
    let channels = board.channels();
    let _recording = channels[&orbit_core::topics::TopicId::Recording].subscribe();
    board.publish(&source);
    let topic = serde_json::to_value(
        &channels[&orbit_core::topics::TopicId::Recording]
            .borrow()
            .data,
    )?;
    Ok(topic["knowledge"]["value"]["recording"].clone())
}

/// Turns of the daemon at `now_ms` until nothing is sent and nothing comes
/// back: each request's answer and each deletion on the store come back
/// through the event path production uses.
async fn settle(
    runtime: &mut Runtime,
    events: &mut mpsc::Receiver<super::NoteEvent>,
    now_ms: i64,
) -> Result<()> {
    runtime.notes.pinned_now = Some(now_ms);
    for _ in 0..40 {
        runtime.maintain_recording_notes_at(now_ms);
        let notes = &runtime.notes;
        if notes.outbox.in_flight() == 0 && notes.deletions.purging.is_none() {
            return Ok(());
        }
        let event = tokio::time::timeout(std::time::Duration::from_secs(10), events.recv())
            .await
            .context("the synthetic hub or the store did not answer")?
            .context("the note channel closed")?;
        runtime.note_answer_at(event, now_ms).await?;
    }
    bail!("the outbox never settled")
}

fn received(hub: &fake_hub::Shared, method: &str, recording: &str) -> Vec<String> {
    lock(hub)
        .received
        .iter()
        .filter(|request| request.method == method && request.path.contains(recording))
        .map(|request| request.path.clone())
        .collect()
}

/// The meetings the Notes list shows now.
async fn listed(runtime: &Runtime) -> Result<Vec<String>> {
    let page = runtime
        .notes_read(&ReadRequest::NotesPage { cursor: None })
        .context("a notes read")?
        .await;
    let Knowledge::Present { value, .. } = page else {
        bail!("the list was not read: {page:?}");
    };
    Ok(value
        .items
        .iter()
        .filter_map(|item| item["recording_id"].as_str().map(str::to_owned))
        .collect())
}

/// What the person wrote about two meetings while the hub was away: each
/// one's title and today's private notes.
async fn written(runtime: &mut Runtime) -> Result<()> {
    for recording in [TODAY, OTHER] {
        runtime
            .command(Command::MeetingRename {
                recording_id: recording.into(),
                title: "Revisión del trimestre".into(),
            })
            .await?;
    }
    runtime
        .command(Command::NotePrivateSave {
            recording_id: TODAY.into(),
            markdown: "- Ojo con el plazo".into(),
            base_revision: 0,
        })
        .await?;
    Ok(())
}

fn said(runtime: &Runtime) -> Vec<NoticeCode> {
    runtime
        .state
        .durable
        .notice_log
        .records()
        .iter()
        .map(|notice| notice.code)
        .collect()
}

fn delete(recording: &str) -> Command {
    Command::MeetingDelete {
        recording_id: recording.into(),
    }
}

#[tokio::test]
async fn a_deleted_meeting_leaves_the_list_this_mac_and_the_hub_and_the_other_stays() -> Result<()>
{
    let directory = tempfile::tempdir()?;
    let hub = hub_with(&[TODAY, OTHER]);
    let url = fake_hub::start(hub.clone()).await?;
    let (mut runtime, _) = fixture(directory.path())?;
    let mut events = runtime.notes.test_events();
    pair(&mut runtime, &url);
    connection(&mut runtime, "offline");
    written(&mut runtime).await?;
    runtime
        .command(delete(TODAY))
        .await
        .context("the delete was refused with the hub away")?;
    ready(&mut runtime, true).await?;
    // Gone from the list at once, before the hub has it.
    assert_eq!(listed(&runtime).await?, vec![OTHER.to_owned()]);
    settle(&mut runtime, &mut events, START).await?;

    assert_eq!(received(&hub, "DELETE", TODAY).len(), 1);
    // Nothing else of it went: the delete makes it moot.
    assert!(received(&hub, "PUT", TODAY).is_empty());
    assert!(!lock(&hub).documents.contains_key(TODAY));
    assert!(runtime.reader.private_notes(TODAY)?.is_none());
    assert!(runtime.reader.note_orders()?.is_empty());
    assert!(runtime.notes.outbox.deleting().is_empty());
    // The other meeting's order reached the hub, and it is still listed.
    assert_eq!(received(&hub, "PUT", OTHER).len(), 1);
    assert_eq!(listed(&runtime).await?, vec![OTHER.to_owned()]);
    // Twice is no failure: the hub answers a second delete as done.
    runtime.command(delete(TODAY)).await?;
    settle(&mut runtime, &mut events, START + 1).await?;
    assert_eq!(received(&hub, "DELETE", TODAY).len(), 2);
    assert!(runtime.reader.note_orders()?.is_empty());
    Ok(())
}

/// A 0.11 hub has no delete: this Mac deletes nothing of the meeting, says
/// so, lists it again, and what the person wrote still goes. A hub known
/// not to delete takes no delete at all.
#[tokio::test]
async fn a_hub_that_cannot_delete_leaves_everything_here_and_says_to_update_it() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let hub = hub_with(&[TODAY, OTHER]);
    lock(&hub).without_recording_delete = true;
    let url = fake_hub::start(hub.clone()).await?;
    let (mut runtime, _) = fixture(directory.path())?;
    let mut events = runtime.notes.test_events();
    pair(&mut runtime, &url);
    connection(&mut runtime, "offline");
    written(&mut runtime).await?;
    runtime.command(delete(TODAY)).await?;
    // The hub's `ready` was not seen: whether it deletes is not known.
    connection(&mut runtime, "online");
    runtime.notes.notes_hub_ready();
    settle(&mut runtime, &mut events, START).await?;

    assert_eq!(received(&hub, "DELETE", TODAY).len(), 1);
    assert!(said(&runtime).contains(&NoticeCode::MeetingDeleteNeedsHubUpdate));
    let mut back = listed(&runtime).await?;
    back.sort();
    assert_eq!(back, vec![TODAY.to_owned(), OTHER.to_owned()]);
    // Its title and private notes went to the hub after all.
    assert_eq!(received(&hub, "PUT", TODAY).len(), 2);
    assert!(runtime.reader.note_orders()?.is_empty());

    let refused = runtime
        .command(delete(TODAY))
        .await
        .err()
        .context("a hub that cannot delete took a delete")?;
    assert!(
        format!("{refused:#}").contains("Actualiza el hub para borrar reuniones"),
        "{refused:#}"
    );
    assert!(runtime.reader.note_orders()?.is_empty());
    Ok(())
}

/// A meeting still closing is deleted once its closure is on the hub.
#[tokio::test]
async fn a_meeting_still_closing_is_deleted_once_it_closed() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let hub = hub_with(&[TODAY]);
    let url = fake_hub::start(hub.clone()).await?;
    let (mut runtime, _) = fixture(directory.path())?;
    let mut events = runtime.notes.test_events();
    pair(&mut runtime, &url);
    let closure = orbit_store::Job {
        id: format!("finalize-{TODAY}"),
        kind: "recording_finalize".into(),
        state: orbit_store::JobState::Queued,
        created_at: START,
        payload: json!({"recording_id": TODAY, "manifest": {"tracks": []}}),
    };
    runtime
        .store_actor
        .run(move |store| Ok(store.enqueue_job(&closure)?))
        .await?;
    ready(&mut runtime, true).await?;
    runtime.command(delete(TODAY)).await?;
    settle(&mut runtime, &mut events, START).await?;
    assert!(received(&hub, "DELETE", TODAY).is_empty());
    assert!(listed(&runtime).await?.is_empty());

    let id = format!("finalize-{TODAY}");
    runtime
        .store_actor
        .run(move |store| Ok(store.update_job(&id, orbit_store::JobState::Succeeded)?))
        .await?;
    settle(&mut runtime, &mut events, START + 6_000).await?;
    assert_eq!(received(&hub, "DELETE", TODAY).len(), 1);
    assert!(runtime.reader.note_orders()?.is_empty());
    Ok(())
}

async fn start_live(runtime: &mut Runtime, helper: &mut mpsc::Receiver<Value>) -> Result<String> {
    runtime
        .apply(Event::StartMeeting {
            id: LIVE.into(),
            title: "Revisión de Lumen".into(),
            audio: orbit_core::protocol::MeetingAudio::MicrophoneOnly,
        })
        .await?;
    let command = helper.recv().await.context("start command absent")?;
    assert_eq!(command["command"], "audio_start");
    runtime
        .native_event(json!({"event":"response", "id":command["id"], "ok":true,
            "result":{"stream_id":command["stream_id"]}}))
        .await?;
    Ok(command["stream_id"]
        .as_str()
        .context("stream absent")?
        .to_owned())
}

/// One second of microphone audio at 8 kHz: one chunk.
fn second_of_audio(stream: &str, sequence: u64) -> Value {
    json!({"event":"audio", "source":"mic_local", "stream_id":stream,
        "sample_rate":8000, "channels":1, "frames":8000, "sequence":sequence,
        "monotonic_ns":sequence.saturating_mul(1_000_000_000),
        "pcm_s16le_base64":STANDARD.encode(vec![1; 16_000])})
}

/// A recording with two seconds kept on this Mac, thrown away: the helper
/// is asked to stop, what drains after is dropped, and the recording ends
/// without being closed («Grabación guardada» is never said).
async fn thrown_away(runtime: &mut Runtime, helper: &mut mpsc::Receiver<Value>) -> Result<()> {
    let stream = start_live(runtime, helper).await?;
    for sequence in 0..2 {
        runtime
            .native_event(second_of_audio(&stream, sequence))
            .await?;
    }
    settle_seal_receipts(runtime).await?;
    assert_eq!(runtime.reader.list_chunks(LIVE, None, 10)?.len(), 2);
    runtime
        .command(Command::MeetingDiscard {
            recording_id: LIVE.into(),
        })
        .await?;
    let stop = helper.recv().await.context("stop absent")?;
    assert_eq!(stop["command"], "audio_stop");
    // Gone for the person at once, while what the helper had drains.
    assert_eq!(recorded(runtime)?, Value::Null);
    runtime.native_event(second_of_audio(&stream, 2)).await?;
    runtime
        .native_event(json!({"event":"response", "id":stop["id"], "ok":true,
            "result":{"stream_id":stream,"drain_complete":true}}))
        .await?;
    settle_seal_receipts(runtime).await?;
    // What drained after the stop was never sealed.
    assert_eq!(runtime.reader.list_chunks(LIVE, None, 10)?.len(), 2);
    assert_eq!(runtime.state.durable.capture.recording, None);
    assert_eq!(
        runtime.state.notice_code(),
        Some(NoticeCode::RecordingDiscarded)
    );
    assert!(!said(runtime).contains(&NoticeCode::RecordingFinished));
    assert!(
        runtime
            .reader
            .get_job(&format!("finalize-{LIVE}"))?
            .is_none()
    );
    Ok(())
}

fn chunk_files(runtime: &Runtime) -> Result<usize> {
    Ok(std::fs::read_dir(runtime.paths.store.join("chunks"))?
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name().to_string_lossy().starts_with(LIVE))
        .count())
}

#[tokio::test]
async fn ending_without_saving_leaves_no_chunk_here_or_on_the_hub_and_no_note() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let hub = hub_with(&[]);
    let url = fake_hub::start(hub.clone()).await?;
    let (mut runtime, mut helper) = fixture(directory.path())?;
    let mut events = runtime.notes.test_events();
    pair(&mut runtime, &url);
    ready(&mut runtime, true).await?;
    thrown_away(&mut runtime, &mut helper).await?;
    settle(&mut runtime, &mut events, START).await?;

    assert!(runtime.reader.list_chunks(LIVE, None, 10)?.is_empty());
    assert_eq!(chunk_files(&runtime)?, 0);
    assert_eq!(received(&hub, "DELETE", LIVE).len(), 1);
    // No closure, so the hub never writes a note nor says «Tu nota está lista».
    assert!(received(&hub, "POST", LIVE).is_empty());
    assert!(runtime.reader.note_orders()?.is_empty());
    Ok(())
}

/// With the hub away the audio leaves this Mac at once, the order waits
/// through a restart, and the hub deletes it when it returns.
#[tokio::test]
async fn ending_without_saving_with_the_hub_away_completes_after_a_restart() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let hub = hub_with(&[]);
    let url = fake_hub::start(hub.clone()).await?;
    {
        let (mut runtime, mut helper) = fixture(directory.path())?;
        let mut events = runtime.notes.test_events();
        pair(&mut runtime, &url);
        connection(&mut runtime, "offline");
        thrown_away(&mut runtime, &mut helper).await?;
        settle(&mut runtime, &mut events, START).await?;
        assert!(runtime.reader.list_chunks(LIVE, None, 10)?.is_empty());
        assert_eq!(chunk_files(&runtime)?, 0);
        assert!(lock(&hub).received.is_empty());
        runtime.store_actor.run(|_| Ok(())).await?;
    }
    let (mut runtime, _helper) = fixture(directory.path())?;
    let mut events = runtime.notes.test_events();
    pair(&mut runtime, &url);
    runtime.recover_recording_notes()?;
    assert_eq!(
        runtime
            .notes
            .outbox
            .discarded()
            .into_iter()
            .collect::<Vec<_>>(),
        vec![LIVE.to_owned()]
    );
    ready(&mut runtime, true).await?;
    settle(&mut runtime, &mut events, START + 1).await?;
    assert_eq!(received(&hub, "DELETE", LIVE).len(), 1);
    assert!(runtime.reader.note_orders()?.is_empty());
    assert!(runtime.reader.list_chunks(LIVE, None, 10)?.is_empty());
    Ok(())
}

/// A hub known not to delete: «Terminar sin guardar» is refused before
/// anything changes, and the recording goes on. Another recording than the
/// one recording is never thrown away.
#[tokio::test]
async fn a_hub_that_cannot_delete_refuses_ending_without_saving() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let hub = hub_with(&[]);
    let url = fake_hub::start(hub.clone()).await?;
    let (mut runtime, mut helper) = fixture(directory.path())?;
    pair(&mut runtime, &url);
    ready(&mut runtime, false).await?;
    let stream = start_live(&mut runtime, &mut helper).await?;
    runtime.native_event(second_of_audio(&stream, 0)).await?;
    settle_seal_receipts(&mut runtime).await?;
    let refused = runtime
        .command(Command::MeetingDiscard {
            recording_id: LIVE.into(),
        })
        .await
        .err()
        .context("a hub that cannot delete took a discard")?;
    assert!(
        format!("{refused:#}").contains("Actualiza el hub"),
        "{refused:#}"
    );
    let recording = runtime
        .state
        .durable
        .capture
        .recording
        .as_ref()
        .context("gone")?;
    assert_eq!(recording.phase, CapturePhase::Recording);
    assert!(runtime.reader.note_orders()?.is_empty());
    assert_eq!(runtime.reader.list_chunks(LIVE, None, 10)?.len(), 1);
    ready(&mut runtime, true).await?;
    let wrong = runtime
        .command(Command::MeetingDiscard {
            recording_id: OTHER.into(),
        })
        .await;
    assert!(wrong.is_err());
    assert!(runtime.reader.note_orders()?.is_empty());
    assert_eq!(runtime.reader.list_chunks(LIVE, None, 10)?.len(), 1);
    Ok(())
}

fn chunk(recording: &str) -> orbit_store::ChunkMetadata {
    orbit_store::ChunkMetadata {
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

/// The capture task uploads the other recording's chunk and never one of
/// a recording thrown away, which it does not even mark as running.
#[tokio::test]
async fn no_chunk_of_a_thrown_away_recording_goes_up() -> Result<()> {
    use tokio::io::AsyncReadExt;
    let directory = tempfile::tempdir()?;
    let (mut runtime, _) = fixture(directory.path())?;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    runtime.settings.hub_url = Some(format!("http://{}", listener.local_addr()?));
    runtime.link.token = Some("synthetic-not-a-credential".into());
    runtime
        .keep_order(orbit_core::note_outbox::QueuedOrder::Meeting {
            recording_id: LIVE.into(),
            order: orbit_core::note_outbox::NoteOrder::Delete { discarded: true },
        })
        .await?;
    for recording in [LIVE, OTHER] {
        runtime
            .fixture_store
            .seal_chunk(&chunk(recording), b"synthetic")?;
    }
    let (mut task, _bridge) = crate::daemon::capture_lane::test_support::spawn_task(&runtime)?;
    let accepted = tokio::time::timeout(
        crate::daemon::capture_lane::test_support::TASK_JAM,
        listener.accept(),
    );
    let (mut stream, _) = accepted.await??;
    let mut head = Vec::new();
    let mut byte = [0];
    while !head.ends_with(b"\r\n\r\n") {
        stream.read_exact(&mut byte).await?;
        head.push(byte[0]);
    }
    task.stop();
    let head = String::from_utf8_lossy(&head);
    assert!(head.contains(OTHER), "{head}");
    assert!(!head.contains(LIVE), "{head}");
    let job = runtime
        .reader
        .get_job(&format!("upload-{LIVE}-mic_local-0"))?
        .context("its job is still there to be deleted")?;
    assert_eq!(job.state, orbit_store::JobState::Queued);
    Ok(())
}
