//! SIM-41: Mars's private notes wait in the store until the hub confirms
//! them; the notes an older Mac kept go up once; the rest of the Mac's copy of
//! the meetings leaves the store when it opens.
use orbit_core::notes::MAX_PRIVATE_NOTES_CHARS;
use orbit_store::{Error, Job, JobState, PendingPrivateNotes, Store};

const RECORDING: &str = "6f2b7d5a-9c41-4b7e-8d3f-1a2b3c4d5e6f";
const OTHER: &str = "11111111-1111-4111-8111-111111111111";
const KEY: [u8; 32] = [7; 32];

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

fn notes(recording: &str, markdown: &str, base_revision: u64) -> PendingPrivateNotes {
    PendingPrivateNotes {
        recording_id: recording.into(),
        markdown: markdown.into(),
        base_revision,
    }
}

#[test]
fn private_notes_wait_until_the_hub_confirms_exactly_that_text() -> Result<()> {
    let directory = tempfile::tempdir()?;
    {
        let mut store = Store::open(directory.path(), KEY)?;
        store.save_private_notes(&notes(RECORDING, "- **presupuesto**", 3), 1)?;
        store.save_private_notes(&notes(OTHER, "otra", 0), 2)?;
    }
    let mut store = Store::open(directory.path(), KEY)?;
    assert_eq!(
        store.pending_private_notes()?,
        vec![OTHER.to_owned(), RECORDING.to_owned()]
    );
    assert_eq!(
        store.private_notes(RECORDING)?,
        Some(notes(RECORDING, "- **presupuesto**", 3))
    );
    // Mars typed more while the first save was on its way: the older answer
    // does not take the newer text away.
    store.save_private_notes(&notes(RECORDING, "- **presupuesto** y plazos", 3), 3)?;
    assert!(!store.confirm_private_notes(RECORDING, "- **presupuesto**")?);
    assert!(store.private_notes(RECORDING)?.is_some());
    assert!(store.confirm_private_notes(RECORDING, "- **presupuesto** y plazos")?);
    assert_eq!(store.private_notes(RECORDING)?, None);
    assert_eq!(store.pending_private_notes()?, vec![OTHER.to_owned()]);
    Ok(())
}

#[test]
fn private_notes_over_the_hub_bound_are_refused_and_a_moved_row_does_not_authenticate() -> Result<()>
{
    let directory = tempfile::tempdir()?;
    let mut store = Store::open(directory.path(), KEY)?;
    let longest = "é".repeat(MAX_PRIVATE_NOTES_CHARS);
    store.save_private_notes(&notes(RECORDING, &longest, 0), 1)?;
    assert_eq!(
        store
            .private_notes(RECORDING)?
            .map(|stored| stored.markdown),
        Some(longest)
    );
    assert!(matches!(
        store.save_private_notes(
            &notes(RECORDING, &"a".repeat(MAX_PRIVATE_NOTES_CHARS + 1), 0),
            2
        ),
        Err(Error::NoteCommand(orbit_core::notes::NoteCommandError::PrivateNotesTooLong { characters, max_characters })) if characters == MAX_PRIVATE_NOTES_CHARS + 1 && max_characters == MAX_PRIVATE_NOTES_CHARS
    ));
    store.save_private_notes(&notes(OTHER, "otra", 0), 3)?;
    drop(store);
    let connection = rusqlite::Connection::open(directory.path().join("orbit.sqlite3"))?;
    connection.execute(
        "UPDATE records SET payload=(SELECT payload FROM records WHERE namespace='recording_notes' AND id=?1) \
         WHERE namespace='recording_notes' AND id=?2",
        [
            format!("note-private-{OTHER}"),
            format!("note-private-{RECORDING}"),
        ],
    )?;
    drop(connection);
    let store = Store::open(directory.path(), KEY)?;
    assert!(matches!(
        store.private_notes(RECORDING),
        Err(Error::Authentication)
    ));
    Ok(())
}

