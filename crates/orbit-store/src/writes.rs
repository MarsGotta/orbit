//! Shared writes accept the owning connection or its transaction. No nested transactions: the caller chooses the complete atomic durability boundary.
use crate::validation::{
    bounded_blob, bounded_json, bounded_setting, context, decode_job, job_bytes, validate_id,
};
use crate::{
    Error, Job, MAX_PAYLOAD_BYTES, Result,
    crypto::{Cipher, OVERHEAD},
    migrations::record_namespace,
};
use rusqlite::{Connection, OptionalExtension, params};
pub(super) fn snapshot(connection: &Connection, cipher: &Cipher, bytes: &[u8]) -> Result<()> {
    // Authenticate before comparison. Unchanged snapshots must not rewrite the conversation cache, and a corrupt previous value must never be ignored.
    let previous = bounded_setting(
        connection,
        "snapshot",
        MAX_PAYLOAD_BYTES + OVERHEAD,
        "The stored snapshot",
    )?;
    if let Some(previous) = previous
        && cipher.decrypt(b"orbit:snapshot:v1", &previous)? == bytes
    {
        return Ok(());
    }
    let payload = cipher.encrypt(b"orbit:snapshot:v1", bytes)?;
    connection.execute(
        "INSERT INTO settings (key, value) VALUES ('snapshot', ?1)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        [payload],
    )?;
    Ok(())
}
pub(super) fn enqueue_job(connection: &Connection, cipher: &Cipher, job: &Job) -> Result<()> {
    validate_id(&job.id)?;
    validate_id(&job.kind)?;
    let bytes = job_bytes(job)?;
    let context = context("job", &job.id);
    let namespace = record_namespace(&job.kind);
    let select = if namespace.is_some() {
        "SELECT kind,state,created_at,payload,length(payload) FROM records WHERE namespace=?1 AND id=?2"
    } else {
        "SELECT kind,state,created_at,payload,length(payload) FROM jobs WHERE ?1 IS NULL AND id=?2"
    };
    let previous = connection
        .query_row(select, params![namespace, job.id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                bounded_blob(row, 3, MAX_PAYLOAD_BYTES + OVERHEAD, "The stored job")?,
            ))
        })
        .optional()?;
    if let Some((kind, state, created_at, payload)) = previous {
        let existing = decode_job(
            cipher,
            &job.id,
            Some(&state),
            Some(&kind),
            Some(created_at),
            &payload,
        )?;
        return if job_bytes(&existing)? == bytes {
            Ok(())
        } else {
            Err(Error::Conflict)
        };
    }
    let payload = cipher.encrypt(&context, &bounded_json(job, MAX_PAYLOAD_BYTES)?)?;
    if let Some(namespace) = namespace {
        connection.execute(
            "INSERT INTO records(namespace,id,kind,state,created_at,payload) VALUES(?1,?2,?3,?4,?5,?6)",
            params![namespace, job.id, job.kind, job.state.as_str(), job.created_at, payload],
        )?;
    } else {
        connection.execute(
            "INSERT INTO jobs (id, kind, state, created_at, payload) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                job.id,
                job.kind,
                job.state.as_str(),
                job.created_at,
                payload
            ],
        )?;
    }
    Ok(())
}
