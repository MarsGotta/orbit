//! A meeting going up: each chunk (`PUT v1/chunks/{id}`) and its closure
//! (`POST v1/recordings/{id}/finalize`), each confirmed only by a durable
//! receipt, and how the hub's answer to an upload reads (a refusal, its code,
//! a hub out of space). When to send and how to retry are the caller's.
use super::*;
use orbit_core::model::ChunkReceipt;

/// Where a chunk goes and who sends it.
pub struct ChunkPut {
    pub base: reqwest::Url,
    pub token: String,
    pub receipt: ChunkReceipt,
    pub bytes: Vec<u8>,
}

/// One chunk to the hub, confirmed only by a durable receipt for the same
/// bytes.
pub async fn put_chunk(put: ChunkPut, op: &Op) -> Result<()> {
    let ChunkPut {
        base,
        token,
        receipt,
        bytes,
    } = put;
    let metadata = &receipt.metadata;
    let mut request = super::request(
        reqwest::Method::PUT,
        base.join(&format!("v1/chunks/{}", metadata.id))?,
        Some(&token),
        op,
    )?
    .header("Content-Type", "audio/wav")
    .header("X-Orbit-Recording", &metadata.recording_id)
    .header("X-Orbit-Track", &metadata.track)
    .header("X-Orbit-Sequence", metadata.sequence)
    .header("X-Orbit-Started-At-Ms", metadata.started_at_ms)
    .header("X-Orbit-Monotonic-Ns", metadata.monotonic_ns)
    .header("X-Orbit-First-Sample", metadata.first_sample)
    .header("X-Orbit-Frames", metadata.frames)
    .header("X-Orbit-Sample-Rate", metadata.sample_rate)
    .header("X-Orbit-Channels", metadata.channels)
    .header("X-Orbit-SHA256", &receipt.sha256);
    if let Some(epoch) = &metadata.capture_epoch_id {
        request = request.header("X-Orbit-Capture-Epoch-Id", epoch);
    }
    let response = check_response(request.body(bytes).send().await?).await?;
    let ack = bounded_json(response).await?;
    if ack["durable"] != true || ack["sha256"] != receipt.sha256 || ack["chunk_id"] != metadata.id {
        bail!("The server did not confirm a valid durable receipt");
    }
    Ok(())
}

/// The closure to the hub, confirmed only by its durable receipt.
pub async fn post_closure(
    base: &reqwest::Url,
    token: &str,
    recording_id: &str,
    manifest: &Value,
    op: &Op,
) -> Result<()> {
    let response = request(
        reqwest::Method::POST,
        base.join(&format!("v1/recordings/{recording_id}/finalize"))?,
        Some(token),
        op,
    )?
    .json(manifest)
    .send()
    .await?;
    let response = check_response(response).await?;
    let receipt: Value = bounded_json(response).await?;
    if receipt["durable"] != true
        || receipt["finalized"] != true
        || receipt["recording_id"] != recording_id
    {
        bail!("Invalid finalization receipt");
    }
    Ok(())
}

#[derive(Debug, thiserror::Error)]
#[error("Transfer refused ({0}); retained locally for review")]
pub struct Refused(pub u16);

/// The hub's own code for a refusal (`recording_finalized`,
/// `chunk_conflict`, `unauthorized`…), kept beside [`Refused`] in the error
/// chain so the person is told which rule fired, not only the HTTP status.
#[derive(Debug, thiserror::Error)]
#[error("Hub code: {0}")]
pub struct HubCode(pub String);

/// The hub has no room for this upload (HTTP 507): its code and the
/// numbers it gave. The audio stays on the Mac; nothing is lost, it waits.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("The hub has no room ({code}): audio stays on the Mac until space is available")]
pub struct OutOfSpace {
    /// `quota_exceeded`, `storage_full`…, or `insufficient_storage` (the
    /// status's own name) when the hub's body named none.
    pub code: String,
    pub used_bytes: Option<u64>,
    pub max_bytes: Option<u64>,
    pub free_bytes: Option<u64>,
}

impl OutOfSpace {
    fn unexplained() -> Self {
        Self {
            code: "insufficient_storage".into(),
            used_bytes: None,
            max_bytes: None,
            free_bytes: None,
        }
    }
}

pub fn check_status(status: reqwest::StatusCode) -> anyhow::Result<()> {
    if status.is_success() {
        return Ok(());
    }
    if matches!(
        status.as_u16(),
        400 | 401 | 403 | 404 | 409 | 413 | 415 | 422
    ) {
        return Err(Refused(status.as_u16()).into());
    }
    if status == reqwest::StatusCode::INSUFFICIENT_STORAGE {
        return Err(OutOfSpace::unexplained().into());
    }
    anyhow::bail!("Transfer remains pending after HTTP {status}")
}

/// [`check_status`] for an upload, with what the hub said: a refusal keeps
/// the hub's code ([`HubCode`]) and a hub out of space its numbers. A body
/// that cannot be read leaves the status alone, never a guess.
pub async fn check_response(response: reqwest::Response) -> anyhow::Result<reqwest::Response> {
    let status = response.status();
    let Err(error) = check_status(status) else {
        return Ok(response);
    };
    let Ok(said) = read_refusal(response, MAX_HUB_HTTP_BODY_BYTES).await else {
        return Err(error);
    };
    let code = said
        .code
        .as_deref()
        .or_else(|| said.details["code"].as_str())
        .and_then(hub_code)
        .map(str::to_owned);
    if error.is::<OutOfSpace>() {
        return Err(OutOfSpace {
            code: code.unwrap_or_else(|| OutOfSpace::unexplained().code),
            used_bytes: said.numbers.get("used_bytes").copied(),
            max_bytes: said.numbers.get("max_bytes").copied(),
            // What is left: of the quota (`remaining_bytes`), or of the
            // disk that refused the write (`storage_full`'s `free_bytes`).
            free_bytes: said
                .numbers
                .get("remaining_bytes")
                .or_else(|| said.numbers.get("free_bytes"))
                .copied(),
        }
        .into());
    }
    match code {
        Some(code) => Err(error.context(HubCode(code))),
        None => Err(error),
    }
}
