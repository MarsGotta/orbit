//! Real SQLite fixtures: opening is not initialization or a write request.
use orbit_store::{Job, JobState, Store};
use rusqlite::{Connection, types::Value};
use serde_json::json;
use std::{collections::BTreeMap, fs, path::Path};

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;
const KEY: [u8; 32] = [0x64; 32];

struct Fixture {
    // Keep WAL coordination alive so an ordinary last-close checkpoint cannot
    // obscure the stronger assertion about an admission's unchanged DB bytes.
    _keeper: Store,
    directory: tempfile::TempDir,
}

impl Fixture {
    fn new() -> Result<Self> {
        let directory = tempfile::tempdir()?;
        let mut keeper = Store::open(directory.path(), KEY)?;
        keeper.enqueue_job(&Job {
            id: "existing-writer-fixture".into(),
            kind: "synthetic".into(),
            state: JobState::Succeeded,
            created_at: 1,
            payload: json!({"text":"SYNTHETIC_PRIVATE_🪐"}),
        })?;
        keeper.save_snapshot(&json!({"revision":1}))?;
        Ok(Self {
            directory,
            _keeper: keeper,
        })
    }

    fn database(&self) -> Result<Connection> {
        Ok(Connection::open(
            self.directory.path().join("orbit.sqlite3"),
        )?)
    }
}

#[derive(Debug, PartialEq)]
struct DatabaseImage {
    schema: Vec<(String, String, Option<String>)>,
    tables: BTreeMap<String, Vec<Vec<Value>>>,
    version: i64,
    mode: String,
    database_bytes: Vec<u8>,
}

fn image(connection: &Connection, directory: &Path) -> Result<DatabaseImage> {
    let schema = connection
        .prepare("SELECT type,name,sql FROM sqlite_schema ORDER BY type,name")?
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
        .collect::<std::result::Result<Vec<(String, String, Option<String>)>, _>>()?;
    let mut tables = BTreeMap::new();
    for (_, name, _) in schema.iter().filter(|(kind, _, _)| kind == "table") {
        let identifier = name.replace('"', "\"\"");
        let mut query =
            connection.prepare(&format!("SELECT * FROM \"{identifier}\" ORDER BY rowid"))?;
        let columns = query.column_count();
        let rows = query
            .query_map([], |row| (0..columns).map(|i| row.get(i)).collect())?
            .collect::<std::result::Result<Vec<Vec<Value>>, _>>()?;
        tables.insert(name.clone(), rows);
    }
    Ok(DatabaseImage {
        schema,
        tables,
        version: connection.pragma_query_value(None, "user_version", |row| row.get(0))?,
        mode: connection.pragma_query_value(None, "journal_mode", |row| row.get(0))?,
        database_bytes: fs::read(directory.join("orbit.sqlite3"))?,
    })
}

#[test]
fn opening_existing_writer_is_non_mutating_and_an_explicit_write_is_durable() -> Result {
    let fixture = Fixture::new()?;
    let database = fixture.database()?;
    let before = image(&database, fixture.directory.path())?;
    let mut writer = Store::open_existing(fixture.directory.path(), KEY)?;
    let diagnostics = writer.diagnostics()?;
    assert_eq!(diagnostics.journal_mode, "wal");
    assert_eq!(diagnostics.synchronous, 2);
    assert_eq!(diagnostics.foreign_keys, 1);
    assert_eq!(diagnostics.fullfsync, 1);
    assert_eq!(image(&database, fixture.directory.path())?, before);
    writer.save_snapshot(&json!({"revision":2}))?;
    assert_eq!(
        writer.load_snapshot::<serde_json::Value>()?,
        Some(json!({"revision":2}))
    );
    drop(writer);
    let reopened = Store::open_existing(fixture.directory.path(), KEY)?;
    assert_eq!(
        reopened.load_snapshot::<serde_json::Value>()?,
        Some(json!({"revision":2}))
    );
    let reader = Store::open_reader(fixture.directory.path(), KEY)?;
    assert_eq!(
        reader.get_job("existing-writer-fixture")?,
        reopened.get_job("existing-writer-fixture")?
    );
    Ok(())
}

