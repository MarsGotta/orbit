use orbit_store::{Job, JobState, Store};
use serde_json::json;

#[test]
fn reader_authenticates_existing_data_with_a_narrow_read_only_api()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let mut writer = Store::open(directory.path(), [7; 32])?;
    let job = Job {
        id: "synthetic-reader".into(),
        kind: "synthetic".into(),
        state: JobState::Succeeded,
        created_at: 1,
        payload: json!({"fixture":true}),
    };
    writer.enqueue_job(&job)?;
    let reader = Store::open_reader(directory.path(), [7; 32])?;
    assert_eq!(reader.get_job(&job.id)?, Some(job));
    assert!(Store::open_reader(directory.path(), [8; 32]).is_err());
    Ok(())
}

#[test]
fn reader_does_not_create_a_store_or_restore_a_missing_database()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    assert!(Store::open_reader(directory.path(), [7; 32]).is_err());
    assert_eq!(std::fs::read_dir(directory.path())?.count(), 0);
    let store = Store::open(directory.path(), [7; 32])?;
    drop(store);
    std::fs::rename(
        directory.path().join("orbit.sqlite3"),
        directory.path().join("fixture-backup.sqlite3"),
    )?;
    assert!(Store::open_reader(directory.path(), [7; 32]).is_err());
    assert!(!directory.path().join("orbit.sqlite3").exists());
    Ok(())
}

#[test]
fn reader_does_not_migrate_a_schema_even_with_the_correct_key()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let store = Store::open(directory.path(), [7; 32])?;
    drop(store);
    let connection = rusqlite::Connection::open(directory.path().join("orbit.sqlite3"))?;
    connection.pragma_update(None, "user_version", 1)?;
    assert!(Store::open_reader(directory.path(), [7; 32]).is_err());
    let version: i64 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
    assert_eq!(version, 1);
    Ok(())
}

#[cfg(unix)]
#[test]
fn reader_rejects_symlinks_and_insecure_permissions_without_chmod()
-> Result<(), Box<dyn std::error::Error>> {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let directory = tempfile::tempdir()?;
    let store_path = directory.path().join("store");
    let store = Store::open(&store_path, [7; 32])?;
    drop(store);
    let alias = directory.path().join("alias");
    symlink(&store_path, &alias)?;
    assert!(Store::open_reader(&alias, [7; 32]).is_err());
    let database = store_path.join("orbit.sqlite3");
    std::fs::set_permissions(&database, std::fs::Permissions::from_mode(0o644))?;
    assert!(Store::open_reader(&store_path, [7; 32]).is_err());
    assert_eq!(
        std::fs::metadata(&database)?.permissions().mode() & 0o777,
        0o644
    );
    Ok(())
}

#[cfg(unix)]
#[test]
fn reader_accepts_ancestor_aliases_but_not_store_or_database_symlinks()
-> Result<(), Box<dyn std::error::Error>> {
    use std::os::unix::fs::symlink;
    let directory = tempfile::tempdir()?;
    let parent = directory.path().join("physical");
    let physical_store = parent.join("store");
    let writer = Store::open(&physical_store, [7; 32])?;
    let alias = directory.path().join("ancestor-alias");
    symlink(&parent, &alias)?;
    let selected = alias.join("store");
    // This is the SQLite path-resolver behavior behind the macOS regression.
    let error = rusqlite::Connection::open_with_flags(
        selected.join("orbit.sqlite3"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NOFOLLOW,
    )
    .err()
    .ok_or("SQLite unexpectedly followed the ancestor alias")?;
    assert_eq!(
        error.sqlite_error().map(|error| error.extended_code),
        Some(rusqlite::ffi::SQLITE_CANTOPEN_SYMLINK)
    );
    let reader = Store::open_reader(&selected, [7; 32])?;
    assert!(reader.get_job("synthetic-missing")?.is_none());
    drop(reader);
    drop(writer);
    let leaf_alias = parent.join("store-alias");
    symlink(&physical_store, &leaf_alias)?;
    assert!(matches!(
        Store::open_reader(alias.join("store-alias"), [7; 32]),
        Err(orbit_store::Error::UnsafePath)
    ));
    let database = physical_store.join("orbit.sqlite3");
    let original = physical_store.join("original.sqlite3");
    std::fs::rename(&database, &original)?;
    symlink(&original, &database)?;
    assert!(matches!(
        Store::open_reader(&selected, [7; 32]),
        Err(orbit_store::Error::UnsafePath)
    ));
    Ok(())
}
