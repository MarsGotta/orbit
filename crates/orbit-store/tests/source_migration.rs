//! Source directory retirement never follows a symlink or needs a ledger.
use orbit_store::{Error, Store};
use std::fs;
type TestResult = Result<(), Box<dyn std::error::Error>>;
const KEY: [u8; 32] = [0x42; 32];
#[cfg(unix)]
#[test]
fn retiring_a_source_directory_does_not_follow_a_symlink() -> TestResult {
    use std::os::unix::fs::symlink;
    let directory = tempfile::tempdir()?;
    drop(Store::open(directory.path(), KEY)?);
    let outside = tempfile::tempdir()?;
    fs::write(outside.path().join("keep"), b"synthetic")?;
    let sources = directory.path().join("sources");
    if sources.try_exists()? {
        fs::remove_dir(&sources)?;
    }
    symlink(outside.path(), &sources)?;
    assert!(matches!(
        Store::open(directory.path(), KEY),
        Err(Error::UnsafePath)
    ));
    assert_eq!(fs::read(outside.path().join("keep"))?, b"synthetic");
    assert_eq!(fs::read_dir(outside.path())?.count(), 1);
    Ok(())
}
#[test]
fn reopening_finishes_file_cleanup_after_the_sql_migration_committed() -> TestResult {
    let directory = tempfile::tempdir()?;
    drop(Store::open(directory.path(), KEY)?);
    fs::create_dir_all(directory.path().join("sources"))?;
    fs::write(directory.path().join("sources/retired.orbit"), b"synthetic")?;
    drop(Store::open(directory.path(), KEY)?);
    assert!(!directory.path().join("sources").exists());
    Ok(())
}

#[cfg(unix)]
#[test]
fn opening_through_an_ancestor_alias_still_accepts_the_current_store() -> TestResult {
    use std::os::unix::fs::symlink;
    let directory = tempfile::tempdir()?;
    let physical = directory.path().join("physical");
    let path = physical.join("store");
    drop(Store::open(&path, KEY)?);
    let alias = directory.path().join("alias");
    symlink(&physical, &alias)?;
    drop(Store::open(alias.join("store"), KEY)?);
    Ok(())
}

#[cfg(unix)]
#[test]
fn schema_inspection_rejects_sidecar_symlinks_before_opening_sqlite() -> TestResult {
    use std::os::unix::fs::symlink;
    for name in ["orbit.sqlite3-wal", "orbit.sqlite3-shm"] {
        let directory = tempfile::tempdir()?;
        drop(Store::open(directory.path(), KEY)?);
        let target = directory.path().join("keep");
        fs::write(&target, b"synthetic unchanged")?;
        symlink(&target, directory.path().join(name))?;
        assert!(matches!(
            Store::open(directory.path(), KEY),
            Err(Error::UnsafePath)
        ));
        assert_eq!(fs::read(&target)?, b"synthetic unchanged");
    }
    Ok(())
}
