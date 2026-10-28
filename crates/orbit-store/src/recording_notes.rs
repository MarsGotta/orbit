//! The user's private notes waiting for the hub, in the `records` table
//! under the `recording_notes` namespace, one row per recording
//! (`note-private-{recording}`).
//!
//! A row exists only while the hub has not confirmed that text: the daemon
//! sends it, and deletes the row when the hub answers with a revision for
//! exactly that text. The rows an older Mac kept here (same kind and
//! identity) are private notes the hub never had, so they go up the same way
//! once and leave the Mac when the hub confirms them.
//!
//! Each row is AEAD-sealed with its kind and identity as associated data, so
//! a row moved to another identity does not authenticate. Only the store
//! actor writes, so a confirmation and a new save never interleave.
use crate::{
    Error, Result, Store,
    crypto::OVERHEAD,
    validation::{bounded_blob, bounded_json, context, validate_id},
};
use orbit_core::notes::check_private_notes;
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};

pub(crate) const NAMESPACE: &str = "recording_notes";
const PRIVATE_KIND: &str = "recording_note_private";
const PRIVATE_PREFIX: &str = "note-private-";
/// Private notes are at most 65,536 characters; JSON escapes a control
/// character as six bytes, so 393,216 bytes is the worst case. 512 KiB holds
/// it; the character bound is checked before this one.
pub const MAX_PRIVATE_NOTES_BYTES: usize = 512 * 1024;
/// What an older Mac kept under this namespace and nothing reads any more:
/// the note's cached document and list line, the hub's templates and
/// assignments, and the template orders not yet sent.
/// Removed when the store opens (`migrations::retire_meeting_mirror`).
pub(crate) const RETIRED_KINDS: [&str; 5] = [
    "recording_note_sync",
    "recording_note_head",
    "recording_note_templates",
    "recording_note_template_links",
    "recording_note_library_outbox",
];

/// Private notes on their way to the hub.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PendingPrivateNotes {
    pub recording_id: String,
    pub markdown: String,
    /// The hub's revision this text was written on; 0 for notes an older
    /// Mac kept, which the hub never had.
    #[serde(default)]
    pub base_revision: u64,
}

pub(crate) fn id_of(recording: &str) -> String {
    format!("{PRIVATE_PREFIX}{recording}")
}

impl Store {
    /// Keeps the user's private notes of one recording until the hub confirms
    /// them, at most `MAX_PRIVATE_NOTES_CHARS` characters (the hub's bound).
    pub fn save_private_notes(&mut self, notes: &PendingPrivateNotes, now_ms: i64) -> Result<()> {
        check_private_notes(&notes.markdown)?;
        let id = id_of(&notes.recording_id);
        validate_id(&id)?;
        let bytes = bounded_json(notes, MAX_PRIVATE_NOTES_BYTES)?;
        let payload = self.cipher.encrypt(&context(PRIVATE_KIND, &id), &bytes)?;
        self.connection.execute(
            "INSERT INTO records(namespace,id,kind,state,created_at,payload) VALUES(?1,?2,?3,'queued',?4,?5)
             ON CONFLICT(namespace,id) DO UPDATE SET kind=excluded.kind, state=excluded.state,
                 created_at=excluded.created_at, payload=excluded.payload",
            params![NAMESPACE, id, PRIVATE_KIND, now_ms, payload],
        )?;
        Ok(())
    }

    pub fn private_notes(&self, recording: &str) -> Result<Option<PendingPrivateNotes>> {
        let id = id_of(recording);
        validate_id(&id)?;
        let row = self
            .connection
            .query_row(
                "SELECT kind,length(payload),payload FROM records WHERE namespace=?1 AND id=?2",
                params![NAMESPACE, id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        bounded_blob(
                            row,
                            2,
                            MAX_PRIVATE_NOTES_BYTES + OVERHEAD,
                            "The stored private notes",
                        )?,
                    ))
                },
            )
            .optional()?;
        let Some((kind, payload)) = row else {
            return Ok(None);
        };
        if kind != PRIVATE_KIND {
            return Err(Error::Authentication);
        }
        let clear = self.cipher.decrypt(&context(PRIVATE_KIND, &id), &payload)?;
        let notes: PendingPrivateNotes = serde_json::from_slice(&clear)?;
        if notes.recording_id != recording {
            return Err(Error::Authentication);
        }
        Ok(Some(notes))
    }

    /// Every recording with private notes waiting for the hub.
    pub fn pending_private_notes(&self) -> Result<Vec<String>> {
        let mut statement = self
            .connection
            .prepare("SELECT id FROM records WHERE namespace=?1 AND kind=?2 ORDER BY id")?;
        let ids = statement
            .query_map(params![NAMESPACE, PRIVATE_KIND], |row| {
                row.get::<_, String>(0)
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        ids.into_iter()
            .map(|id| {
                id.strip_prefix(PRIVATE_PREFIX)
                    .map(str::to_owned)
                    .ok_or(Error::Invalid("private notes identifier"))
            })
            .collect()
    }

    /// The hub confirmed `markdown` for `recording`: the row leaves the Mac,
    /// unless the user wrote something newer meanwhile, which stays to be sent.
    /// Returns whether it left.
    pub fn confirm_private_notes(&mut self, recording: &str, markdown: &str) -> Result<bool> {
        if self
            .private_notes(recording)?
            .is_none_or(|stored| stored.markdown != markdown)
        {
            return Ok(false);
        }
        self.connection.execute(
            "DELETE FROM records WHERE namespace=?1 AND id=?2",
            params![NAMESPACE, id_of(recording)],
        )?;
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The private notes an older Mac sealed had no revision: they
    /// read as revision 0, the hub's «never had them», and go up once.
    #[test]
    fn the_older_macs_private_notes_are_pending_at_revision_zero() -> Result<()> {
        let directory = tempfile::tempdir().map_err(Error::Io)?;
        let mut store = Store::open(directory.path(), [7; 32])?;
        let recording = "6f2b7d5a-9c41-4b7e-8d3f-1a2b3c4d5e6f";
        let id = id_of(recording);
        let older = serde_json::to_vec(&serde_json::json!({
            "recording_id": recording, "markdown": "- de la 0.4.0"
        }))?;
        let payload = store.cipher.encrypt(&context(PRIVATE_KIND, &id), &older)?;
        store.connection.execute(
            "INSERT INTO records(namespace,id,kind,state,created_at,payload) VALUES(?1,?2,?3,'succeeded',1,?4)",
            params![NAMESPACE, id, PRIVATE_KIND, payload],
        )?;
        assert_eq!(store.pending_private_notes()?, vec![recording.to_owned()]);
        assert_eq!(
            store.private_notes(recording)?,
            Some(PendingPrivateNotes {
                recording_id: recording.into(),
                markdown: "- de la 0.4.0".into(),
                base_revision: 0,
            })
        );
        assert!(store.confirm_private_notes(recording, "- de la 0.4.0")?);
        assert!(store.pending_private_notes()?.is_empty());
        Ok(())
    }
}
