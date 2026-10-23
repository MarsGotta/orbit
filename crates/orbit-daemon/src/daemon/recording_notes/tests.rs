//! The meeting notes at the daemon: the Mac passes through what the hub
//! computes, adds only what it owns, and carries the user's orders to the hub.
//! Every request goes to a synthetic hub on loopback (`fake_hub`); none of
//! this is evidence about the real hub.
use super::fake_hub::{self, Hub, lock};
use crate::daemon::{Runtime, audio::tests::fixture};
use anyhow::{Context, Result, bail};
use orbit_core::{
    notes::{ListItem, LocalPhase, MeetingDocument, Turn},
    protocol::Command,
    reads::ReadRequest,
    topics::{Cursor, Knowledge, Page},
};
use orbit_store::{Job, JobState};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};

const RECORDING: &str = "6f2b7d5a-9c41-4b7e-8d3f-1a2b3c4d5e6f";
const UPLOADING: &str = "7a2b7d5a-9c41-4b7e-8d3f-1a2b3c4d5e6f";
const OLDER: &str = "8b2b7d5a-9c41-4b7e-8d3f-1a2b3c4d5e6f";
const DEVICE: &str = "11111111-1111-4111-8111-111111111111";
const START: i64 = 1_800_000_000_000;

async fn paired(runtime: &mut Runtime) -> Result<fake_hub::Shared> {
    let hub: fake_hub::Shared = Arc::new(Mutex::new(Hub { ..Hub::default() }));
    runtime.settings.hub_url = Some(fake_hub::start(hub.clone()).await?);
    runtime.settings.device_id = Some(DEVICE.into());
    runtime.token = Some("synthetic-not-a-credential".into());
    Ok(hub)
}

fn start_recording(runtime: &mut Runtime) {
    crate::daemon::authority_fixture::set(
        &mut runtime.state,
        "recording",
        json!({"id": RECORDING, "title": "Comité de dirección", "started_at_ms": START,
            "ended_at_ms": null, "expires_at_ms": START + 3_600_000, "application_pid": null,
            "audio": "system", "phase": "recording", "sealed_chunks": 0, "gaps": 0}),
    );
}

fn uploading(runtime: &mut Runtime, recording: &str, started_at_ms: i64) -> Result<()> {
    runtime.fixture_store.enqueue_job(&Job {
        id: format!("finalize-{recording}"),
        kind: "recording_finalize".into(),
        payload: json!({"recording_id": recording, "manifest": {
            "started_at_ms": started_at_ms, "finalized_at_ms": started_at_ms + 60_000,
            "title": "Uno a uno", "tracks": []}}),
        state: JobState::Queued,
        created_at: started_at_ms + 60_000,
    })?;
    Ok(())
}

/// One turn of the daemon: whatever is due goes out, and its answer comes
/// back through the same event path production uses.
async fn turn(
    runtime: &mut Runtime,
    events: &mut tokio::sync::mpsc::Receiver<super::NoteEvent>,
    now_ms: i64,
) -> Result<usize> {
    runtime.features.notes.pinned_now = Some(now_ms);
    let started = runtime.maintain_recording_notes_at(now_ms);
    for _ in 0..started {
        answer(runtime, events, now_ms).await?;
    }
    Ok(started)
}

async fn answer(
    runtime: &mut Runtime,
    events: &mut tokio::sync::mpsc::Receiver<super::NoteEvent>,
    now_ms: i64,
) -> Result<()> {
    let event = tokio::time::timeout(std::time::Duration::from_secs(10), events.recv())
        .await
        .context("the synthetic hub did not answer")?
        .context("the note channel closed")?;
    runtime.note_answer_at(event, now_ms).await
}

async fn read(runtime: &Runtime, request: ReadRequest) -> Result<Page<Value>> {
    let future = runtime.notes_read(&request).context("not a notes read")?;
    match future.await {
        Knowledge::Present { value, .. } => Ok(value),
        other => bail!("the read was not answered: {other:?}"),
    }
}

async fn document(runtime: &Runtime) -> Result<MeetingDocument> {
    let page = read(
        runtime,
        ReadRequest::NoteDetail {
            id: RECORDING.into(),
            cursor: None,
        },
    )
    .await?;
    Ok(serde_json::from_value(
        page.items.into_iter().next().context("empty detail")?,
    )?)
}

async fn every_turn(runtime: &Runtime) -> Result<Vec<Turn>> {
    let mut turns = Vec::new();
    let mut cursor = None;
    loop {
        let page = read(
            runtime,
            ReadRequest::NoteTranscript {
                id: RECORDING.into(),
                cursor: cursor.clone(),
            },
        )
        .await?;
        for item in page.items {
            turns.push(serde_json::from_value(item)?);
        }
        match page.next {
            Some(next) => cursor = Some(next),
            None => return Ok(turns),
        }
    }
}

fn received(hub: &fake_hub::Shared, method: &str, route: &str) -> Vec<Value> {
    lock(hub)
        .received
        .iter()
        .filter(|request| request.method == method && request.path.ends_with(route))
        .map(|request| request.body.clone())
        .collect()
}

/// The daemon forwards every turn unchanged across the hub's pages.
/// Language, speaker names and relative times are computed by the real hub,
/// covered by server/tests/test_meeting_views.py::test_c1_turns_are_relative_named_never_empty_and_the_meeting_says_its_language.
#[tokio::test]
async fn the_hubs_turns_arrive_unchanged_across_pages() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _) = fixture(directory.path())?;
    let hub = paired(&mut runtime).await?;
    {
        let mut hub = lock(&hub);
        hub.documents.insert(
            RECORDING.into(),
            fake_hub::document(RECORDING, "Comité de dirección", START),
        );
        hub.turns.insert(
            RECORDING.into(),
            vec![
                json!({"at_ms": 0, "speaker_key": "mic_local", "text": "Empezamos."}),
                json!({"at_ms": 12_000, "speaker_key": "remote_app", "text": "Vale."}),
                json!({"at_ms": 61_000, "speaker_key": "mic_local", "text": "Siguiente punto."}),
            ],
        );
    }
    let turns = every_turn(&runtime).await?;
    let expected: Vec<Turn> = lock(&hub).turns[RECORDING]
        .iter()
        .cloned()
        .map(serde_json::from_value)
        .collect::<std::result::Result<_, _>>()?;
    assert_eq!(turns, expected);
    // Two turns per page: three turns took two reads with a cursor.
    assert_eq!(received(&hub, "GET", "/turns").len(), 2);
    Ok(())
}

