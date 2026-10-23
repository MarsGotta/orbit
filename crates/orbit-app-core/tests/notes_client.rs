#![allow(clippy::expect_used, clippy::unwrap_used)]
//! At the app: the notes are read from the daemon as the hub computed
//! them, the list again when its head moves, and an open note again only when
//! the hub said that recording changed. The daemon here is a typed actor fixture; none of this is evidence about the real daemon or hub.

use orbit_app_core::{
    AppCommand, DaemonClient, DaemonConnection, LocalZone, NoteReadState, NotesIndexState,
    calendar::CalendarClock,
};
use orbit_core::{
    PROTOCOL_VERSION,
    protocol::{Command, Response, TopicResponse},
    reads::ReadRequest,
    topics::{Cursor, Knowledge, Page, TopicData, TopicFrame, TopicId, notes},
};
use orbit_ipc::{Paths, TopicChannels};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    error::Error,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};
use tokio::{
    sync::{mpsc, watch},
    task::JoinHandle,
};

const RECORDING_ID: &str = "11111111-1111-4111-8111-111111111111";
const OTHER_ID: &str = "33333333-3333-4333-8333-333333333333";
const EPOCH: &str = "notes-epoch";

struct FixedClock;
impl CalendarClock for FixedClock {
    fn now_ms(&self) -> i64 {
        1_790_508_000_000
    }
    fn local_zone(&self, _now_ms: i64) -> LocalZone {
        LocalZone::utc()
    }
    fn timezone_name(&self) -> Result<String, String> {
        Ok("UTC".into())
    }
}

fn head_frame(revision: u64, value: notes::Value) -> TopicFrame {
    TopicFrame {
        epoch: EPOCH.into(),
        revision,
        data: TopicData::Notes(Knowledge::Present {
            value,
            as_of_ms: None,
        }),
    }
}

/// What the fake hub's document says now: a speaker renamed, a transcript
/// still being written.
#[derive(Default)]
struct HubNow {
    renamed: AtomicBool,
    transcribing: AtomicBool,
    /// A change to English is transcribing the meeting again.
    retranscribing: AtomicBool,
    final_pass_completed: AtomicBool,
    dictionary: AtomicBool,
    dictionary_corrected: AtomicBool,
    /// The list's row: the hub is writing the note's summary.
    note_writing: AtomicBool,
    /// The list's row: the note's summary is written.
    note_ready: AtomicBool,
    /// The meeting was deleted: the hub answers that it has no such recording.
    deleted: AtomicBool,
}

fn document_now(now: &HubNow) -> Value {
    let mut document = document();
    if now.dictionary.load(Ordering::SeqCst) {
        document["dictionary"] =
            json!({"dictionary_id": "vocabulary", "source": "requested", "series": null});
    }
    if now.dictionary_corrected.load(Ordering::SeqCst) {
        document["title"] = json!("Diccionario cambiado");
    }
    if now.renamed.load(Ordering::SeqCst) {
        document["speakers"][1]["name"] = json!("Lucía");
    }
    if now.transcribing.load(Ordering::SeqCst) {
        document["transcript"] = json!({"state": "pending", "pending_chunks": 3,
            "failed_groups": []});
    }
    if now.retranscribing.load(Ordering::SeqCst) {
        document["language"] = json!("en");
        document["transcript"] = json!({"state": "pending", "pending_chunks": 3,
            "failed_groups": [], "retranscribing": {"language": "en", "done": 1, "total": 4}});
    }
    if now.final_pass_completed.load(Ordering::SeqCst) {
        document["transcript"]["final_pass"] = json!({"state":"completed", "error":null});
    }
    document
}

