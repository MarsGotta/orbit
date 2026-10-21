use crate::{ChunkMetadata, Error, Job, JobState, MAX_PAGE_SIZE, MAX_PAYLOAD_BYTES, Result};
use serde::Serialize;
use std::{fs, io::Write, path::Path};
pub(crate) fn context(kind: &str, id: &str) -> Vec<u8> {
    format!("orbit:{kind}:v1:{id}").into_bytes()
}
pub(super) fn validate_id(id: &str) -> Result<()> {
    if id.is_empty()
        || id.len() > 128
        || !id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b':'))
        || id == "."
        || id == ".."
    {
        return Err(Error::Invalid("record identifier"));
    }
    Ok(())
}
pub(super) fn validate_metadata(metadata: &ChunkMetadata) -> Result<()> {
    validate_id(&metadata.id)?;
    if metadata.id.len() > 121 {
        return Err(Error::Invalid("chunk identifier"));
    }
    validate_id(&metadata.recording_id)?;
    validate_id(&metadata.track)?;
    if let Some(epoch) = &metadata.capture_epoch_id {
        validate_id(epoch)?;
    }
    if !(8_000..=192_000).contains(&metadata.sample_rate)
        || !(1..=8).contains(&metadata.channels)
        || metadata.frames == 0
        || metadata.sequence > i64::MAX as u64
        || metadata.frames > i64::MAX as u64
    {
        return Err(Error::Invalid("chunk format"));
    }
    Ok(())
}
pub(super) fn validate_page(limit: usize) -> Result<()> {
    if limit == 0 || limit > MAX_PAGE_SIZE {
        return Err(Error::CountOutOfRange {
            what: "Número de filas de la página",
            count: limit,
            min_count: 1,
            max_count: MAX_PAGE_SIZE,
        });
    }
    Ok(())
}
pub(super) fn job_bytes(job: &Job) -> Result<Vec<u8>> {
    let mut canonical = job.clone();
    canonical.state = JobState::Queued;
    bounded_json(&canonical, MAX_PAYLOAD_BYTES)
}
pub(crate) fn bounded_json<T: Serialize>(value: &T, limit: usize) -> Result<Vec<u8>> {
    struct BoundedWriter {
        bytes: Vec<u8>,
        limit: usize,
        size: u64,
    }
    impl Write for BoundedWriter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.size = self.size.saturating_add(bytes.len() as u64);
            if self.size <= self.limit as u64 {
                self.bytes.extend_from_slice(bytes);
            }
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut writer = BoundedWriter {
        bytes: Vec::new(),
        limit,
        size: 0,
    };
    let result = serde_json::to_writer(&mut writer, value);
    if writer.size > limit as u64 {
        return Err(Error::TooLarge {
            what: "El contenido serializado",
            bytes: writer.size,
            max_bytes: limit as u64,
        });
    }
    result?;
    Ok(writer.bytes)
}
pub(super) fn regular_file_if_exists(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if !metadata.file_type().is_file() => Err(Error::UnsafePath),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}
pub(super) fn private_directory(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if !metadata.file_type().is_dir() => return Err(Error::UnsafePath),
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let mut builder = fs::DirBuilder::new();
            builder.recursive(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                builder.mode(0o700);
            }
            builder.create(path)?;
        }
        Err(error) => return Err(error.into()),
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}
pub(super) fn private_file(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

/// Inspect a setting's stored length without materializing an oversized value.
/// CASE keeps SQLite from loading the blob, and checks length and value in one
/// statement so a concurrent writer cannot replace it between two reads.
pub(crate) fn bounded_setting(
    connection: &rusqlite::Connection,
    key: &str,
    max_bytes: usize,
    what: &'static str,
) -> Result<Option<Vec<u8>>> {
    use rusqlite::OptionalExtension;
    let stored = connection
        .query_row(
            "SELECT length(value), CASE WHEN length(value) <= ?2 THEN value END
         FROM settings WHERE key = ?1",
            rusqlite::params![key, max_bytes as i64],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, Option<Vec<u8>>>(1)?)),
        )
        .optional()?;
    let Some((length, payload)) = stored else {
        return Ok(None);
    };
    let bytes = u64::try_from(length).map_err(|_| Error::Invalid("stored setting length"))?;
    if bytes > max_bytes as u64 {
        return Err(Error::TooLarge {
            what,
            bytes,
            max_bytes: max_bytes as u64,
        });
    }
    payload
        .map(Some)
        .ok_or(Error::Invalid("stored setting payload"))
}

/// Bound the Rust copy of a blob SQLite has already materialized. Settings
/// that must check length before loading use `bounded_setting` instead.
/// The caller supplies its format's ceiling, including the envelope.
pub(crate) fn bounded_blob(
    row: &rusqlite::Row<'_>,
    index: usize,
    max_bytes: usize,
    what: &'static str,
) -> rusqlite::Result<Vec<u8>> {
    let value = row.get_ref(index)?;
    let column_name = row.as_ref().column_name(index)?;
    let bytes = value.as_blob().map_err(|_| {
        rusqlite::Error::InvalidColumnType(index, column_name.to_owned(), value.data_type())
    })?;
    if bytes.len() > max_bytes {
        return Err(rusqlite::Error::FromSqlConversionFailure(
            index,
            rusqlite::types::Type::Blob,
            Box::new(Error::TooLarge {
                what,
                bytes: bytes.len() as u64,
                max_bytes: max_bytes as u64,
            }),
        ));
    }
    Ok(bytes.to_vec())
}

/// Authenticate the immutable identity and every supplied SQL index. Callers
/// without a kind or timestamp index cannot use those columns as authority.
pub(crate) fn decode_job(
    cipher: &crate::crypto::Cipher,
    id: &str,
    state: Option<&str>,
    kind: Option<&str>,
    created_at: Option<i64>,
    payload: &[u8],
) -> Result<Job> {
    let job: Job = serde_json::from_slice(&cipher.decrypt(&context("job", id), payload)?)?;
    if job.id != id
        || state
            .map(JobState::parse)
            .transpose()?
            .is_some_and(|state| job.state != state)
        || kind.is_some_and(|kind| job.kind != kind)
        || created_at.is_some_and(|created_at| job.created_at != created_at)
    {
        return Err(Error::Authentication);
    }
    Ok(job)
}