/// The owner's name goes to the hub when it connects, once.
#[tokio::test]
async fn the_account_name_goes_to_the_hub_on_connecting() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _) = fixture(directory.path())?;
    runtime.features.notes = super::NotesRuntime::for_tests(runtime.features.notes.events.clone());
    let mut events = runtime.features.notes.test_events();
    let hub = paired(&mut runtime).await?;
    assert_eq!(
        turn(&mut runtime, &mut events, START).await?,
        0,
        "not yet ready"
    );
    runtime.notes_hub_ready();
    assert_eq!(turn(&mut runtime, &mut events, START).await?, 1);
    assert_eq!(
        received(&hub, "PUT", "/v1/device/profile"),
        [json!({"owner_name": "Alina"})]
    );
    assert_eq!(turn(&mut runtime, &mut events, START + 1).await?, 0);
    Ok(())
}

/// The list: the recording in progress, the ones uploading and the hub's,
/// each once, newest first.
#[tokio::test]
async fn the_list_joins_this_macs_recordings_and_the_hubs() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _) = fixture(directory.path())?;
    let hub = paired(&mut runtime).await?;
    start_recording(&mut runtime);
    uploading(&mut runtime, UPLOADING, START - 3_600_000)?;
    {
        let mut hub = lock(&hub);
        // The hub already has chunks of both: their rows must not repeat.
        let mut open = fake_hub::document(RECORDING, "Comité de dirección", START);
        open["capture"] = json!("open");
        hub.documents.insert(RECORDING.into(), open);
        hub.documents.insert(
            UPLOADING.into(),
            fake_hub::document(UPLOADING, "Uno a uno", START - 3_600_000),
        );
        hub.documents.insert(
            OLDER.into(),
            fake_hub::document(OLDER, "Retro", START - 86_400_000),
        );
    }
    runtime.features.notes.pinned_now = Some(START + 60_000);
    let page = read(&runtime, ReadRequest::NotesPage { cursor: None }).await?;
    let rows = page
        .items
        .into_iter()
        .map(serde_json::from_value::<ListItem>)
        .collect::<Result<Vec<_>, _>>()?;
    let rows = rows
        .into_iter()
        .map(|item| match item {
            ListItem::Meeting(row) => Ok(row),
            ListItem::HubRefused { hub_refusal } => bail!("hub refused: {hub_refusal}"),
        })
        .collect::<Result<Vec<_>>>()?;
    assert_eq!(
        rows.iter()
            .map(|row| row.recording_id.as_str())
            .collect::<Vec<_>>(),
        [RECORDING, UPLOADING, OLDER]
    );
    assert_eq!(
        rows[0].local.map(|local| local.phase),
        Some(LocalPhase::Recording)
    );
    assert_eq!(rows[0].duration_ms, 60_000);
    assert_eq!(
        rows[1].local.map(|local| local.phase),
        Some(LocalPhase::Uploading)
    );
    assert!(rows[2].local.is_none());
    assert_eq!(page.next, None);
    Ok(())
}

/// Without the hub the list still shows this Mac's recordings, and says why
/// the rest is missing.
#[tokio::test]
async fn without_the_hub_the_macs_recordings_are_still_listed() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _) = fixture(directory.path())?;
    start_recording(&mut runtime);
    let page = read(&runtime, ReadRequest::NotesPage { cursor: None }).await?;
    let items = page
        .items
        .into_iter()
        .map(serde_json::from_value::<ListItem>)
        .collect::<Result<Vec<_>, _>>()?;
    assert!(matches!(
        &items[0],
        ListItem::HubRefused {
            hub_refusal: orbit_core::refusal::Refusal::NotPaired
        }
    ));
    assert!(matches!(&items[1], ListItem::Meeting(row) if row.recording_id == RECORDING));
    // The note being recorded opens all the same, with what the Mac knows.
    let shown = document(&runtime).await?;
    assert_eq!(shown.title, "Comité de dirección");
    assert!(shown.hub_refusal.is_some());
    Ok(())
}

/// A recording in progress is named in its note as in the list. The
/// hub has no title for it until it closes («Reunión del…», from its first
/// chunk); this Mac does, and gives it to both.
#[tokio::test]
async fn a_recording_in_progress_is_named_in_its_note_as_in_the_list() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _) = fixture(directory.path())?;
    let hub = paired(&mut runtime).await?;
    start_recording(&mut runtime);
    let mut open = fake_hub::document(RECORDING, "Reunión del 15/01/2027 08:00", START);
    open["capture"] = json!("open");
    lock(&hub).documents.insert(RECORDING.into(), open);
    let shown = document(&runtime).await?;
    assert!(shown.hub_refusal.is_none());
    assert_eq!(shown.title, "Comité de dirección");
    Ok(())
}

