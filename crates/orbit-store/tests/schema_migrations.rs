//! Real SQLite migration and durability fixtures with synthetic encrypted data.
#[path = "support/integrity.rs"]
mod integrity_support;
use orbit_store::{Error, Job, JobState, Store};
use rusqlite::{Connection, ErrorCode, TransactionBehavior};
use serde_json::{Value, json};
use std::{
    error::Error as StdError,
    fs::{self, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};
type TestResult<T = ()> = Result<T, Box<dyn StdError>>;
const KEY: [u8; 32] = [0x42; 32];

fn database_path(directory: &Path) -> PathBuf {
    directory.join("orbit.sqlite3")
}

fn migration_history(directory: &Path) -> TestResult<Vec<(i64, i64, String)>> {
    let database = Connection::open(database_path(directory))?;
    let mut statement = database
        .prepare("SELECT version,applied_at,description FROM migrations ORDER BY version")?;
    Ok(statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
        .collect::<Result<Vec<_>, _>>()?)
}

fn private_directory(path: &Path) -> TestResult {
    fs::create_dir(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

fn copy_live_store(source: &Path, destination: &Path) -> TestResult {
    private_directory(destination)?;
    for name in ["chunks", "sources"] {
        private_directory(&destination.join(name))?;
    }
    for name in ["orbit.sqlite3", "orbit.sqlite3-wal", "orbit.sqlite3-shm"] {
        let source_file = source.join(name);
        if source_file.try_exists()? {
            fs::copy(source_file, destination.join(name))?;
        }
    }
    Ok(())
}

#[test]
fn succeeded_job_cannot_transition_back_to_queued() -> TestResult {
    let directory = tempfile::tempdir()?;
    let mut store = Store::open(directory.path(), KEY)?;
    let job = Job {
        id: "terminal-job".into(),
        kind: "chunk_upload".into(),
        state: JobState::Succeeded,
        created_at: 1,
        payload: json!({"fixture":"terminal"}),
    };
    store.enqueue_job(&job)?;

    assert!(store.update_job(&job.id, JobState::Queued).is_err());
    assert_eq!(
        store.get_job(&job.id)?.map(|job| job.state),
        Some(JobState::Succeeded)
    );
    Ok(())
}

#[test]
fn full_database_returns_sqlite_full_without_a_partial_job() -> TestResult {
    let directory = tempfile::tempdir()?;
    drop(Store::open(directory.path(), KEY)?);
    let mut database = Connection::open(database_path(directory.path()))?;
    let pages: i64 = database.pragma_query_value(None, "page_count", |row| row.get(0))?;
    let accepted: i64 =
        database.query_row(&format!("PRAGMA max_page_count={pages}"), [], |row| {
            row.get(0)
        })?;
    assert_eq!(accepted, pages);
    let transaction = database.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let error = transaction
        .execute(
            "INSERT INTO jobs(id,kind,state,created_at,payload) \
             VALUES('disk-full-job','chunk_upload','queued',1,zeroblob(?1))",
            [orbit_store::MAX_PAYLOAD_BYTES as i64],
        )
        .err()
        .ok_or("the page limit did not make SQLite report SQLITE_FULL")?;
    assert!(matches!(
        error.sqlite_error_code(),
        Some(ErrorCode::DiskFull)
    ));
    drop(transaction);
    assert_eq!(
        database.query_row::<i64, _, _>(
            "SELECT count(*) FROM jobs WHERE id='disk-full-job'",
            [],
            |row| row.get(0)
        )?,
        0
    );
    Ok(())
}

#[test]
fn live_wal_copy_reopens_only_the_last_committed_state() -> TestResult {
    let source = tempfile::tempdir()?;
    let mut store = Store::open(source.path(), KEY)?;
    store.save_snapshot(&json!({"revision":1}))?;
    let database = Connection::open(database_path(source.path()))?;
    database.execute_batch(
        "PRAGMA cache_size=1;
         PRAGMA cache_spill=ON;
         BEGIN IMMEDIATE;
         UPDATE settings SET value=zeroblob(length(value)) WHERE key='snapshot';
         WITH RECURSIVE numbers(value) AS (
             SELECT 1 UNION ALL SELECT value+1 FROM numbers WHERE value<64
         )
         INSERT INTO settings(key,value)
         SELECT printf('uncommitted-%03d',value),zeroblob(4096) FROM numbers;",
    )?;
    let wal = source.path().join("orbit.sqlite3-wal");
    assert!(
        fs::metadata(&wal)?.len() > 32,
        "fixture must contain WAL frames"
    );

    let copy_root = tempfile::tempdir()?;
    let copied = copy_root.path().join("copied-store");
    copy_live_store(source.path(), &copied)?;
    database.execute_batch("ROLLBACK;")?;
    drop(database);
    drop(store);

    let reopened = Store::open_existing(&copied, KEY)?;
    assert_eq!(
        reopened.load_snapshot::<Value>()?,
        Some(json!({"revision":1}))
    );
    integrity_support::assert_integrity(&copied)?;
    Ok(())
}

#[test]
fn corrupted_sqlite_bytes_are_rejected_on_reopen() -> TestResult {
    let directory = tempfile::tempdir()?;
    let mut store = Store::open(directory.path(), KEY)?;
    store.save_snapshot(&json!({"revision":1}))?;
    drop(store);
    let database = Connection::open(database_path(directory.path()))?;
    database.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")?;
    drop(database);

    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(database_path(directory.path()))?;
    let mut first = [0u8; 1];
    file.read_exact(&mut first)?;
    file.seek(SeekFrom::Start(0))?;
    file.write_all(&[first[0] ^ 0xff])?;
    file.sync_all()?;
    drop(file);

    assert!(Store::open_existing(directory.path(), KEY).is_err());
    Ok(())
}

// Frozen v3 tables: the fixture must not get its legacy schema from the
// migration being tested. Real encrypted audio, jobs and private notes are
// written through Store before adding the retired rows.
const LEGACY_TABLES: &str = "
CREATE TABLE IF NOT EXISTS action_journal(id TEXT PRIMARY KEY NOT NULL,state TEXT NOT NULL,payload BLOB NOT NULL);
CREATE TABLE IF NOT EXISTS calendar_copies(device_id TEXT NOT NULL,copy_id TEXT NOT NULL,control BLOB NOT NULL,payload BLOB,evidence BLOB NOT NULL,PRIMARY KEY(device_id,copy_id));
CREATE TABLE IF NOT EXISTS source_intents(id TEXT PRIMARY KEY NOT NULL,state TEXT NOT NULL CHECK(state IN ('prepared','stored','cancelled','failed','unknown')),reserved_bytes INTEGER NOT NULL CHECK(reserved_bytes>=0),payload BLOB NOT NULL);
CREATE TABLE IF NOT EXISTS sources(id TEXT PRIMARY KEY NOT NULL REFERENCES source_intents(id),bytes INTEGER NOT NULL CHECK(bytes>0),payload BLOB NOT NULL);
PRAGMA user_version=3;";
const RETIRED_NAMESPACES: [&str; 10] = [
    "app_capabilities",
    "meeting_calendar",
    "meeting_proposals",
    "meeting_synthesis",
    "native_actions",
    "people_identity",
    "proposals",
    "publications",
    "source_reading",
    "voice_memory",
];
const RETIRED_SETTINGS: [&str; 12] = [
    "action_count_v1",
    "meeting_calendar_registry_v1",
    "meeting_calendar_registry_revision_v1",
    "proposal_usage_v1",
    "publication_usage_v1",
    "ref208_notes_publication_generation_v1",
    "calendar_copies_registry_v1",
    "source_usage_v1",
    "ref19_history_job_generation_v1:recording_finalize",
    "ref19_history_job_generation_v1:recording_manifest",
    "ref19_history_job_generation_v1:transcript",
    "ref19_history_synthesis_generation_v1",
];

fn retired_v3_store() -> TestResult<tempfile::TempDir> {
    let directory = tempfile::tempdir()?;
    let mut store = Store::open(directory.path(), KEY)?;
    store.seal_chunk(
        &orbit_store::ChunkMetadata {
            id: "pending-audio".into(),
            recording_id: "meeting".into(),
            track: "mic_local".into(),
            capture_epoch_id: None,
            sequence: 0,
            sample_rate: 8000,
            channels: 1,
            frames: 4,
            monotonic_ns: 1000,
            first_sample: 0,
            started_at_ms: 1,
        },
        b"synthetic audio",
    )?;
    store.save_private_notes(
        &orbit_store::PendingPrivateNotes {
            recording_id: "meeting".into(),
            markdown: "nota privada pendiente".into(),
            base_revision: 2,
        },
        1,
    )?;
    store.save_snapshot(&json!({"privacy_stopped":true,"generation":9}))?;
    drop(store);
    let database = Connection::open(database_path(directory.path()))?;
    database.execute_batch(LEGACY_TABLES)?;
    for namespace in RETIRED_NAMESPACES {
        database.execute("INSERT INTO records(namespace,id,kind,state,created_at,payload) VALUES(?1,?1,'retired','succeeded',1,x'00')", [namespace])?;
    }
    for key in RETIRED_SETTINGS {
        database.execute(
            "INSERT INTO settings(key,value) VALUES(?1,x'00') ON CONFLICT(key) DO NOTHING",
            [key],
        )?;
    }
    database.execute_batch("INSERT INTO settings(key,value) VALUES('unrelated_setting',x'0102');
    INSERT INTO records(namespace,id,kind,state,created_at,payload) VALUES('unrelated','keep','keep','succeeded',1,x'0304');
    INSERT INTO action_journal VALUES('old','done',x'00');
    INSERT INTO calendar_copies VALUES('device','copy',x'00',x'00',x'00');
    INSERT INTO source_intents VALUES('old','stored',0,x'00');
    INSERT INTO sources VALUES('old',1,x'00');")?;
    // An old history trigger refuses audio writes if its counter disappears
    // first. The migration must retire this hook together with its setting.
    database.execute_batch("CREATE TRIGGER ref19_history_mark_jobs_insert_v1
    AFTER INSERT ON jobs WHEN NEW.kind='recording_manifest'
    BEGIN SELECT CASE WHEN NOT EXISTS(SELECT 1 FROM settings WHERE key='ref19_history_job_generation_v1:recording_manifest')
        THEN RAISE(ABORT,'history counter missing') END;
    UPDATE settings SET value=value+1 WHERE key='ref19_history_job_generation_v1:recording_manifest'; END;")?;
    fs::create_dir_all(directory.path().join("sources"))?;
    fs::write(
        directory.path().join("sources/retired.orbit"),
        b"synthetic ciphertext",
    )?;
    Ok(directory)
}

#[test]
fn version_three_retirement_preserves_pending_audio_private_notes_and_other_data() -> TestResult {
    let directory = retired_v3_store()?;
    let database = Connection::open(database_path(directory.path()))?;
    let before = database.query_row(
        "SELECT payload FROM chunks WHERE id='pending-audio'",
        [],
        |row| row.get::<_, Vec<u8>>(0),
    )?;
    let blob = fs::read(directory.path().join("chunks/pending-audio.orbit"))?;
    let mut store = Store::open(directory.path(), KEY)?;
    assert_eq!(store.diagnostics()?.schema_version, 4);
    for namespace in RETIRED_NAMESPACES {
        assert_eq!(
            database.query_row(
                "SELECT count(*) FROM records WHERE namespace=?1",
                [namespace],
                |row| row.get::<_, i64>(0)
            )?,
            0,
            "{namespace}"
        );
    }
    for key in RETIRED_SETTINGS {
        assert_eq!(
            database.query_row("SELECT count(*) FROM settings WHERE key=?1", [key], |row| {
                row.get::<_, i64>(0)
            })?,
            0,
            "{key}"
        );
    }
    for table in [
        "action_journal",
        "calendar_copies",
        "sources",
        "source_intents",
    ] {
        assert_eq!(
            database.query_row(
                "SELECT count(*) FROM sqlite_schema WHERE type='table' AND name=?1",
                [table],
                |row| row.get::<_, i64>(0)
            )?,
            0,
            "{table}"
        );
    }
    assert!(!directory.path().join("sources").exists());
    assert_eq!(store.read_chunk("pending-audio")?, b"synthetic audio");
    assert_eq!(
        store
            .get_job("upload-pending-audio")?
            .ok_or("upload lost")?
            .state,
        JobState::Queued
    );
    assert_eq!(
        store
            .private_notes("meeting")?
            .ok_or("private note lost")?
            .markdown,
        "nota privada pendiente"
    );
    assert_eq!(
        store.load_snapshot::<Value>()?,
        Some(json!({"privacy_stopped":true,"generation":9}))
    );
    assert_eq!(
        database.query_row(
            "SELECT payload FROM chunks WHERE id='pending-audio'",
            [],
            |row| row.get::<_, Vec<u8>>(0)
        )?,
        before
    );
    assert_eq!(
        fs::read(directory.path().join("chunks/pending-audio.orbit"))?,
        blob
    );
    assert_eq!(
        database.query_row(
            "SELECT value FROM settings WHERE key='unrelated_setting'",
            [],
            |row| row.get::<_, Vec<u8>>(0)
        )?,
        vec![1, 2]
    );
    assert_eq!(
        database.query_row(
            "SELECT payload FROM records WHERE namespace='unrelated'",
            [],
            |row| row.get::<_, Vec<u8>>(0)
        )?,
        vec![3, 4]
    );
    assert_eq!(database.query_row("SELECT count(*) FROM sqlite_schema WHERE type='trigger' AND name='ref19_history_mark_jobs_insert_v1'", [], |row| row.get::<_,i64>(0))?,0);
    store.enqueue_job(&Job {
        id: "new-manifest".into(),
        kind: "recording_manifest".into(),
        state: JobState::Succeeded,
        created_at: 2,
        payload: json!({"synthetic":true}),
    })?;
    drop(store);
    let version: i64 = database.pragma_query_value(None, "data_version", |row| row.get(0))?;
    drop(Store::open(directory.path(), KEY)?);
    assert_eq!(
        database.pragma_query_value::<i64, _>(None, "data_version", |row| row.get(0))?,
        version
    );
    Ok(())
}

#[test]
fn schemas_before_three_are_named_and_rejected_without_changing_the_store() -> TestResult {
    for version in [2, 1] {
        let directory = tempfile::tempdir()?;
        let mut store = Store::open(directory.path(), KEY)?;
        store.save_snapshot(&json!({"private":"synthetic"}))?;
        drop(store);
        let database = Connection::open(database_path(directory.path()))?;
        database.pragma_update(None, "user_version", version)?;
        database.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")?;
        drop(database);
        let before = fs::read(database_path(directory.path()))?;
        let error = Store::open(directory.path(), KEY)
            .err()
            .ok_or("old schema was accepted")?;
        let detail = error.to_string();
        assert!(detail.contains("entre 3 y 4"), "{detail}");
        assert!(detail.contains("orbit.sqlite3"), "{detail}");
        assert!(detail.contains(&version.to_string()), "{detail}");
        assert!(detail.contains('3'), "{detail}");
        assert_eq!(fs::read(database_path(directory.path()))?, before);
    }
    Ok(())
}

#[test]
fn retirement_rolls_back_sql_and_does_not_remove_files_when_a_delete_fails() -> TestResult {
    let directory = retired_v3_store()?;
    let database = Connection::open(database_path(directory.path()))?;
    database.execute_batch("CREATE TRIGGER fixture_retirement_failure BEFORE DELETE ON records WHEN OLD.namespace='people_identity' BEGIN SELECT RAISE(ABORT,'synthetic delete failure'); END")?;
    assert!(Store::open(directory.path(), KEY).is_err());
    assert_eq!(schema_version(&database)?, 3);
    for namespace in RETIRED_NAMESPACES {
        assert_eq!(
            database.query_row(
                "SELECT count(*) FROM records WHERE namespace=?1",
                [namespace],
                |row| row.get::<_, i64>(0)
            )?,
            1,
            "{namespace}"
        );
    }
    assert!(directory.path().join("sources/retired.orbit").exists());
    database.execute_batch("DROP TRIGGER fixture_retirement_failure")?;
    drop(Store::open(directory.path(), KEY)?);
    assert_eq!(schema_version(&database)?, 4);
    Ok(())
}

#[test]
fn wrong_key_cannot_retire_rows_or_files() -> TestResult {
    let directory = retired_v3_store()?;
    let database = Connection::open(database_path(directory.path()))?;
    assert!(matches!(
        Store::open(directory.path(), [33; 32]),
        Err(Error::Authentication)
    ));
    assert_eq!(schema_version(&database)?, 3);
    assert_eq!(
        database.query_row("SELECT count(*) FROM sources", [], |row| row
            .get::<_, i64>(0))?,
        1
    );
    assert!(directory.path().join("sources/retired.orbit").exists());
    Ok(())
}

#[test]
fn future_schema_version_is_rejected_explicitly() -> TestResult {
    let directory = tempfile::tempdir()?;
    drop(Store::open(directory.path(), KEY)?);
    let database = Connection::open(database_path(directory.path()))?;
    database.pragma_update(None, "user_version", 5)?;
    let error = Store::open(directory.path(), KEY)
        .err()
        .ok_or("future schema was accepted")?;
    let detail = error.to_string();
    assert!(detail.contains("orbit.sqlite3"), "{detail}");
    assert!(detail.contains("esquema 5"), "{detail}");
    assert!(detail.contains("entre 3 y 4"), "{detail}");
    assert_eq!(schema_version(&database)?, 5);
    Ok(())
}

#[test]
fn version_four_opens_without_running_migrations() -> TestResult {
    let directory = tempfile::tempdir()?;
    drop(Store::open(directory.path(), KEY)?);
    let before = migration_history(directory.path())?;
    drop(Store::open_existing(directory.path(), KEY)?);
    assert_eq!(migration_history(directory.path())?, before);
    Ok(())
}

fn schema_version(connection: &Connection) -> rusqlite::Result<i64> {
    connection.pragma_query_value(None, "user_version", |row| row.get(0))
}