fn document() -> Value {
    json!({
        "recording_id": RECORDING_ID, "title": "Comité de dirección",
        "started_at_ms": 1_790_500_000_000_i64, "duration_ms": 120_000,
        "capture": "closed", "language": "es",
        "dictionary": {"dictionary_id": null, "source": null, "series": null},
        "speakers": [
            {"key": "mic_local", "track": "mic_local", "name": "Alina"},
            {"key": "remote_app", "track": "remote_app", "name": "Otra persona"}
        ],
        "transcript": {"state": "ready", "pending_chunks": 0, "failed_groups": []},
        "note": null,
        "private_notes": {"text": "", "revision": 0}
    })
}

fn row(id: &str, started_at_ms: i64) -> Value {
    json!({"recording_id": id, "title": format!("Reunión {id}"), "started_at_ms": started_at_ms,
        "duration_ms": 60_000, "capture": "closed", "transcript": "ready", "note": null})
}

/// The first meeting's row as the fake hub lists it now.
fn row_now(now: &HubNow) -> Value {
    let mut row = row(RECORDING_ID, 1_790_500_000_000);
    if now.note_writing.load(Ordering::SeqCst) {
        row["note"] = json!({"state": "generating"});
    }
    if now.note_ready.load(Ordering::SeqCst) {
        row["note"] = json!({"state": "ready", "short_summary": "Se aprobó el plan."});
    }
    row
}

struct Fixture {
    client: DaemonClient,
    notes_topic: watch::Sender<TopicFrame>,
    detail_reads: Arc<AtomicU64>,
    transcript_reads: Arc<AtomicU64>,
    hub_now: Arc<HubNow>,
    list_reads: Arc<AtomicU64>,
    library_reads: Arc<AtomicU64>,
    /// Moves with every read the fake answers: the test waits on it.
    reads_seen: watch::Receiver<u64>,
    /// Every order the app sent that is not a read or a subscription.
    orders: Arc<std::sync::Mutex<Vec<Command>>>,
    listener: fake_daemon::Endpoint,
    actor: JoinHandle<()>,
    _directory: tempfile::TempDir,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.listener.abort();
        self.actor.abort();
    }
}

fn answer(read: &ReadRequest, now: &HubNow) -> Knowledge<Page<Value>> {
    let page = |items: Vec<Value>, next: Option<Cursor>| Knowledge::Present {
        value: Page {
            total: items.len() as u64,
            items,
            next,
            revision: 0,
            store_version: None,
        },
        as_of_ms: None,
    };
    let gone = || Knowledge::Absent {
        refusal: orbit_core::refusal::Refusal::NotFound {
            resource: orbit_core::refusal::Resource::Recording,
        },
    };
    match read {
        ReadRequest::NoteDetail { .. } | ReadRequest::NoteTranscript { .. }
            if now.deleted.load(Ordering::SeqCst) =>
        {
            gone()
        }
        ReadRequest::NotesPage { cursor: None } => page(
            vec![row_now(now), row(OTHER_ID, 1_790_400_000_000)],
            Some(Cursor::DescendingDateAndId {
                date_ms: 1_790_400_000_000,
                id: OTHER_ID.into(),
            }),
        ),
        ReadRequest::NotesPage { cursor: Some(_) } => page(
            vec![row(
                "44444444-4444-4444-8444-444444444444",
                1_790_300_000_000,
            )],
            None,
        ),
        ReadRequest::NoteDetail { .. } => page(vec![document_now(now)], None),
        ReadRequest::NoteTranscript { cursor: None, .. } => page(
            vec![
                json!({"at_ms": 0, "speaker_key": "mic_local", "text": "Empezamos."}),
                json!({"at_ms": 12_000, "speaker_key": "remote_app", "text": "Vale."}),
            ],
            Some(Cursor::Index { index: 2 }),
        ),
        ReadRequest::NoteTranscript {
            cursor: Some(_), ..
        } => page(
            vec![
                json!({"at_ms": 61_000, "speaker_key": "mic_local", "text": if now.dictionary_corrected.load(Ordering::SeqCst) { "Corregido." } else if now.final_pass_completed.load(Ordering::SeqCst) { "Texto nuevo." } else { "Siguiente." }}),
            ],
            None,
        ),
        ReadRequest::NoteLibrary { .. } => page(Vec::new(), None),
        _ => page(Vec::new(), None),
    }
}