#[test]
fn opening_a_current_store_does_not_request_the_wal_writer_lock() -> Result {
    let fixture = Fixture::new()?;
    let writer = fixture.database()?;
    writer.execute_batch("BEGIN IMMEDIATE")?;

    // A current store has no schema, ledger or retirement work left. In WAL a
    // reader can coexist with this reservation, so an open failure here means
    // startup requested the single writer lock despite having nothing to write.
    let opened = Store::open(fixture.directory.path(), KEY);

    writer.execute_batch("ROLLBACK")?;
    drop(opened?);
    Ok(())
}

#[test]
fn opening_a_current_store_does_not_advance_an_observers_data_version() -> Result {
    let fixture = Fixture::new()?;
    let observer = fixture.database()?;
    let before: i64 = observer.pragma_query_value(None, "data_version", |row| row.get(0))?;

    drop(Store::open(fixture.directory.path(), KEY)?);

    let after: i64 = observer.pragma_query_value(None, "data_version", |row| row.get(0))?;
    assert_eq!(after, before);
    Ok(())
}

#[test]
fn wrong_key_or_invalid_verifier_never_replaces_data_or_makes_a_key() -> Result {
    for fault in ["wrong_key", "missing", "ciphertext", "oversized", "text"] {
        let fixture = Fixture::new()?;
        let database = fixture.database()?;
        match fault {
            "wrong_key" => {}
            "missing" => {
                database.execute("DELETE FROM settings WHERE key='key_check'", [])?;
            }
            "ciphertext" => {
                database.execute(
                    "UPDATE settings SET value=x'0102' WHERE key='key_check'",
                    [],
                )?;
            }
            "oversized" => {
                database.execute(
                    "UPDATE settings SET value=zeroblob(257) WHERE key='key_check'",
                    [],
                )?;
            }
            "text" => {
                database.execute(
                    "UPDATE settings SET value='not a blob' WHERE key='key_check'",
                    [],
                )?;
            }
            _ => unreachable!(),
        }
        let before = image(&database, fixture.directory.path())?;
        let key = if fault == "wrong_key" {
            [0x65; 32]
        } else {
            KEY
        };
        assert!(
            Store::open_existing(fixture.directory.path(), key).is_err(),
            "{fault}"
        );
        assert_eq!(
            image(&database, fixture.directory.path())?,
            before,
            "{fault}"
        );
        assert!(
            !fixture
                .directory
                .path()
                .join("development-storage.key")
                .exists()
        );
    }
    Ok(())
}

#[test]
fn old_zero_and_future_schemas_are_rejected_without_migration() -> Result {
    for version in [0, 1, 999] {
        let fixture = Fixture::new()?;
        let database = fixture.database()?;
        if version == 1 {
            // Genuine v1 table shape: no additive source tables to silently add.
            database.execute_batch(
                "DROP TABLE IF EXISTS sources; DROP TABLE IF EXISTS source_intents;",
            )?;
        }
        database.pragma_update(None, "user_version", version)?;
        let before = image(&database, fixture.directory.path())?;
        assert!(matches!(
            Store::open_existing(fixture.directory.path(), KEY),
            Err(orbit_store::Error::SchemaVersion { .. })
        ));
        assert_eq!(image(&database, fixture.directory.path())?, before);
    }
    Ok(())
}

#[test]
fn absent_domain_registries_are_not_initialized_by_opening() -> Result {
    let fixture = Fixture::new()?;
    let database = fixture.database()?;
    database.execute("DELETE FROM settings WHERE key!='key_check'", [])?;
    let before = image(&database, fixture.directory.path())?;
    let writer = Store::open_existing(fixture.directory.path(), KEY)?;
    assert!(writer.get_job("existing-writer-fixture")?.is_some());
    assert_eq!(image(&database, fixture.directory.path())?, before);
    Ok(())
}