/// The hub's list continues by cursor, the start of its last row.
#[tokio::test]
async fn the_list_continues_from_the_last_row() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _) = fixture(directory.path())?;
    let hub = paired(&mut runtime).await?;
    {
        let mut hub = lock(&hub);
        for index in 0..51_i64 {
            let id = format!("00000000-0000-4000-8000-{index:012x}");
            hub.documents.insert(
                id.clone(),
                fake_hub::document(&id, "Reunión", START - index * 60_000),
            );
        }
    }
    let first = read(&runtime, ReadRequest::NotesPage { cursor: None }).await?;
    assert_eq!(first.items.len(), 50);
    let Some(next @ Cursor::DescendingDateAndId { date_ms, .. }) = first.next.clone() else {
        bail!("no cursor after a full page: {:?}", first.next);
    };
    assert_eq!(date_ms, START - 49 * 60_000);
    let second = read(&runtime, ReadRequest::NotesPage { cursor: Some(next) }).await?;
    assert_eq!(second.items.len(), 1);
    assert_eq!(second.next, None);
    assert!(
        lock(&hub)
            .received
            .iter()
            .any(|request| request.query.contains(&format!("before_ms={date_ms}")))
    );
    Ok(())
}

/// A voice call uploads as it goes, so the hub lists it open until it closes
/// (only then does it say `voice_call`); this Mac knows it is its call and
/// leaves it out.
#[tokio::test]
async fn the_voice_call_in_progress_is_not_a_meeting() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _) = fixture(directory.path())?;
    let hub = paired(&mut runtime).await?;
    {
        let mut hub = lock(&hub);
        let mut call = fake_hub::document(UPLOADING, "Reunión en curso", START);
        call["capture"] = json!("open");
        hub.documents.insert(UPLOADING.into(), call);
        hub.documents.insert(
            OLDER.into(),
            fake_hub::document(OLDER, "Comité", START - 60_000),
        );
    }
    crate::daemon::authority_fixture::set(
        &mut runtime.state,
        "call",
        json!({"id": UPLOADING, "phase": "listening", "started_at_ms": START,
            "expires_at_ms": START + 600_000, "turns": 0}),
    );
    let page = read(&runtime, ReadRequest::NotesPage { cursor: None }).await?;
    let listed = page
        .items
        .iter()
        .filter_map(|item| item["recording_id"].as_str())
        .collect::<Vec<_>>();
    assert_eq!(listed, [OLDER]);
    Ok(())
}

/// A first page at every bound — fifty of the hub's rows with a 256-character
/// title and a 1,000-character summary, a hundred recordings of this Mac
/// still uploading and the one being recorded, every character escaped —
/// fits its read.
#[tokio::test]
async fn a_first_page_at_every_bound_fits_its_read() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _) = fixture(directory.path())?;
    let hub = paired(&mut runtime).await?;
    let escaped = |characters: usize| "\u{1}".repeat(characters);
    {
        let mut hub = lock(&hub);
        for index in 0..50_i64 {
            let id = format!("00000000-0000-4000-8000-{index:012x}");
            let mut document = fake_hub::document(&id, &escaped(256), START - index * 60_000);
            document["note"] = fake_hub::note(1);
            document["note"]["note"]["short_summary"] = json!(escaped(1000));
            hub.documents.insert(id, document);
        }
    }
    for index in 0..100_i64 {
        let id = format!("10000000-0000-4000-8000-{index:012x}");
        runtime.fixture_store.enqueue_job(&Job {
            id: format!("finalize-{id}"),
            kind: "recording_finalize".into(),
            payload: json!({"recording_id": id, "manifest": {
                "started_at_ms": START + index, "finalized_at_ms": START + index + 60_000,
                "title": escaped(256), "tracks": []}}),
            state: JobState::Queued,
            created_at: START,
        })?;
    }
    start_recording(&mut runtime);
    let page = read(&runtime, ReadRequest::NotesPage { cursor: None }).await?;
    assert_eq!(page.items.len(), 151);
    Ok(())
}

/// `recording.changed` names the recording to read again; nothing else does.
#[tokio::test]
async fn an_open_note_is_read_again_by_recording_changed_and_nothing_else() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _) = fixture(directory.path())?;
    let mut events = runtime.features.notes.test_events();
    let _hub = paired(&mut runtime).await?;
    let before = runtime.features.notes.head.changed_at(RECORDING);
    // A minute of the daemon's upkeep moves nothing about this note.
    for second in 0..60 {
        turn(&mut runtime, &mut events, START + second * 1_000).await?;
    }
    assert_eq!(runtime.features.notes.head.changed_at(RECORDING), before);
    let frame = orbit_core::wire::hub::HubEvent::decode(json!({
        "v": 1, "id": null, "event": "recording.changed",
        "data": {"recording_id": RECORDING}
    }))
    .map_err(anyhow::Error::msg)?;
    runtime
        .hub_event(crate::hub::HubEvent::Decoded(Box::new(frame)))
        .await?;
    assert!(runtime.features.notes.head.changed_at(RECORDING) > before);
    assert_eq!(runtime.features.notes.head.changed_at(OLDER), 0);
    Ok(())
}

/// Private notes written without the hub wait in the store and reach it when
/// it returns; the hub's confirmation empties the outbox.
#[tokio::test]
async fn private_notes_written_offline_reach_the_hub_when_it_returns() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _) = fixture(directory.path())?;
    let mut events = runtime.features.notes.test_events();
    let hub = paired(&mut runtime).await?;
    start_recording(&mut runtime);
    crate::daemon::authority_fixture::set(&mut runtime.state, "connection", json!("offline"));
    runtime
        .command(Command::NotePrivateSave {
            recording_id: RECORDING.into(),
            markdown: "- **presupuesto** pendiente".into(),
            base_revision: 0,
        })
        .await?;
    assert_eq!(turn(&mut runtime, &mut events, START).await?, 0);
    // What the user typed is what the note shows, before the hub has it.
    let shown = document(&runtime).await?;
    assert_eq!(shown.private_notes.text, "- **presupuesto** pendiente");
    assert!(shown.private_notes.pending);

    crate::daemon::authority_fixture::set(&mut runtime.state, "connection", json!("online"));
    runtime.notes_hub_ready();
    turn(&mut runtime, &mut events, START + 1_000).await?;
    assert_eq!(
        received(&hub, "PUT", "/private-notes"),
        [json!({"text": "- **presupuesto** pendiente", "base_revision": 0})]
    );
    assert_eq!(
        runtime.reader.pending_private_notes()?,
        Vec::<String>::new()
    );
    assert_eq!(turn(&mut runtime, &mut events, START + 2_000).await?, 0);
    Ok(())
}