async fn fixture() -> Result<Fixture, Box<dyn Error>> {
    fixture_with(HubNow::default()).await
}

async fn fixture_with(hub_now: HubNow) -> Result<Fixture, Box<dyn Error>> {
    let directory = tempfile::Builder::new()
        .prefix("orbit-notes-")
        .tempdir_in("/tmp")?;
    let paths = Paths::resolve(Some(directory.path().to_path_buf()))?;
    let (calls, mut actor) = mpsc::channel(16);
    let mut topic_channels = TopicChannels::new();
    let (notes_topic, _) = watch::channel(head_frame(1, notes::Value::default()));
    topic_channels.insert(TopicId::Notes, notes_topic.clone());
    for id in TopicId::ALL {
        if *id != TopicId::Notes {
            let (sender, _) = watch::channel(TopicFrame {
                epoch: EPOCH.into(),
                revision: 1,
                data: TopicData::not_observed(*id),
            });
            topic_channels.insert(*id, sender);
        }
    }
    let listener = fake_daemon::channel(calls, topic_channels);
    let detail_reads = Arc::new(AtomicU64::new(0));
    let transcript_reads = Arc::new(AtomicU64::new(0));
    let hub_now = Arc::new(hub_now);
    let list_reads = Arc::new(AtomicU64::new(0));
    let (reads_tx, reads_seen) = watch::channel(0_u64);
    let library_reads = Arc::new(AtomicU64::new(0));
    let (actor_details, actor_lists, actor_libraries) = (
        Arc::clone(&detail_reads),
        Arc::clone(&list_reads),
        Arc::clone(&library_reads),
    );
    let (actor_transcripts, actor_now) = (Arc::clone(&transcript_reads), Arc::clone(&hub_now));
    let orders = Arc::new(std::sync::Mutex::new(Vec::new()));
    let actor_orders = Arc::clone(&orders);
    let actor = tokio::spawn(async move {
        while let Some(call) = actor.recv().await {
            match call.request.body {
                Command::SubscribeTopics { .. } => {
                    let response = TopicResponse {
                        v: PROTOCOL_VERSION,
                        id: call.request.id,
                        ok: true,
                        revisions: BTreeMap::from([(TopicId::Notes, 1)]),
                        read: None,
                        answer: None,
                        refusal: None,
                    };
                    if call.reply.send(response).is_err() {
                        eprintln!("fixture topic subscriber ended before handshake");
                    }
                }
                Command::Read { read } => {
                    match read {
                        ReadRequest::NoteDetail { .. } => {
                            actor_details.fetch_add(1, Ordering::SeqCst);
                        }
                        ReadRequest::NotesPage { cursor: None } => {
                            actor_lists.fetch_add(1, Ordering::SeqCst);
                        }
                        ReadRequest::NoteLibrary { .. } => {
                            actor_libraries.fetch_add(1, Ordering::SeqCst);
                        }
                        ReadRequest::NoteTranscript { .. } => {
                            actor_transcripts.fetch_add(1, Ordering::SeqCst);
                        }
                        _ => {}
                    }
                    let response = TopicResponse {
                        v: PROTOCOL_VERSION,
                        id: call.request.id,
                        ok: true,
                        revisions: BTreeMap::new(),
                        read: Some(read.id()),
                        answer: Some(answer(&read, &actor_now)),
                        refusal: None,
                    };
                    if call.reply.send(response).is_err() {
                        eprintln!("fixture read reply had no receiver");
                    }
                    reads_tx.send_modify(|reads| *reads += 1);
                }
                order => {
                    if let Ok(mut orders) = actor_orders.lock() {
                        orders.push(order);
                    }
                    let response = Response {
                        refusal: None,
                        v: PROTOCOL_VERSION,
                        id: call.request.id,
                        ok: true,
                        status: None,
                        error: None,
                        epoch: Some(EPOCH.into()),
                        revisions: Some(BTreeMap::new()),
                    };
                    if call.reply.send(response).is_err() {
                        eprintln!("fixture command reply had no receiver");
                    }
                }
            }
        }
    });
    let client =
        DaemonClient::connect_with_clock(paths, Arc::new(FixedClock), listener.client.clone());
    tokio::time::timeout(Duration::from_secs(5), async {
        let mut view = client.watch_frames();
        while client.composed_snapshot().connection.daemon != DaemonConnection::Ready {
            view.changed().await?;
        }
        Ok::<(), watch::error::RecvError>(())
    })
    .await??;
    Ok(Fixture {
        client,
        notes_topic,
        detail_reads,
        transcript_reads,
        hub_now,
        list_reads,
        library_reads,
        reads_seen,
        orders,
        listener,
        actor,
        _directory: directory,
    })
}