#[test]
fn non_wal_store_is_not_switched_even_with_an_authenticated_key() -> Result {
    let directory = tempfile::tempdir()?;
    drop(Store::open(directory.path(), KEY)?);
    let database = Connection::open(directory.path().join("orbit.sqlite3"))?;
    database.pragma_update(None, "journal_mode", "DELETE")?;
    let before = image(&database, directory.path())?;
    assert!(Store::open_existing(directory.path(), KEY).is_err());
    assert_eq!(image(&database, directory.path())?, before);
    assert!(!directory.path().join("orbit.sqlite3-wal").exists());
    Ok(())
}

#[test]
fn missing_directory_database_or_required_subdirectory_is_never_created() -> Result {
    let empty = tempfile::tempdir()?;
    assert!(Store::open_existing(empty.path().join("missing"), KEY).is_err());
    assert!(Store::open_existing(empty.path(), KEY).is_err());
    assert_eq!(fs::read_dir(empty.path())?.count(), 0);
    for name in ["orbit.sqlite3", "chunks"] {
        let fixture = Fixture::new()?;
        let missing = fixture.directory.path().join(name);
        fs::rename(
            &missing,
            fixture.directory.path().join(format!("held-{name}")),
        )?;
        assert!(
            Store::open_existing(fixture.directory.path(), KEY).is_err(),
            "{name}"
        );
        assert!(!missing.exists());
    }
    Ok(())
}

#[cfg(unix)]
#[test]
fn insecure_files_and_directories_are_not_repaired_with_chmod() -> Result {
    use std::os::unix::fs::PermissionsExt;
    for name in ["", "orbit.sqlite3", "chunks"] {
        let fixture = Fixture::new()?;
        let path = fixture.directory.path().join(name);
        fs::set_permissions(&path, fs::Permissions::from_mode(0o777))?;
        let database_bytes = fs::read(fixture.directory.path().join("orbit.sqlite3"))?;
        assert!(
            Store::open_existing(fixture.directory.path(), KEY).is_err(),
            "{name}"
        );
        assert!(
            Store::open_reader(fixture.directory.path(), KEY).is_err(),
            "{name}"
        );
        assert_eq!(fs::metadata(&path)?.permissions().mode() & 0o777, 0o777);
        assert_eq!(
            fs::read(fixture.directory.path().join("orbit.sqlite3"))?,
            database_bytes
        );
    }
    Ok(())
}

#[cfg(unix)]
#[test]
fn ancestor_alias_is_accepted_but_leaf_symlinks_are_not_followed() -> Result {
    use std::os::unix::fs::symlink;
    let parent = tempfile::tempdir()?;
    let physical = parent.path().join("physical");
    let store_path = physical.join("store");
    let _keeper = Store::open(&store_path, KEY)?;
    let alias = parent.path().join("ancestor-alias");
    symlink(&physical, &alias)?;
    drop(Store::open_existing(alias.join("store"), KEY)?);
    symlink(&store_path, parent.path().join("store-alias"))?;
    assert!(Store::open_existing(parent.path().join("store-alias"), KEY).is_err());
    for name in ["orbit.sqlite3", "chunks"] {
        let fixture = Fixture::new()?;
        let leaf = fixture.directory.path().join(name);
        let held = fixture.directory.path().join(format!("held-{name}"));
        fs::rename(&leaf, &held)?;
        symlink(&held, &leaf)?;
        assert!(
            Store::open_existing(fixture.directory.path(), KEY).is_err(),
            "{name}"
        );
        assert!(fs::symlink_metadata(&leaf)?.file_type().is_symlink());
    }
    Ok(())
}

#[cfg(unix)]
#[test]
fn sidecar_symlinks_are_rejected_before_sqlite_can_touch_their_targets() -> Result {
    use std::os::unix::fs::symlink;
    for name in ["orbit.sqlite3-wal", "orbit.sqlite3-shm"] {
        let directory = tempfile::tempdir()?;
        drop(Store::open(directory.path(), KEY)?);
        let target = directory.path().join("synthetic-sidecar-target");
        fs::write(&target, b"UNCHANGED_SYNTHETIC_TARGET")?;
        symlink(&target, directory.path().join(name))?;
        assert!(Store::open_existing(directory.path(), KEY).is_err());
        assert_eq!(fs::read(&target)?, b"UNCHANGED_SYNTHETIC_TARGET");
    }
    Ok(())
}
