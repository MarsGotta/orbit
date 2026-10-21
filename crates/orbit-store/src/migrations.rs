use crate::Result;
use rusqlite::{Connection, TransactionBehavior};

pub(crate) const SCHEMA_VERSION: i64 = 4;
const RECORD_SCHEMA: &str = "
CREATE TABLE records (
    namespace TEXT NOT NULL,
    id TEXT NOT NULL,
    kind TEXT NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('queued','running','retry_wait','succeeded','blocked','cancelled','unknown')),
    created_at INTEGER NOT NULL,
    payload BLOB NOT NULL,
    PRIMARY KEY(namespace,id)
);
CREATE INDEX records_namespace_kind_time ON records(namespace,kind,created_at,id);
CREATE TABLE migrations(version INTEGER PRIMARY KEY,applied_at INTEGER NOT NULL,description TEXT NOT NULL);
";

// Generic job writes still route these legacy kinds away from the audio queue.
// No schema migration or retired ledger is needed to classify a kind.
pub(crate) fn record_namespace(kind: &str) -> Option<&'static str> {
    match kind {
        "app_capability_head" | "app_capability_operation" => Some("app_capabilities"),
        "meeting_archive" | "meeting_archive_page" => Some("meeting_archives"),
        "meeting_calendar_head" | "meeting_calendar_source" | "meeting_calendar_operation" => {
            Some("meeting_calendar")
        }
        "meeting_proposal_control" => Some("meeting_proposals"),
        "meeting_source" => Some("meeting_sources"),
        "meeting_synthesis_run" | "meeting_synthesis_receipt" => Some("meeting_synthesis"),
        "native_action_attempt" | "native_adapter_head" | "native_adapter_operation" => {
            Some("native_actions")
        }
        "people_identity_head" | "people_identity_operation" => Some("people_identity"),
        "proposal_materialization" | "proposal_control" | "proposal_receipt" => Some("proposals"),
        "publication_preview" | "publication_control" | "publication_receipt" => {
            Some("publications")
        }
        "source_reading_head" | "source_reading_operation" => Some("source_reading"),
        "speaker_review_head" | "speaker_review_operation" | "speaker_review_page" => {
            Some("speaker_review")
        }
        "transcript_fetch_retry" => Some("transcript_fetches"),
        "voice_memory_head" | "voice_memory_operation" | "voice_memory_artifact" => {
            Some("voice_memory")
        }
        _ => None,
    }
}

pub(crate) fn initialize(connection: &mut Connection, verifier: &[u8]) -> Result<()> {
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    transaction.execute_batch(include_str!("schema.sql"))?;
    transaction.execute_batch(RECORD_SCHEMA)?;
    transaction.execute(
        "INSERT INTO settings(key,value) VALUES('key_check',?1)",
        [verifier],
    )?;
    transaction.pragma_update(None, "user_version", SCHEMA_VERSION)?;
    transaction.commit()?;
    Ok(())
}

/// v3 is the oldest schema any installed Mac uses. Remove only retired
/// namespaces and settings; pending audio, private notes and unknown data stay.
pub(crate) fn migrate(connection: &mut Connection, version: i64) -> Result<()> {
    if version == SCHEMA_VERSION {
        return Ok(());
    }
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    // Retire their triggers before their counters: otherwise the history
    // triggers left by older versions would reject future audio job writes.
    // The trigger and setting names below are the ones older stores hold, so
    // they cannot change.
    transaction.execute_batch(
        "
DROP TRIGGER IF EXISTS ref19_history_mark_jobs_insert_v1;
DROP TRIGGER IF EXISTS ref19_history_mark_jobs_update_v1;
DROP TRIGGER IF EXISTS ref19_history_mark_jobs_delete_v1;
DROP TRIGGER IF EXISTS ref19_history_mark_synthesis_insert_v1;
DROP TRIGGER IF EXISTS ref19_history_mark_synthesis_update_v1;
DROP TRIGGER IF EXISTS ref19_history_mark_synthesis_delete_v1;
DROP TRIGGER IF EXISTS ref208_notes_publication_insert_v1;
DROP TRIGGER IF EXISTS ref208_notes_publication_update_v1;
DROP TRIGGER IF EXISTS ref208_notes_publication_delete_v1;
DROP TRIGGER IF EXISTS ref208_notes_publication_intent_insert_v1;
DROP TRIGGER IF EXISTS ref208_notes_publication_intent_update_v1;
DROP TRIGGER IF EXISTS ref208_notes_publication_intent_delete_v1;
DELETE FROM records WHERE namespace IN (
    'app_capabilities','meeting_calendar','meeting_proposals','meeting_synthesis',
    'native_actions','people_identity','proposals','publications','source_reading','voice_memory'
);
DELETE FROM settings WHERE key IN (
    'action_count_v1','meeting_calendar_registry_v1','meeting_calendar_registry_revision_v1',
    'proposal_usage_v1','publication_usage_v1','ref208_notes_publication_generation_v1',
    'calendar_copies_registry_v1','source_usage_v1',
    'ref19_history_job_generation_v1:recording_finalize',
    'ref19_history_job_generation_v1:recording_manifest',
    'ref19_history_job_generation_v1:transcript','ref19_history_synthesis_generation_v1'
);
DROP TABLE IF EXISTS sources;
DROP TABLE IF EXISTS source_intents;
DROP TABLE IF EXISTS action_journal;
DROP TABLE IF EXISTS calendar_copies;
INSERT INTO migrations(version,applied_at,description)
VALUES(4,unixepoch('now'),'retire source persistence and unused verticals');
",
    )?;
    transaction.pragma_update(None, "user_version", SCHEMA_VERSION)?;
    transaction.commit()?;
    Ok(())
}