/// Waits on the fake daemon's reads until `ready`; five seconds only bound a
/// hang.
async fn reads_until(fixture: &Fixture, ready: impl Fn() -> bool) -> Result<(), Box<dyn Error>> {
    let mut seen = fixture.reads_seen.clone();
    tokio::time::timeout(Duration::from_secs(5), seen.wait_for(|_| ready())).await??;
    Ok(())
}

/// Waits on the app's frames for `ready`; the five seconds only bound a hang.
async fn until(
    client: &DaemonClient,
    ready: impl Fn(&orbit_app_core::AppSnapshot) -> bool,
) -> Result<orbit_app_core::AppSnapshot, Box<dyn Error>> {
    let mut frames = client.watch_frames();
    let snapshot = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let snapshot = client.composed_snapshot();
            if ready(&snapshot) {
                return Ok::<_, watch::error::RecvError>(snapshot);
            }
            frames.changed().await?;
        }
    })
    .await??;
    Ok(snapshot)
}

async fn open(fixture: &Fixture) -> Result<(), Box<dyn Error>> {
    fixture
        .client
        .send(AppCommand::Note(
            orbit_app_core::features::note::NoteCommand::NotesOpen {
                recording_id: RECORDING_ID.into(),
            },
        ))
        .await?;
    Ok(())
}

/// Defects 1–4 through the app's client: the open note carries the hub's
/// times from the start of the recording, its speakers' names, its turns
/// (all pages) and its language.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_open_note_is_the_hubs_document_and_turns() -> Result<(), Box<dyn Error>> {
    let fixture = fixture().await?;
    open(&fixture).await?;
    let snapshot = fixture.client.composed_snapshot();
    let notes = snapshot.notes.unwrap();
    assert_eq!(notes.open_read.unwrap().state, NoteReadState::Available);
    let note = notes.open.unwrap();
    assert_eq!(note.title, "Comité de dirección");
    assert_eq!(note.transcript.language.as_deref(), Some("es"));
    assert_eq!(
        note.transcript
            .turns
            .iter()
            .map(|turn| turn.at_ms)
            .collect::<Vec<_>>(),
        [0, 12_000, 61_000]
    );
    assert_eq!(note.speakers[0].name, "Alina");
    assert_eq!(note.owner_name, "Alina");
    assert!(
        note.transcript
            .turns
            .iter()
            .all(|turn| !turn.text.is_empty())
    );
    Ok(())
}