/// A stale revision is refused by the hub with a 409: the daemon reads the
/// current one and sends the user's text once more on it.
#[tokio::test]
async fn private_notes_on_a_stale_revision_go_again_on_the_current_one() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _) = fixture(directory.path())?;
    let mut events = runtime.features.notes.test_events();
    let hub = paired(&mut runtime).await?;
    let mut document = fake_hub::document(RECORDING, "Comité", START);
    document["private_notes"] = json!({"text": "antes", "revision": 4});
    lock(&hub).documents.insert(RECORDING.into(), document);
    runtime
        .command(Command::NotePrivateSave {
            recording_id: RECORDING.into(),
            markdown: "después".into(),
            base_revision: 3,
        })
        .await?;
    turn(&mut runtime, &mut events, START).await?;
    let sent = received(&hub, "PUT", "/private-notes");
    assert_eq!(sent.len(), 2);
    assert_eq!(sent[1]["base_revision"], 4);
    // The 409 names the current revision (`{revision, base_revision}`):
    // nothing else is read to learn it.
    assert!(received(&hub, "GET", RECORDING).is_empty());
    assert!(runtime.reader.pending_private_notes()?.is_empty());
    Ok(())
}

/// The private notes an older Mac kept go up once, and leave the Mac when the
/// hub confirms them.
#[tokio::test]
async fn the_old_stores_private_notes_go_up_once() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _) = fixture(directory.path())?;
    let mut events = runtime.features.notes.test_events();
    let hub = paired(&mut runtime).await?;
    runtime.fixture_store.save_private_notes(
        &orbit_store::PendingPrivateNotes {
            recording_id: OLDER.into(),
            markdown: "- de la 0.4.0".into(),
            base_revision: 0,
        },
        1,
    )?;
    runtime.recover_recording_notes()?;
    runtime.notes_hub_ready();
    turn(&mut runtime, &mut events, START).await?;
    turn(&mut runtime, &mut events, START + 60_000).await?;
    assert_eq!(
        received(&hub, "PUT", "/private-notes"),
        [json!({"text": "- de la 0.4.0", "base_revision": 0})]
    );
    assert!(runtime.reader.pending_private_notes()?.is_empty());
    Ok(())
}

/// Renaming a speaker, editing and rewriting the note go to the hub's routes
/// as the user gave them.
#[tokio::test]
async fn orders_reach_the_hubs_routes() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _) = fixture(directory.path())?;
    let mut events = runtime.features.notes.test_events();
    let hub = paired(&mut runtime).await?;
    let mut document = fake_hub::document(RECORDING, "Comité", START);
    document["note"] = fake_hub::note(7);
    lock(&hub).documents.insert(RECORDING.into(), document);

    runtime
        .command(Command::SpeakerRename {
            recording_id: RECORDING.into(),
            key: "mic_local".into(),
            name: Some("Ada".into()),
            scope: orbit_core::notes::SpeakerRenameScope::Voice,
        })
        .await?;
    answer(&mut runtime, &mut events, START).await?;
    assert_eq!(
        received(&hub, "PUT", "/speakers"),
        [json!({"names": {"mic_local": "Ada"}})]
    );

    // An edit on a revision that moved goes once more on the current one.
    runtime
        .command(Command::NoteEdit {
            recording_id: RECORDING.into(),
            sections: Some(vec![orbit_core::notes::Section {
                name: "Resumen".into(),
                markdown: "- otro punto".into(),
            }]),
            done: [("a0123456789ab".to_owned(), true)].into(),
            base_revision: 6,
        })
        .await?;
    answer(&mut runtime, &mut events, START).await?;
    let edits = received(&hub, "PUT", "/note/edits");
    assert_eq!(edits.len(), 2);
    assert_eq!(edits[1]["base_revision"], 7);
    assert_eq!(edits[1]["done"]["a0123456789ab"], true);

    runtime
        .command(Command::NoteRegenerate {
            recording_id: RECORDING.into(),
            template_id: "one_on_one".into(),
        })
        .await?;
    answer(&mut runtime, &mut events, START).await?;
    assert_eq!(
        received(&hub, "POST", "/note"),
        [json!({"template_id": "one_on_one"})]
    );
    Ok(())
}

/// A change of language and a rename go to the hub's routes as the user
/// gave them, and the document the app reads again says what the hub did.
#[tokio::test]
async fn language_and_title_reach_the_hubs_routes() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _) = fixture(directory.path())?;
    let mut events = runtime.features.notes.test_events();
    let hub = paired(&mut runtime).await?;
    lock(&hub).documents.insert(
        RECORDING.into(),
        fake_hub::document(RECORDING, "Comité", START),
    );

    runtime
        .command(Command::MeetingLanguage {
            recording_id: RECORDING.into(),
            language: "en".into(),
        })
        .await?;
    answer(&mut runtime, &mut events, START).await?;
    assert_eq!(
        received(&hub, "PUT", "/language"),
        [json!({"language": "en"})]
    );
    let shown = document(&runtime).await?;
    assert_eq!(shown.language.as_deref(), Some("en"));
    assert_eq!(
        shown.transcript.retranscribing,
        Some(orbit_core::notes::Retranscribing {
            language: "en".into(),
            done: 0,
            total: 4,
        })
    );

    runtime
        .command(Command::MeetingRename {
            recording_id: RECORDING.into(),
            title: "  Presupuesto 2027 ".into(),
        })
        .await?;
    answer(&mut runtime, &mut events, START).await?;
    runtime
        .command(Command::MeetingRename {
            recording_id: RECORDING.into(),
            title: String::new(),
        })
        .await?;
    answer(&mut runtime, &mut events, START).await?;
    assert_eq!(
        received(&hub, "PUT", "/title"),
        [json!({"title": "Presupuesto 2027"}), json!({"title": ""})]
    );
    assert_eq!(document(&runtime).await?.title, "Comité");

    // The hub's refusal is said with the order and the hub's code.
    lock(&hub).failing = Some(409);
    runtime
        .command(Command::MeetingLanguage {
            recording_id: RECORDING.into(),
            language: "fr".into(),
        })
        .await?;
    answer(&mut runtime, &mut events, START).await?;
    let notice = serde_json::to_value(runtime.state.durable.notice.clone().context("no notice")?)?;
    assert_eq!(notice["code"], "note_change_refused");
    assert_eq!(notice["data"]["step"], "language");
    assert_eq!(notice["data"]["status"], 409);
    Ok(())
}

