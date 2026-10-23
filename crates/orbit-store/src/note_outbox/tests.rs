use super::*;
use orbit_core::note_outbox::NoteOrder;
use std::collections::BTreeMap;

const RECORDING: &str = "6f2b7d5a-9c41-4b7e-8d3f-1a2b3c4d5e6f";
const KEY: [u8; 32] = [7; 32];

fn tick(done: bool) -> QueuedOrder {
    QueuedOrder::Meeting {
        recording_id: RECORDING.into(),
        order: NoteOrder::Edit {
            sections: None,
            done: BTreeMap::from([("a".to_owned(), done)]),
            base_revision: 7,
        },
    }
}

fn title(title: &str) -> QueuedOrder {
    QueuedOrder::Meeting {
        recording_id: RECORDING.into(),
        order: NoteOrder::Title {
            title: title.into(),
        },
    }
}

#[test]
fn an_order_waits_across_a_reopen_and_a_newer_one_on_its_target_replaces_it() -> Result<()> {
    let directory = tempfile::tempdir().map_err(Error::Io)?;
    let mut store = Store::open(directory.path(), KEY)?;
    let first = store.queue_note_order("edit", tick(true), 1)?;
    store.queue_note_order("title", title("Comité"), 2)?;
    let replaced = store.queue_note_order("edit", tick(false), 3)?;
    assert!(replaced.sequence > first.sequence);
    drop(store);
    let mut store = Store::open(directory.path(), KEY)?;
    let waiting = store.note_orders()?;
    assert_eq!(
        waiting
            .iter()
            .map(|waiting| (waiting.target.as_str(), waiting.order.clone()))
            .collect::<Vec<_>>(),
        [("title", title("Comité")), ("edit", tick(false))]
    );
    // The answer to the older one does not take the newer one with it.
    let lane = format!("meeting:{RECORDING}");
    assert!(!store.remove_note_order(&lane, "edit", first.sequence)?);
    assert!(store.remove_note_order(&lane, "edit", replaced.sequence)?);
    assert_eq!(store.note_orders()?.len(), 1);
    Ok(())
}

#[test]
fn a_row_moved_to_another_target_does_not_authenticate() -> Result<()> {
    let directory = tempfile::tempdir().map_err(Error::Io)?;
    let mut store = Store::open(directory.path(), KEY)?;
    store.queue_note_order("title", title("Comité"), 1)?;
    store
        .connection
        .execute("UPDATE note_outbox SET target='language'", [])?;
    assert!(matches!(store.note_orders(), Err(Error::Authentication)));
    Ok(())
}

/// A store an older Orbit wrote (schema 4, no outbox) gains the table when
/// it opens, keeps what it held, and stays at schema 4; a store that has it
/// opens without writing.
#[test]
fn a_store_without_the_outbox_gains_it_and_keeps_its_private_notes() -> Result<()> {
    let directory = tempfile::tempdir().map_err(Error::Io)?;
    let mut store = Store::open(directory.path(), KEY)?;
    store.save_private_notes(
        &crate::PendingPrivateNotes {
            recording_id: RECORDING.into(),
            markdown: "- de antes".into(),
            base_revision: 0,
        },
        1,
    )?;
    store.connection.execute_batch("DROP TABLE note_outbox")?;
    drop(store);
    let mut store = Store::open(directory.path(), KEY)?;
    assert_eq!(store.pending_private_notes()?, vec![RECORDING.to_owned()]);
    assert_eq!(store.diagnostics()?.schema_version, 4);
    store.queue_note_order("title", title("Comité"), 2)?;
    drop(store);
    let database = rusqlite::Connection::open(directory.path().join("orbit.sqlite3"))?;
    let before: i64 = database.pragma_query_value(None, "data_version", |row| row.get(0))?;
    drop(Store::open(directory.path(), KEY)?);
    let after: i64 = database.pragma_query_value(None, "data_version", |row| row.get(0))?;
    assert_eq!(before, after);
    Ok(())
}