/// An open note is read again when the hub says it changed, and only then:
/// another recording's change and a library change do not read it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_open_note_is_read_again_by_its_recording_changed() -> Result<(), Box<dyn Error>> {
    let fixture = fixture().await?;
    open(&fixture).await?;
    assert_eq!(fixture.detail_reads.load(Ordering::SeqCst), 1);

    // Another recording changed, then this one, then only the library. The
    // app reads the head's changes one after another, so once the library
    // read of the last one arrived, the two before it were handled.
    let mut head = notes::Value::default();
    head.touch_recording(OTHER_ID);
    let lists = fixture.list_reads.load(Ordering::SeqCst);
    fixture
        .notes_topic
        .send_replace(head_frame(2, head.clone()));
    reads_until(&fixture, || {
        fixture.list_reads.load(Ordering::SeqCst) > lists
    })
    .await?;
    head.touch_recording(RECORDING_ID);
    fixture
        .notes_topic
        .send_replace(head_frame(3, head.clone()));
    reads_until(&fixture, || {
        fixture.detail_reads.load(Ordering::SeqCst) >= 2
    })
    .await?;
    let libraries = fixture.library_reads.load(Ordering::SeqCst);
    head.touch_library();
    fixture.notes_topic.send_replace(head_frame(4, head));
    reads_until(&fixture, || {
        fixture.library_reads.load(Ordering::SeqCst) > libraries
    })
    .await?;
    assert_eq!(
        fixture.detail_reads.load(Ordering::SeqCst),
        2,
        "read again once, for its own change; never for another's or the library's"
    );
    Ok(())
}

/// A speaker renamed on a meeting whose transcript is complete: the names
/// come with the document, so the note is read again without its turns.
/// While the transcript is still being written, a change reads the turns too.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_rename_reads_the_document_and_keeps_the_complete_turns() -> Result<(), Box<dyn Error>> {
    let fixture = fixture().await?;
    open(&fixture).await?;
    let turns = fixture.transcript_reads.load(Ordering::SeqCst);
    assert_eq!(turns, 2, "the open note read both pages of turns");

    fixture.hub_now.renamed.store(true, Ordering::SeqCst);
    let mut head = notes::Value::default();
    head.touch_recording(RECORDING_ID);
    fixture
        .notes_topic
        .send_replace(head_frame(2, head.clone()));
    let renamed = until(&fixture.client, |snapshot| {
        snapshot
            .notes
            .as_ref()
            .and_then(|notes| notes.open.as_ref())
            .is_some_and(|note| note.speakers.iter().any(|speaker| speaker.name == "Lucía"))
    })
    .await?;
    assert_eq!(fixture.transcript_reads.load(Ordering::SeqCst), turns);
    assert_eq!(
        renamed.notes.unwrap().open.unwrap().transcript.turns.len(),
        3
    );

    fixture.hub_now.transcribing.store(true, Ordering::SeqCst);
    head.touch_recording(RECORDING_ID);
    fixture.notes_topic.send_replace(head_frame(3, head));
    reads_until(&fixture, || {
        fixture.transcript_reads.load(Ordering::SeqCst) >= turns + 2
    })
    .await?;
    Ok(())
}

/// At the app: the open note says how far a change of language got,
/// and the header's two orders reach the daemon for that meeting.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_repetition_is_shown_and_the_headers_orders_reach_the_daemon()
-> Result<(), Box<dyn Error>> {
    let fixture = fixture().await?;
    fixture.hub_now.retranscribing.store(true, Ordering::SeqCst);
    open(&fixture).await?;
    let note = fixture
        .client
        .composed_snapshot()
        .notes
        .unwrap()
        .open
        .unwrap();
    assert_eq!(note.transcript.language.as_deref(), Some("en"));
    let repetition = note.transcript.retranscribing.unwrap();
    assert_eq!(
        (
            repetition.language.as_str(),
            repetition.done,
            repetition.total
        ),
        ("en", 1, 4)
    );
    for command in [
        Command::MeetingLanguage {
            recording_id: RECORDING_ID.into(),
            language: "fr".into(),
        },
        Command::MeetingRename {
            recording_id: RECORDING_ID.into(),
            title: "Presupuesto 2027".into(),
        },
    ] {
        fixture
            .client
            .send(AppCommand::Daemon(Box::new(command)))
            .await?;
    }
    let orders = fixture.orders.lock().unwrap().clone();
    assert!(orders.iter().any(|order| matches!(order,
        Command::MeetingLanguage { recording_id, language }
            if recording_id == RECORDING_ID && language == "fr")));
    assert!(orders.iter().any(|order| matches!(order,
        Command::MeetingRename { recording_id, title }
            if recording_id == RECORDING_ID && title == "Presupuesto 2027")));
    Ok(())
}

