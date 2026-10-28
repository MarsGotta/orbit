//! «Al detectar una llamada» is kept by the daemon, encrypted, and
//! survives a restart. Real encrypted SQLite; nothing native is started.
use orbit_core::call_detection::CallPreference;
use orbit_store::Store;
use rusqlite::Connection;

#[test]
fn a_preference_never_chosen_reads_as_absent_not_as_a_choice()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let store = Store::open(directory.path(), [41; 32])?;
    assert_eq!(store.load_call_preference()?, None);
    Ok(())
}

#[test]
fn the_preference_survives_a_restart_and_is_not_stored_in_clear()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let mut store = Store::open(directory.path(), [41; 32])?;
    store.save_call_preference(CallPreference::Record)?;
    assert_eq!(store.load_call_preference()?, Some(CallPreference::Record));
    store.save_call_preference(CallPreference::Nothing)?;
    drop(store);
    let reopened = Store::open(directory.path(), [41; 32])?;
    assert_eq!(
        reopened.load_call_preference()?,
        Some(CallPreference::Nothing)
    );
    let database = Connection::open(directory.path().join("orbit.sqlite3"))?;
    let bytes: Vec<u8> = database.query_row(
        "SELECT value FROM settings WHERE key='call_preference_v1'",
        [],
        |row| row.get(0),
    )?;
    assert!(
        !bytes
            .windows(b"nothing".len())
            .any(|value| value == b"nothing"),
        "the preference is encrypted like every other setting"
    );
    Ok(())
}

#[test]
fn an_unreadable_preference_is_an_error_never_a_default() -> Result<(), Box<dyn std::error::Error>>
{
    let directory = tempfile::tempdir()?;
    let mut store = Store::open(directory.path(), [41; 32])?;
    store.save_call_preference(CallPreference::Record)?;
    let database = Connection::open(directory.path().join("orbit.sqlite3"))?;
    database.execute(
        "UPDATE settings SET value=X'010203' WHERE key='call_preference_v1'",
        [],
    )?;
    assert!(store.load_call_preference().is_err());
    // Choosing again replaces what could not be read.
    store.save_call_preference(CallPreference::Notify)?;
    assert_eq!(store.load_call_preference()?, Some(CallPreference::Notify));
    Ok(())
}

#[test]
fn a_row_too_large_to_be_the_preference_says_its_bytes_and_the_limit()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let mut store = Store::open(directory.path(), [41; 32])?;
    store.save_call_preference(CallPreference::Record)?;
    let database = Connection::open(directory.path().join("orbit.sqlite3"))?;
    database.execute(
        "UPDATE settings SET value=zeroblob(10000) WHERE key='call_preference_v1'",
        [],
    )?;
    let Err(error) = store.load_call_preference() else {
        return Err("a 10 000-byte row read as a preference".into());
    };
    let text = error.to_string();
    assert!(text.contains("10000 bytes"), "{text}");
    assert!(text.contains("maximum"), "{text}");
    Ok(())
}
