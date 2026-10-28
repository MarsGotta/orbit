//! «Al detectar una llamada», one encrypted setting. A preference,
//! not a permission: the macOS microphone permission is the decision.
use crate::{
    Result, Store,
    crypto::OVERHEAD,
    validation::{bounded_json, bounded_setting},
};
use orbit_core::call_detection::CallPreference;

const CONTEXT: &[u8] = b"orbit:call-preference:v1";
/// One quoted word: `"nothing"` is nine bytes. The bound only refuses a row
/// that is not this setting; it is not a limit the user can reach.
const MAX_BYTES: usize = 64;

impl Store {
    /// The preference the user chose, or `None` if they never chose one. A row
    /// that cannot be read is an error, never a default.
    pub fn load_call_preference(&self) -> Result<Option<CallPreference>> {
        let bytes = bounded_setting(
            &self.connection,
            "call_preference_v1",
            MAX_BYTES + OVERHEAD,
            "The stored video call preference",
        )?;
        let Some(bytes) = bytes else {
            return Ok(None);
        };
        let clear = self.cipher.decrypt(CONTEXT, &bytes)?;
        Ok(Some(serde_json::from_slice(&clear)?))
    }

    /// Replaces the preference, including one that could not be read: a new
    /// choice is the answer to a setting that was lost.
    pub fn save_call_preference(&mut self, preference: CallPreference) -> Result<()> {
        let bytes = bounded_json(&preference, MAX_BYTES)?;
        let payload = self.cipher.encrypt(CONTEXT, &bytes)?;
        self.connection.execute(
            "INSERT INTO settings(key,value)VALUES('call_preference_v1',?1)
            ON CONFLICT(key)DO UPDATE SET value=excluded.value",
            [payload],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::{ErrorCode, limits::Limit};
    use serde_json::json;

    #[test]
    fn oversized_settings_are_measured_without_materializing_the_blob() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let mut store = Store::open(directory.path(), [7; 32])?;
        let bytes = 8 * crate::MAX_PAYLOAD_BYTES;
        store.connection.execute(
            "INSERT INTO settings(key,value) VALUES('snapshot',zeroblob(?1)),
                ('call_preference_v1',zeroblob(?1))",
            [bytes as i64],
        )?;
        // A connection-local SQLite limit lets length(value) inspect the stored
        // length, but refuses to materialize the blob as a query result.
        store.connection.set_limit(
            Limit::SQLITE_LIMIT_LENGTH,
            (2 * crate::MAX_PAYLOAD_BYTES) as i32,
        )?;
        let raw = store.connection.query_row(
            "SELECT value FROM settings WHERE key='snapshot'",
            [],
            |row| row.get::<_, Vec<u8>>(0),
        );
        assert!(matches!(raw, Err(rusqlite::Error::SqliteFailure(code, _))
            if code.code == ErrorCode::TooBig));
        let before = store.decryption_count();
        let mut failures = Vec::new();
        for (reader, result, max_bytes) in [
            (
                "load_snapshot",
                store.load_snapshot::<serde_json::Value>().map(|_| ()),
                crate::MAX_PAYLOAD_BYTES + OVERHEAD,
            ),
            (
                "save_snapshot",
                store.save_snapshot(&json!({"revision":1})),
                crate::MAX_PAYLOAD_BYTES + OVERHEAD,
            ),
            (
                "load_call_preference",
                store.load_call_preference().map(|_| ()),
                MAX_BYTES + OVERHEAD,
            ),
        ] {
            if !matches!(&result, Err(crate::Error::TooLarge { bytes: actual, max_bytes: max, .. })
                if *actual == bytes as u64 && *max == max_bytes as u64)
            {
                failures.push(format!("{reader}: {result:?}"));
            }
        }
        assert!(failures.is_empty(), "{failures:?}");
        assert_eq!(store.decryption_count(), before);
        Ok(())
    }
}