/// The list: read when the head moves, one page more with «Cargar más».
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_list_follows_its_head_and_loads_more() -> Result<(), Box<dyn Error>> {
    let fixture = fixture().await?;
    let first = until(&fixture.client, |snapshot| {
        snapshot
            .notes
            .as_ref()
            .is_some_and(|notes| notes.list.len() == 2)
    })
    .await?;
    let notes = first.notes.unwrap();
    assert_eq!(notes.list[0].recording_id, RECORDING_ID);
    assert_eq!(notes.list[0].title, format!("Reunión {RECORDING_ID}"));
    let index = notes.index.unwrap();
    assert_eq!(index.state, NotesIndexState::Available);
    assert!(index.more);
    fixture
        .client
        .send(AppCommand::Notes(
            orbit_app_core::features::notes::NotesCommand::NotesMore,
        ))
        .await?;
    let more = fixture.client.composed_snapshot().notes.unwrap();
    assert_eq!(more.list.len(), 3);
    assert!(!more.index.unwrap().more);
    Ok(())
}

/// «Tu nota está lista»: a note seen being written and then read ready
/// is said once, by its recording and title, apart from the recording's end.
/// Read again unchanged it is not said again; and an app that starts with
/// the note already written (a restart) says nothing about it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_note_seen_being_written_says_once_that_it_is_ready() -> Result<(), Box<dyn Error>> {
    let fixture = fixture().await?;
    let listed = until(&fixture.client, |snapshot| {
        snapshot
            .notes
            .as_ref()
            .is_some_and(|notes| notes.list.len() == 2)
    })
    .await?;
    assert_eq!(listed.notes.and_then(|notes| notes.ready), None);

    let mut head = notes::Value::default();
    fixture.hub_now.note_writing.store(true, Ordering::SeqCst);
    head.touch_recording(RECORDING_ID);
    fixture
        .notes_topic
        .send_replace(head_frame(2, head.clone()));
    let writing = until(&fixture.client, |snapshot| {
        snapshot.notes.as_ref().is_some_and(|notes| {
            notes.list.first().and_then(|item| item.headline.as_ref())
                == Some(&orbit_app_core::NoteHeadlineSnapshot::Writing {})
        })
    })
    .await?;
    assert_eq!(writing.notes.and_then(|notes| notes.ready), None);

    fixture.hub_now.note_writing.store(false, Ordering::SeqCst);
    fixture.hub_now.note_ready.store(true, Ordering::SeqCst);
    head.touch_recording(RECORDING_ID);
    fixture
        .notes_topic
        .send_replace(head_frame(3, head.clone()));
    let ready = until(&fixture.client, |snapshot| {
        snapshot
            .notes
            .as_ref()
            .is_some_and(|notes| notes.ready.is_some())
    })
    .await?;
    let expected = orbit_app_core::NoteReadySnapshot {
        recording_id: RECORDING_ID.into(),
        title: format!("Reunión {RECORDING_ID}"),
        sequence: 1,
    };
    assert_eq!(
        ready.notes.and_then(|notes| notes.ready),
        Some(expected.clone())
    );

    // The list and the library move together: the app reads the list
    // first, so once the library read arrives the list read was applied.
    let (lists, libraries) = (
        fixture.list_reads.load(Ordering::SeqCst),
        fixture.library_reads.load(Ordering::SeqCst),
    );
    head.touch();
    head.touch_library();
    fixture.notes_topic.send_replace(head_frame(4, head));
    reads_until(&fixture, || {
        fixture.library_reads.load(Ordering::SeqCst) > libraries
    })
    .await?;
    assert!(fixture.list_reads.load(Ordering::SeqCst) > lists);
    assert_eq!(
        fixture
            .client
            .composed_snapshot()
            .notes
            .and_then(|notes| notes.ready),
        Some(expected),
        "read again, the same note is not said again"
    );
    drop(fixture);

    let restarted = fixture_with(HubNow {
        note_ready: AtomicBool::new(true),
        ..HubNow::default()
    })
    .await?;
    let listed = until(&restarted.client, |snapshot| {
        snapshot.notes.as_ref().is_some_and(|notes| {
            notes
                .list
                .first()
                .and_then(|item| item.headline.as_ref())
                .is_some()
        })
    })
    .await?;
    assert_eq!(
        listed.notes.and_then(|notes| notes.ready),
        None,
        "a note already written when the app starts is not news"
    );
    Ok(())
}