/// The copy of each conversation an older daemon wrote here, one
/// encrypted row per message, which nothing read. The conversations are the
/// hub's, read a page at a time, so the table goes with its rows.
///
/// Why it does not raise `user_version`: a version would lock the previous
/// release out of the store it wrote; that release and this one are not
/// mixed (hub and app are published together), and the table holds nothing
/// the product reads. `IF EXISTS` makes it a no-op once dropped.
pub(crate) fn retire_messages(connection: &Connection) -> Result<()> {
    connection.execute_batch("DROP TABLE IF EXISTS messages")?;
    Ok(())
}

/// What the Mac kept of the hub's meetings and notes. The hub is now
/// their only home, so an older store's copy is removed when the store opens:
/// the final transcripts and their fetches (`transcript`, `transcript_fetch`
/// jobs, `transcript_fetches` retries), the long transcripts' archives
/// (`meeting_archives`), the retired meeting sources (`meeting_sources`), the
/// speaker reviews (`speaker_review`) and, under `recording_notes`, every
/// kind but the private notes, which wait there for the hub
/// (`recording_notes::RETIRED_KINDS`), with the ledgers' and marks' settings.
///
/// Why it does not raise `user_version`: the tables stay as the v3 schema made
/// them, only emptier, so an older build still opens the store (its own quota
/// ledgers start afresh, with no rows to count). A store with nothing to
/// remove only reads: the IMMEDIATE transaction is taken for actual legacy
/// data.
pub(crate) fn retire_meeting_mirror(connection: &mut Connection) -> Result<()> {
    const NAMESPACES: [&str; 4] = [
        "meeting_archives",
        "meeting_sources",
        "speaker_review",
        "transcript_fetches",
    ];
    const JOB_KINDS: [&str; 2] = ["transcript", "transcript_fetch"];
    const SETTINGS: [&str; 4] = [
        "recording_notes_mark_v1",
        "recording_note_heads_mark_v1",
        "meeting_archive_usage_v1",
        "speaker_review_usage_v1",
    ];
    let retired = crate::recording_notes::RETIRED_KINDS;
    let notes = crate::recording_notes::NAMESPACE;
    let placeholders = |count: usize| {
        std::iter::repeat_n("?", count)
            .collect::<Vec<_>>()
            .join(",")
    };
    let records = format!(
        "namespace IN ({}) OR (namespace=? AND kind IN ({}))",
        placeholders(NAMESPACES.len()),
        placeholders(retired.len())
    );
    let record_values = NAMESPACES
        .iter()
        .chain(std::iter::once(&notes))
        .chain(retired.iter())
        .copied()
        .collect::<Vec<_>>();
    let jobs = format!("kind IN ({})", placeholders(JOB_KINDS.len()));
    let settings = format!("key IN ({})", placeholders(SETTINGS.len()));
    let exists: bool = connection.query_row(
        &format!(
            "SELECT EXISTS(SELECT 1 FROM records WHERE {records})
                 OR EXISTS(SELECT 1 FROM jobs WHERE {jobs})
                 OR EXISTS(SELECT 1 FROM settings WHERE {settings})"
        ),
        rusqlite::params_from_iter(
            record_values
                .iter()
                .chain(JOB_KINDS.iter())
                .chain(SETTINGS.iter()),
        ),
        |row| row.get(0),
    )?;
    if !exists {
        return Ok(());
    }
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    transaction.execute(
        &format!("DELETE FROM records WHERE {records}"),
        rusqlite::params_from_iter(record_values.iter()),
    )?;
    transaction.execute(
        &format!("DELETE FROM jobs WHERE {jobs}"),
        rusqlite::params_from_iter(JOB_KINDS.iter()),
    )?;
    transaction.execute(
        &format!("DELETE FROM settings WHERE {settings}"),
        rusqlite::params_from_iter(SETTINGS.iter()),
    )?;
    transaction.commit()?;
    Ok(())
}
