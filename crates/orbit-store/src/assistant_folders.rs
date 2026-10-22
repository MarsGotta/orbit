//! The folders the user gave the assistant, one encrypted setting. Each
//! carries the macOS bookmark that finds it again; the panel choice that
//! produced it is the user's whole decision, so nothing here expires.
use crate::{
    Result, Store,
    crypto::OVERHEAD,
    validation::{bounded_json, bounded_setting},
};
use orbit_core::assistant_files::{Folder, MAX_BOOKMARK_BYTES, MAX_FOLDERS, MAX_PATH_BYTES};

const CONTEXT: &[u8] = b"orbit:assistant-folders:v1";
const KEY: &str = "assistant_folders_v1";

/// The largest list a row may hold: every folder at its longest path and
/// bookmark (base64 is 4/3 of it), its id and the JSON around them. It only
/// refuses a row that cannot be this list; it is not a limit the user reaches
/// before `MAX_FOLDERS` does.
const MAX_BYTES: usize =
    MAX_FOLDERS * (MAX_BOOKMARK_BYTES.div_ceil(3) * 4 + MAX_PATH_BYTES * 6 + 256) + 2;

impl Store {
    /// The folders, or none when the user never chose one. A row that cannot
    /// be read is an error, never an empty list.
    pub fn load_assistant_folders(&self) -> Result<Vec<Folder>> {
        let Some(bytes) = bounded_setting(
            &self.connection,
            KEY,
            MAX_BYTES + OVERHEAD,
            "Las carpetas del asistente guardadas",
        )?
        else {
            return Ok(Vec::new());
        };
        let clear = self.cipher.decrypt(CONTEXT, &bytes)?;
        Ok(serde_json::from_slice(&clear)?)
    }

    /// Replaces the whole list; an empty list is kept as such.
    pub fn save_assistant_folders(&mut self, folders: &[Folder]) -> Result<()> {
        let bytes = bounded_json(&folders, MAX_BYTES)?;
        let payload = self.cipher.encrypt(CONTEXT, &bytes)?;
        self.connection.execute(
            "INSERT INTO settings(key,value)VALUES(?1,?2)
            ON CONFLICT(key)DO UPDATE SET value=excluded.value",
            rusqlite::params![KEY, payload],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(index: usize) -> Folder {
        Folder {
            id: format!("00000000-0000-4000-8000-{index:012}"),
            path: format!("/Users/ejemplo/Carpeta{index}"),
            bookmark_base64: "Ym9va21hcms=".into(),
        }
    }

    #[test]
    fn folders_survive_reopening_encrypted_and_none_is_an_empty_list() -> Result<()> {
        let directory = tempfile::tempdir()?;
        {
            let mut store = Store::open(directory.path(), [7; 32])?;
            assert_eq!(store.load_assistant_folders()?, Vec::new());
            store.save_assistant_folders(&[folder(1), folder(2)])?;
        }
        let store = Store::open(directory.path(), [7; 32])?;
        assert_eq!(store.load_assistant_folders()?, vec![folder(1), folder(2)]);
        // The path is not in the database in clear.
        let raw: Vec<u8> = store.connection.query_row(
            "SELECT value FROM settings WHERE key = ?1",
            [KEY],
            |row| row.get(0),
        )?;
        assert!(!raw.windows(8).any(|window| window == b"/Users/e"));
        Ok(())
    }

    #[test]
    fn the_longest_list_fits_its_row() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let mut store = Store::open(directory.path(), [7; 32])?;
        let longest: Vec<Folder> = (0..MAX_FOLDERS)
            .map(|index| Folder {
                id: format!("00000000-0000-4000-8000-{index:012}"),
                path: format!("/{}", "\u{01}".repeat(MAX_PATH_BYTES - 1)),
                bookmark_base64: "A".repeat(MAX_BOOKMARK_BYTES.div_ceil(3) * 4),
            })
            .collect();
        store.save_assistant_folders(&longest)?;
        assert_eq!(store.load_assistant_folders()?, longest);
        Ok(())
    }
}