#[path = "support/fake_daemon.rs"]
mod fake_daemon;

/// The hub may finish between two app reads, so no pending state was seen.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_completed_final_pass_reads_replacement_turns_even_without_observing_pending()
-> Result<(), Box<dyn Error>> {
    let fixture = fixture().await?;
    open(&fixture).await?;
    fixture
        .hub_now
        .final_pass_completed
        .store(true, Ordering::SeqCst);
    let mut head = notes::Value::default();
    head.touch_recording(RECORDING_ID);
    fixture.notes_topic.send_replace(head_frame(2, head));
    let snapshot = until(&fixture.client, |snapshot| {
        snapshot
            .notes
            .as_ref()
            .and_then(|notes| notes.open.as_ref())
            .is_some_and(|note| note.transcript.final_pass.is_some())
    })
    .await?;
    let note = snapshot.notes.unwrap().open.unwrap();
    assert_eq!(note.transcript.turns.last().unwrap().text, "Texto nuevo.");
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dictionary_changes_reread_the_turns_of_an_already_complete_meeting()
-> Result<(), Box<dyn Error>> {
    let fixture = fixture().await?;
    fixture.hub_now.dictionary.store(true, Ordering::SeqCst);
    open(&fixture).await?;
    fixture
        .hub_now
        .dictionary_corrected
        .store(true, Ordering::SeqCst);
    let mut head = notes::Value::default();
    head.touch_recording(RECORDING_ID);
    fixture
        .notes_topic
        .send_replace(head_frame(2, head.clone()));
    let changed = until(&fixture.client, |snapshot| {
        snapshot
            .notes
            .as_ref()
            .and_then(|notes| notes.open.as_ref())
            .is_some_and(|note| note.title == "Diccionario cambiado")
    })
    .await?;
    let projected =
        serde_json::to_value(changed.notes.as_ref().and_then(|notes| notes.open.as_ref()))?;
    assert_eq!(
        projected["dictionary"],
        json!({"dictionary_id":"vocabulary", "source":"requested", "series":null})
    );
    assert_eq!(
        changed
            .notes
            .unwrap()
            .open
            .unwrap()
            .transcript
            .turns
            .last()
            .unwrap()
            .text,
        "Corregido."
    );
    fixture
        .hub_now
        .dictionary_corrected
        .store(false, Ordering::SeqCst);
    head.touch_recording(RECORDING_ID);
    fixture.notes_topic.send_replace(head_frame(3, head));
    let reverted = until(&fixture.client, |snapshot| {
        snapshot
            .notes
            .as_ref()
            .and_then(|notes| notes.open.as_ref())
            .is_some_and(|note| note.title == "Comité de dirección")
    })
    .await?;
    assert_eq!(
        reverted
            .notes
            .unwrap()
            .open
            .unwrap()
            .transcript
            .turns
            .last()
            .unwrap()
            .text,
        "Siguiente."
    );
    Ok(())
}

/// A meeting deleted on the hub (here, or on another Mac) while its note is
/// open: the read that follows its change says it is gone, not that it
/// failed. The note leaves, nothing reads it again, and opening it again
/// says the same without an error.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_meeting_deleted_on_the_hub_is_gone_not_a_failed_read() -> Result<(), Box<dyn Error>> {
    let fixture = fixture().await?;
    open(&fixture).await?;
    fixture.hub_now.deleted.store(true, Ordering::SeqCst);
    let mut head = notes::Value::default();
    head.touch_recording(RECORDING_ID);
    fixture
        .notes_topic
        .send_replace(head_frame(2, head.clone()));
    let gone = until(&fixture.client, |snapshot| {
        snapshot
            .notes
            .as_ref()
            .and_then(|notes| notes.open_read.as_ref())
            .is_some_and(|read| read.state == NoteReadState::Gone)
    })
    .await?;
    let notes = gone.notes.unwrap();
    assert!(
        notes.open.is_none(),
        "the deleted meeting's note is not kept"
    );
    let read = notes.open_read.unwrap();
    assert_eq!(read.recording_id, RECORDING_ID);
    assert_eq!(
        read.detail, None,
        "a meeting that is gone is no read problem"
    );

    // Another change of it reads nothing: there is no open note to read.
    let details = fixture.detail_reads.load(Ordering::SeqCst);
    let lists = fixture.list_reads.load(Ordering::SeqCst);
    head.touch_recording(RECORDING_ID);
    fixture.notes_topic.send_replace(head_frame(3, head));
    reads_until(&fixture, || {
        fixture.list_reads.load(Ordering::SeqCst) > lists
    })
    .await?;
    assert_eq!(fixture.detail_reads.load(Ordering::SeqCst), details);

    // Opened again (a link to it): gone, and not an error.
    open(&fixture).await?;
    let notes = fixture.client.composed_snapshot().notes.unwrap();
    assert!(notes.open.is_none());
    let read = notes.open_read.unwrap();
    assert_eq!((read.state, read.detail), (NoteReadState::Gone, None));
    Ok(())
}

