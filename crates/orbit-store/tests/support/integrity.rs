//! Check the real SQLite database of a synthetic fixture.
pub fn assert_integrity(directory: &std::path::Path) -> rusqlite::Result<()> {
    let database = rusqlite::Connection::open_with_flags(
        directory.join("orbit.sqlite3"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?;
    let result: String = database.query_row("PRAGMA integrity_check", [], |row| row.get(0))?;
    assert_eq!(result, "ok");
    Ok(())
}