/// Audio the hub cannot read is said on screen with its cause and numbers,
/// never as the generic refusal of a note change.
#[tokio::test]
async fn unreadable_audio_is_said_with_how_many_groups_of_how_many() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _) = fixture(directory.path())?;
    let mut events = runtime.features.notes.test_events();
    let hub = paired(&mut runtime).await?;
    lock(&hub).documents.insert(
        RECORDING.into(),
        fake_hub::document(RECORDING, "Comité", START),
    );
    lock(&hub).unreadable_audio = Some((1, 4));
    runtime
        .command(Command::MeetingLanguage {
            recording_id: RECORDING.into(),
            language: "en".into(),
        })
        .await?;
    answer(&mut runtime, &mut events, START).await?;
    let notice = runtime.state.durable.notice.clone().context("no notice")?;
    assert_eq!(
        orbit_core::notice::NoticeCode::MeetingAudioUnreadable.class(),
        orbit_core::notice::NoticeClass::Fixable
    );
    let notice = serde_json::to_value(notice)?;
    assert_eq!(notice["code"], "meeting_audio_unreadable");
    assert_eq!(notice["data"]["count"], 1);
    assert_eq!(notice["data"]["total"], 4);
    Ok(())
}

/// What the hub would refuse is refused on the Mac at once, with its
/// numbers, and nothing is sent.
#[tokio::test]
async fn a_language_or_title_the_hub_refuses_is_refused_here() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _) = fixture(directory.path())?;
    let hub = paired(&mut runtime).await?;
    let language = runtime
        .command(Command::MeetingLanguage {
            recording_id: RECORDING.into(),
            language: "auto".into(),
        })
        .await;
    assert!(language.is_err_and(|error| matches!(
        error.downcast_ref::<orbit_core::notes::NoteCommandError>(),
        Some(orbit_core::notes::NoteCommandError::MeetingLanguage)
    )));
    let max = orbit_core::MAX_MANIFEST_TITLE_CHARACTERS;
    let title = runtime
        .command(Command::MeetingRename {
            recording_id: RECORDING.into(),
            title: "a".repeat(max + 1),
        })
        .await;
    assert!(title.is_err_and(|error| matches!(
        error.downcast_ref::<orbit_core::notes::NoteCommandError>(),
        Some(orbit_core::notes::NoteCommandError::MeetingTitleTooLong { characters, max_characters })
            if *characters == max + 1 && *max_characters == max
    )));
    assert!(lock(&hub).received.is_empty());
    Ok(())
}

/// A note order the hub refuses is said, with the hub's code. One given
/// while the hub cannot be reached is kept on this Mac, owed, and nothing
/// goes; one that is not sent twice and whose request found no hub is said
/// as not sent, with its step.
#[tokio::test]
async fn a_refused_or_unsent_order_is_said() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _) = fixture(directory.path())?;
    let mut events = runtime.features.notes.test_events();
    let hub = paired(&mut runtime).await?;
    lock(&hub).failing = Some(422);
    runtime
        .command(Command::NoteRegenerate {
            recording_id: RECORDING.into(),
            template_id: "general".into(),
        })
        .await?;
    answer(&mut runtime, &mut events, START).await?;
    let notice = runtime.state.durable.notice.clone().context("no notice")?;
    assert_eq!(
        notice.code,
        orbit_core::notice::NoticeCode::NoteChangeRefused
    );
    let data = serde_json::to_value(&notice)?;
    assert_eq!(data["data"]["status"], 422);
    assert_eq!(data["data"]["code"], "synthetic_failure");
    // The refused order left the outbox.
    assert!(runtime.reader.note_orders()?.is_empty());

    crate::daemon::authority_fixture::set(&mut runtime.state, "connection", json!("offline"));
    let sent_before = lock(&hub).received.len();
    runtime
        .command(Command::SpeakerRename {
            recording_id: RECORDING.into(),
            key: "remote_app".into(),
            name: Some("Lucía".into()),
            scope: orbit_core::notes::SpeakerRenameScope::Voice,
        })
        .await?;
    assert_eq!(lock(&hub).received.len(), sent_before);
    assert_eq!(runtime.reader.note_orders()?.len(), 1);

    // The hub's address answers nothing: a rewrite is said as not sent.
    let unused = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    runtime.settings.hub_url = Some(format!("http://{}", unused.local_addr()?));
    drop(unused);
    crate::daemon::authority_fixture::set(&mut runtime.state, "connection", json!("online"));
    runtime
        .command(Command::NoteRegenerate {
            recording_id: OLDER.into(),
            template_id: "general".into(),
        })
        .await?;
    answer(&mut runtime, &mut events, START).await?;
    let notice = serde_json::to_value(runtime.state.durable.notice.clone().context("no notice")?)?;
    assert_eq!(notice["code"], "note_change_unsent");
    assert_eq!(notice["data"]["step"], "write");
    Ok(())
}