/// «Borrar reunión» and «Terminar sin guardar» taken by the daemon: the
/// app leaves that meeting's note at once, before the hub deletes it, and
/// nothing reads it again; another meeting's open note stays.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_meeting_deleted_or_discarded_here_leaves_its_open_note() -> Result<(), Box<dyn Error>> {
    for order in [
        Command::MeetingDelete {
            recording_id: RECORDING_ID.into(),
        },
        Command::MeetingDiscard {
            recording_id: RECORDING_ID.into(),
        },
    ] {
        let fixture = fixture().await?;
        open(&fixture).await?;
        fixture
            .client
            .send(AppCommand::Daemon(Box::new(Command::MeetingDelete {
                recording_id: OTHER_ID.into(),
            })))
            .await?;
        assert!(
            fixture
                .client
                .composed_snapshot()
                .notes
                .unwrap()
                .open
                .is_some(),
            "another meeting's deletion leaves this note open"
        );
        fixture
            .client
            .send(AppCommand::Daemon(Box::new(order.clone())))
            .await?;
        assert!(fixture.orders.lock().unwrap().contains(&order));
        let notes = fixture.client.composed_snapshot().notes.unwrap();
        assert!(notes.open.is_none(), "{order:?} leaves the note");
        let read = notes.open_read.unwrap();
        assert_eq!((read.state, read.detail), (NoteReadState::Gone, None));

        let details = fixture.detail_reads.load(Ordering::SeqCst);
        let lists = fixture.list_reads.load(Ordering::SeqCst);
        let mut head = notes::Value::default();
        head.touch_recording(RECORDING_ID);
        fixture.notes_topic.send_replace(head_frame(2, head));
        reads_until(&fixture, || {
            fixture.list_reads.load(Ordering::SeqCst) > lists
        })
        .await?;
        assert_eq!(
            fixture.detail_reads.load(Ordering::SeqCst),
            details,
            "{order:?}: nothing reads the meeting again"
        );
    }
    Ok(())
}