/// The rows an older Mac left: its private notes (their older shape is read
/// by the unit test in `recording_notes.rs`) and its copy of the meetings,
/// whose payloads are never read again, only removed.
fn older_mac_store(directory: &std::path::Path) -> Result<()> {
    let mut store = Store::open(directory, KEY)?;
    store.save_private_notes(&notes(RECORDING, "- escrito en la 0.4.0", 0), 1)?;
    for (id, kind) in [
        (
            "transcript-6f2b7d5a-9c41-4b7e-8d3f-1a2b3c4d5e6f",
            "transcript",
        ),
        (
            "fetch-6f2b7d5a-9c41-4b7e-8d3f-1a2b3c4d5e6f",
            "transcript_fetch",
        ),
        (
            "recording-6f2b7d5a-9c41-4b7e-8d3f-1a2b3c4d5e6f",
            "recording_manifest",
        ),
    ] {
        store.enqueue_job(&Job {
            id: id.into(),
            kind: kind.into(),
            payload: serde_json::json!({"recording_id": RECORDING}),
            state: JobState::Succeeded,
            created_at: 1,
        })?;
    }
    drop(store);
    let connection = rusqlite::Connection::open(directory.join("orbit.sqlite3"))?;
    for (namespace, id, kind) in [
        ("recording_notes", "note-sync-x", "recording_note_sync"),
        ("recording_notes", "note-head-x", "recording_note_head"),
        (
            "recording_notes",
            "note-templates",
            "recording_note_templates",
        ),
        (
            "recording_notes",
            "note-template-links",
            "recording_note_template_links",
        ),
        (
            "recording_notes",
            "note-library-outbox",
            "recording_note_library_outbox",
        ),
        ("speaker_review", "sr-x", "speaker_review_head"),
        ("meeting_archives", "ma-x", "meeting_archive"),
        ("meeting_sources", "ms-x", "meeting_source"),
        ("transcript_fetches", "tf-x", "transcript_fetch_retry"),
    ] {
        connection.execute(
            "INSERT INTO records(namespace,id,kind,state,created_at,payload) VALUES(?1,?2,?3,'succeeded',1,x'00')",
            [namespace, id, kind],
        )?;
    }
    for key in [
        "recording_notes_mark_v1",
        "speaker_review_usage_v1",
        "meeting_archive_usage_v1",
    ] {
        connection.execute("INSERT INTO settings(key,value) VALUES(?1,1)", [key])?;
    }
    Ok(())
}

#[test]
fn opening_removes_the_copy_of_the_meetings_and_keeps_the_private_notes_to_send() -> Result<()> {
    let directory = tempfile::tempdir()?;
    older_mac_store(directory.path())?;
    let store = Store::open(directory.path(), KEY)?;
    assert_eq!(store.pending_private_notes()?, vec![RECORDING.to_owned()]);
    assert_eq!(
        store.private_notes(RECORDING)?,
        Some(notes(RECORDING, "- escrito en la 0.4.0", 0))
    );
    assert!(
        store
            .get_job("recording-6f2b7d5a-9c41-4b7e-8d3f-1a2b3c4d5e6f")?
            .is_some()
    );
    assert!(
        store
            .get_job("transcript-6f2b7d5a-9c41-4b7e-8d3f-1a2b3c4d5e6f")?
            .is_none()
    );
    drop(store);
    let connection = rusqlite::Connection::open(directory.path().join("orbit.sqlite3"))?;
    let left: i64 = connection.query_row(
        "SELECT count(*) FROM records WHERE NOT (namespace='recording_notes' AND kind='recording_note_private')",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(left, 0);
    let jobs: i64 = connection.query_row(
        "SELECT count(*) FROM jobs WHERE kind IN ('transcript','transcript_fetch')",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(jobs, 0);
    let marks: i64 = connection.query_row(
        "SELECT count(*) FROM settings WHERE key IN ('recording_notes_mark_v1','speaker_review_usage_v1','meeting_archive_usage_v1')",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(marks, 0);
    // A store with nothing left to remove opens without writing.
    let before: i64 = connection.query_row("PRAGMA data_version", [], |row| row.get(0))?;
    drop(Store::open(directory.path(), KEY)?);
    let after: i64 = connection.query_row("PRAGMA data_version", [], |row| row.get(0))?;
    assert_eq!(before, after);
    Ok(())
}