/// Without a queue: a template order goes to the hub as the user gives
/// it, one at a time, and a refused one is shown by its target.
#[tokio::test]
async fn template_orders_go_to_the_hub_and_a_refusal_is_shown() -> Result<()> {
    use orbit_core::notes::{LibraryItem, LibraryRequest, TemplateDraft, TemplateSection};
    let directory = tempfile::tempdir()?;
    let (mut runtime, _) = fixture(directory.path())?;
    let mut events = runtime.features.notes.test_events();
    let hub = paired(&mut runtime).await?;
    let draft = TemplateDraft {
        name: "Comité".into(),
        context: "Inversiones".into(),
        sections: vec![TemplateSection {
            name: "Propuestas".into(),
            instructions: "Con importe".into(),
        }],
        forked_from: None,
    };
    runtime
        .command(Command::NoteLibrary {
            request: LibraryRequest::SaveTemplate {
                template_id: "mia".into(),
                draft: draft.clone(),
            },
        })
        .await?;
    // A second save of the same template waits behind the first.
    runtime
        .command(Command::NoteLibrary {
            request: LibraryRequest::SaveTemplate {
                template_id: "mia".into(),
                draft: TemplateDraft {
                    name: "Comité de inversión".into(),
                    ..draft.clone()
                },
            },
        })
        .await?;
    assert!(runtime.library_state().contains(&LibraryItem::Pending {
        target: "template:mia".into()
    }));
    answer(&mut runtime, &mut events, START).await?;
    turn(&mut runtime, &mut events, START).await?;
    let saved = received(&hub, "PUT", "/v1/note-templates/mia");
    assert_eq!(saved.len(), 2);
    assert_eq!(saved[1]["name"], "Comité de inversión");
    let page = read(&runtime, ReadRequest::NoteLibrary { cursor: None }).await?;
    assert!(
        page.items
            .iter()
            .any(|item| item["item"] == "template" && item["id"] == "mia")
    );

    lock(&hub).failing = Some(409);
    runtime
        .command(Command::NoteLibrary {
            request: LibraryRequest::DeleteTemplate {
                template_id: "mia".into(),
            },
        })
        .await?;
    answer(&mut runtime, &mut events, START).await?;
    assert!(runtime.library_state().iter().any(|item| matches!(item,
        LibraryItem::Refused(refusal) if refusal.target_id == "mia"
            && refusal.action == "delete" && refusal.status == 409)));
    Ok(())
}

/// The account's name labels the user's microphone on the hub: read from the
/// program that prints it, «Yo» when it fails or is late.
#[tokio::test]
async fn the_account_name_is_read_with_a_deadline_and_says_yo_when_late() -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let directory = tempfile::tempdir()?;
    let script = |name: &str, body: &str| -> Result<std::path::PathBuf> {
        let path = directory.path().join(name);
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n"))?;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))?;
        Ok(path)
    };
    let fast = script("fast-id", "echo 'Ada Lovelace,,,'")?;
    let slow = script("slow-id", "sleep 30; echo 'Nunca'")?;
    let failing = script("failing-id", "exit 1")?;
    // The programs that finish are not raced against the clock: macOS assesses
    // a freshly written executable on its first exec, measured at 108–297 ms
    // on an idle Mac (11–17 ms after), so a short deadline decided the result.
    // The call returns when the program exits; this bound is never reached.
    let unhurried = std::time::Duration::from_secs(60);
    assert_eq!(
        super::read_account_name(&fast, unhurried).await,
        "Ada Lovelace"
    );
    assert_eq!(super::read_account_name(&failing, unhurried).await, "Yo");
    // Had it waited for the program, the name would be «Nunca».
    let deadline = std::time::Duration::from_millis(300);
    assert_eq!(super::read_account_name(&slow, deadline).await, "Yo");
    Ok(())
}

/// A privacy stop: nothing goes to the hub, the reads say why, and what is
/// owed stays in the store for the rearm.
#[tokio::test]
async fn a_privacy_stop_sends_nothing_and_keeps_what_is_owed() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _) = fixture(directory.path())?;
    let mut events = runtime.features.notes.test_events();
    let hub = paired(&mut runtime).await?;
    runtime
        .command(Command::NotePrivateSave {
            recording_id: RECORDING.into(),
            markdown: "- antes de parar".into(),
            base_revision: 0,
        })
        .await?;
    crate::daemon::authority_fixture::set(&mut runtime.state, "privacy_stopped", json!(true));
    runtime.retire_recording_notes();
    assert_eq!(turn(&mut runtime, &mut events, START).await?, 0);
    assert!(received(&hub, "PUT", "/private-notes").is_empty());
    let answer = runtime
        .notes_read(&ReadRequest::NoteDetail {
            id: OLDER.into(),
            cursor: None,
        })
        .context("a notes read")?
        .await;
    assert!(matches!(
        answer,
        Knowledge::Absent {
            refusal: orbit_core::refusal::Refusal::PrivacyStopped { .. }
        }
    ));
    assert_eq!(
        runtime.reader.pending_private_notes()?,
        vec![RECORDING.to_owned()]
    );
    Ok(())
}

#[tokio::test]
async fn the_linked_events_invitees_reach_the_note_read() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _) = fixture(directory.path())?;
    let hub = paired(&mut runtime).await?;
    lock(&hub).documents.insert(
        RECORDING.into(),
        fake_hub::document(RECORDING, "Comité", START),
    );
    let attendees = json!([
        {"name": "Alina", "email": "alina@example.invalid", "is_self": true},
        {"name": "Lucía", "email": "lucia@example.invalid", "is_self": false}
    ]);
    runtime.fixture_store.enqueue_job(&Job {
        id: format!("recording-{RECORDING}"),
        kind: "recording_manifest".into(),
        state: JobState::Succeeded,
        created_at: START,
        payload: json!({"recording_id": RECORDING, "event": {
            "title": "Comité", "calendar_item_id": "series", "occurrence_start_ms": START,
            "starts_at_ms": START, "ends_at_ms": START + 120000, "attendees": attendees
        }}),
    })?;
    let document = serde_json::to_value(document(&runtime).await?)?;
    assert_eq!(document["event"]["attendees"], attendees);
    Ok(())
}

/// Transport only: the server's real database tests cover applying/merging.
#[tokio::test]
async fn a_rename_carries_the_selected_turn_or_whole_voice_to_http() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _) = fixture(directory.path())?;
    let mut events = runtime.features.notes.test_events();
    let hub = paired(&mut runtime).await?;
    lock(&hub).documents.insert(
        RECORDING.into(),
        fake_hub::document(RECORDING, "Comité", START),
    );
    for scope in [
        json!({"kind": "turn", "at_ms": 12000}),
        json!({"kind": "voice"}),
    ] {
        let command = serde_json::from_value(json!({
            "command": "speaker_rename", "recording_id": RECORDING,
            "key": "remote_app", "name": "Lucía", "scope": scope
        }))?;
        runtime.command(command).await?;
        answer(&mut runtime, &mut events, START).await?;
    }
    assert_eq!(
        received(&hub, "PUT", "/speakers"),
        [
            json!({"names": {"remote_app": "Lucía"}, "turn": {"at_ms": 12000, "speaker_key": "remote_app"}}),
            json!({"names": {"remote_app": "Lucía"}}),
        ]
    );
    Ok(())
}

#[tokio::test]
async fn a_missing_turn_rename_keeps_the_hubs_code_and_numbers() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _) = fixture(directory.path())?;
    runtime.report_log = Some(orbit_ipc::report::ReportLog::daemon(directory.path()));
    let mut events = runtime.features.notes.test_events();
    let hub = paired(&mut runtime).await?;
    lock(&hub).missing_turn = Some((12_000, 3));
    runtime
        .command(Command::SpeakerRename {
            recording_id: RECORDING.into(),
            key: "remote_app".into(),
            name: Some("Lucía".into()),
            scope: orbit_core::notes::SpeakerRenameScope::Turn { at_ms: 12_000 },
        })
        .await?;
    answer(&mut runtime, &mut events, START).await?;
    assert_eq!(
        received(&hub, "PUT", "/speakers"),
        [json!({
        "names":{"remote_app":"Lucía"},"turn":{"at_ms":12000,"speaker_key":"remote_app"}})]
    );
    let notice = serde_json::to_value(runtime.state.durable.notice.as_ref().context("no notice")?)?;
    assert_eq!(notice["data"]["at_ms"], 12_000);
    assert_eq!(notice["data"]["turns"], 3);
    assert_eq!(notice["code"], "turn_no_longer_exists");
    assert_eq!(notice["data"]["code"], "turn_no_longer_exists");
    assert_eq!(notice["data"]["status"], 409);
    assert_eq!(notice["data"]["step"], "rename");
    let log = std::fs::read_to_string(runtime.report_log.as_ref().context("no log")?.path())?;
    let report: Value = serde_json::from_str(log.lines().last().context("empty log")?)?;
    assert_eq!(report["data"], notice["data"]);
    assert_eq!(report["code"], notice["code"]);
    assert!(!log.contains("Lucía"));
    Ok(())
}

const SERIES: &str = "serie-comité 1";

async fn series_previous(runtime: &Runtime) -> Knowledge<Page<Value>> {
    series_previous_titled(runtime, None).await
}

async fn series_previous_titled(
    runtime: &Runtime,
    title_digest: Option<String>,
) -> Knowledge<Page<Value>> {
    let request = ReadRequest::SeriesPrevious {
        calendar_item_id: SERIES.into(),
        before_ms: START,
        title_digest,
        cursor: None,
    };
    match runtime.notes_read(&request) {
        Some(future) => future.await,
        None => Knowledge::Unknown {
            why: orbit_core::topics::Pending::NotObservedYet,
        },
    }
}

/// «Pendiente de la última vez»: the earlier meeting of the next meeting's
/// series, with its tasks done or not, as the hub gives it; the hub's «none»
/// is a fact, and a hub without the route is not «none».
#[tokio::test]
async fn the_series_earlier_meeting_passes_through_and_an_older_hub_is_not_none() -> Result<()> {
    use orbit_core::notes::{NoteState, SeriesPrevious};
    let directory = tempfile::tempdir()?;
    let (mut runtime, _) = fixture(directory.path())?;
    let hub = paired(&mut runtime).await?;

    let Knowledge::Present { value: page, .. } = series_previous(&runtime).await else {
        bail!("a series without an earlier meeting was not answered");
    };
    let none: SeriesPrevious = serde_json::from_value(page.items[0].clone())?;
    assert_eq!(none.previous, None);
    assert_eq!(
        (none.calendar_item_id.as_str(), none.before_ms),
        (SERIES, START)
    );
    let asked = lock(&hub)
        .received
        .last()
        .map(|request| request.query.clone());
    assert!(
        asked.is_some_and(|query| query.contains("before_ms=1800000000000")
            && query.contains("calendar_item_id=serie-comit%C3%A9+1"))
    );

    let note = fake_hub::note(7);
    lock(&hub).series_previous.insert(
        SERIES.into(),
        json!({"recording_id": OLDER, "title": "Comité", "started_at_ms": START - 604_800_000,
            "note": {"state": "ready", "revision": 7, "short_summary": "Resumen corto.",
                "action_items": note["note"]["action_items"]}}),
    );
    let Knowledge::Present { value: page, .. } = series_previous(&runtime).await else {
        bail!("the earlier meeting was not answered");
    };
    let found: SeriesPrevious = serde_json::from_value(page.items[0].clone())?;
    let meeting = found.previous.context("the earlier meeting")?;
    assert_eq!(meeting.recording_id, OLDER);
    let note = meeting.note.context("its note")?;
    assert_eq!((note.state, note.revision), (NoteState::Ready, 7));
    let items = note.action_items.context("its tasks")?;
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].owner.as_deref(), Some("Nerea"));
    assert!(!items[0].done);

    lock(&hub).without_series_routes = true;
    match series_previous(&runtime).await {
        Knowledge::Absent {
            refusal:
                orbit_core::refusal::Refusal::UpstreamFailed {
                    status: Some(404), ..
                },
        } => {}
        other => bail!("an older hub was read as {other:?}"),
    }
    Ok(())
}

/// The next meeting's title goes to the hub as its digest, never as text, so
/// the hub can fall back to the earlier meeting of the same title; a read
/// without one asks what an older Mac asked.
#[tokio::test]
async fn the_series_read_names_the_title_by_its_digest_only() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _) = fixture(directory.path())?;
    let hub = paired(&mut runtime).await?;
    let digest = orbit_core::series_title::title_digest("Comité de dirección")
        .context("a title with words")?;
    let Knowledge::Present { .. } = series_previous_titled(&runtime, Some(digest.clone())).await
    else {
        bail!("the titled read was not answered");
    };
    let asked = lock(&hub)
        .received
        .last()
        .map(|request| request.query.clone())
        .unwrap_or_default();
    assert!(asked.contains(&format!("title_digest={digest}")), "{asked}");
    assert!(!asked.contains("Comit"), "{asked}");
    let Knowledge::Present { .. } = series_previous(&runtime).await else {
        bail!("the read without a title was not answered");
    };
    let asked = lock(&hub)
        .received
        .last()
        .map(|request| request.query.clone())
        .unwrap_or_default();
    assert!(!asked.contains("title_digest"), "{asked}");
    Ok(())
}

/// «Usar lo mismo en toda la serie»: one order to the hub's one route; done,
/// every read is read again (the series' dictionary can change transcripts);
/// refused, the refusal keeps its code and numbers; an older hub names itself.
#[tokio::test]
async fn the_series_template_and_dictionary_go_together_and_a_refusal_keeps_its_numbers()
-> Result<()> {
    use orbit_core::notes::{LibraryItem, LibraryRequest};
    let directory = tempfile::tempdir()?;
    let (mut runtime, _) = fixture(directory.path())?;
    let mut events = runtime.features.notes.test_events();
    let hub = paired(&mut runtime).await?;
    lock(&hub).documents.insert(
        RECORDING.into(),
        fake_hub::document(RECORDING, "Comité", START),
    );
    let order = || Command::NoteLibrary {
        request: LibraryRequest::UseForSeries {
            recording_id: RECORDING.into(),
            template_id: "project".into(),
            dictionary_id: Some("0d9a7c3e-2b1f-4e6a-9c8d-7f6e5d4c3b2a".into()),
            dictionary_ids: None,
        },
    };

    runtime.features.notes.head.touch_recording(OLDER);
    let library_before = runtime.features.notes.head.library_revision;
    runtime.command(order()).await?;
    assert!(runtime.library_state().contains(&LibraryItem::Pending {
        target: format!("meeting_series:{RECORDING}")
    }));
    answer(&mut runtime, &mut events, START).await?;
    assert_eq!(
        received(&hub, "PUT", &format!("/v1/recordings/{RECORDING}/series")),
        [json!({"template_id": "project",
            "dictionary_id": "0d9a7c3e-2b1f-4e6a-9c8d-7f6e5d4c3b2a"})]
    );
    let head = &runtime.features.notes.head;
    assert!(head.recordings.is_empty());
    assert_eq!(head.floor, head.revision);
    assert!(head.library_revision > library_before);
    assert!(runtime.library_state().is_empty());

    lock(&hub).series_refusal = Some((
        409,
        "note_template_links_full",
        json!({"links": 256, "max_links": 256}),
    ));
    runtime.command(order()).await?;
    answer(&mut runtime, &mut events, START).await?;
    assert!(runtime.library_state().iter().any(|item| matches!(item,
        LibraryItem::Refused(refusal) if refusal.target_kind == "meeting_series"
            && refusal.target_id == RECORDING && refusal.action == "use_for_series"
            && refusal.status == 409 && refusal.code == "note_template_links_full"
            && refusal.numbers.get("links") == Some(&256)
            && refusal.numbers.get("max_links") == Some(&256))));

    // Several dictionaries go as the whole list, for a hub that takes it.
    lock(&hub).series_refusal = None;
    runtime
        .command(Command::NoteLibrary {
            request: LibraryRequest::UseForSeries {
                recording_id: RECORDING.into(),
                template_id: "project".into(),
                dictionary_id: None,
                dictionary_ids: Some(vec![
                    "0d9a7c3e-2b1f-4e6a-9c8d-7f6e5d4c3b2a".into(),
                    "5e1b2c3d-4a5f-4b6c-8d7e-9f0a1b2c3d4e".into(),
                ]),
            },
        })
        .await?;
    answer(&mut runtime, &mut events, START).await?;
    assert_eq!(
        received(&hub, "PUT", &format!("/v1/recordings/{RECORDING}/series")).last(),
        Some(&json!({"template_id": "project", "dictionary_ids": [
            "0d9a7c3e-2b1f-4e6a-9c8d-7f6e5d4c3b2a",
            "5e1b2c3d-4a5f-4b6c-8d7e-9f0a1b2c3d4e"]}))
    );

    lock(&hub).without_series_routes = true;
    runtime.command(order()).await?;
    answer(&mut runtime, &mut events, START).await?;
    assert!(runtime.library_state().iter().any(|item| matches!(item,
        LibraryItem::Refused(refusal) if refusal.action == "use_for_series"
            && refusal.status == 404 && refusal.code == "hub_route_missing")));
    Ok(())
}
