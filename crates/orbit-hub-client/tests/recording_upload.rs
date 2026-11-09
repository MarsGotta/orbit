//! A recording's chunk and its closure, as the client sends them to the hub:
//! the method, the route, the headers and the body of today. An answer that
//! is not the hub's durable receipt for those same bytes is an error, and a
//! refusal keeps what the daemon reads from it (status, code, numbers). A
//! synthetic hub on loopback takes each request; nothing here is evidence
//! about the real hub.
use anyhow::{Context, Result};
use orbit_core::model::{ChunkMetadata, ChunkReceipt};
use orbit_core::notice::Op;
use orbit_hub_client::hub::{self, ChunkPut, HubCode, OutOfSpace, Refused};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::task::JoinHandle;

const TOKEN: &str = "synthetic-not-a-credential";
const RECORDING: &str = "5b0e8c2a-3f41-4d6e-9a7b-2c8d1e0f4a63";
const OTHER_RECORDING: &str = "9d4c2b1a-0e8f-4a7b-8c6d-5e4f3a2b1c0d";
const CHUNK: &str = "5b0e8c2a-3f41-4d6e-9a7b-2c8d1e0f4a63-mic_local-7";
/// The digest the Mac recorded for the chunk. Invented: the client sends the
/// one it is given and never hashes the bytes itself.
const SHA256: &str = "4f2d8a61c09b7e35d1a6f8c24b90e7d35a1c6f08e2b49d73c5a1e6f9082b4d7c";
const OTHER_SHA256: &str = "0b9e7c5a3f1d2e4b6a8c0d9e7f5a3b1c2d4e6f8a0b9c7d5e3f1a2b4c6d8e0f9a";
const EPOCH: &str = "epoca-sintetica-2";
const AUDIO: &[u8] = b"RIFF sintetico: no es audio de nadie";
const CHUNK_UNCONFIRMED: &str = "The server did not confirm a valid durable receipt";
const CLOSURE_UNCONFIRMED: &str = "Invalid finalization receipt";

/// One request as the synthetic hub read it, header names in lower case.
struct Received {
    method: String,
    path: String,
    headers: BTreeMap<String, String>,
    body: Vec<u8>,
}

impl Received {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).map(String::as_str)
    }

    /// The `X-Orbit-*` headers, the hub's own contract, and nothing else.
    fn orbit_headers(&self) -> BTreeMap<String, String> {
        self.headers
            .iter()
            .filter(|(name, _)| name.starts_with("x-orbit-"))
            .map(|(name, value)| (name.clone(), value.clone()))
            .collect()
    }
}

/// A hub that takes one request, answers it with `status` and the JSON
/// `answer`, and hands the test what it received.
async fn hub_answering(
    status: &'static str,
    answer: Value,
) -> Result<(reqwest::Url, JoinHandle<Result<Received>>)> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let base = reqwest::Url::parse(&format!("http://{}/", listener.local_addr()?))?;
    let hub = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await?;
        let mut bytes = Vec::new();
        let mut buffer = [0_u8; 4096];
        let head_end = loop {
            if let Some(at) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
                break at + 4;
            }
            let read = socket.read(&mut buffer).await?;
            anyhow::ensure!(read > 0, "the request ended before its headers");
            bytes.extend_from_slice(&buffer[..read]);
        };
        let head = String::from_utf8(bytes[..head_end].to_vec())?;
        let mut lines = head.split("\r\n");
        let mut start = lines.next().context("no request line")?.split(' ');
        let method = start.next().context("no method")?.to_owned();
        let path = start.next().context("no path")?.to_owned();
        let mut headers = BTreeMap::new();
        for line in lines.filter(|line| !line.is_empty()) {
            let (name, value) = line.split_once(':').context("a header without a colon")?;
            let name = name.trim().to_ascii_lowercase();
            let repeated = headers.insert(name.clone(), value.trim().to_owned());
            anyhow::ensure!(repeated.is_none(), "the header {name} came twice");
        }
        anyhow::ensure!(
            !headers.contains_key("transfer-encoding"),
            "the body came without its length"
        );
        let length: usize = match headers.get("content-length") {
            Some(length) => length.parse()?,
            None => 0,
        };
        while bytes.len() < head_end + length {
            let read = socket.read(&mut buffer).await?;
            anyhow::ensure!(read > 0, "the body was cut");
            bytes.extend_from_slice(&buffer[..read]);
        }
        let body = bytes[head_end..head_end + length].to_vec();
        let answer = answer.to_string();
        socket
            .write_all(
                format!(
                    "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{answer}",
                    answer.len()
                )
                .as_bytes(),
            )
            .await?;
        Ok(Received {
            method,
            path,
            headers,
            body,
        })
    });
    Ok((base, hub))
}

fn new_op() -> Op {
    uuid::Uuid::new_v4().into()
}

fn receipt(epoch: Option<&str>) -> ChunkReceipt {
    ChunkReceipt {
        metadata: ChunkMetadata {
            id: CHUNK.into(),
            recording_id: RECORDING.into(),
            track: "mic_local".into(),
            capture_epoch_id: epoch.map(str::to_owned),
            sequence: 7,
            sample_rate: 48_000,
            channels: 1,
            frames: 48_000,
            monotonic_ns: 987_654_321_000,
            first_sample: 336_000,
            started_at_ms: 1_767_225_607_000,
        },
        bytes: AUDIO.len() as u64,
        sha256: SHA256.into(),
    }
}

/// The `X-Orbit-*` headers a chunk carries today: the op, its capture facts
/// and its digest, and its capture epoch only when it has one.
fn chunk_headers(op: &Op, epoch: Option<&str>) -> BTreeMap<String, String> {
    let mut headers: BTreeMap<String, String> = [
        ("x-orbit-op", op.as_str()),
        ("x-orbit-recording", RECORDING),
        ("x-orbit-track", "mic_local"),
        ("x-orbit-sequence", "7"),
        ("x-orbit-started-at-ms", "1767225607000"),
        ("x-orbit-monotonic-ns", "987654321000"),
        ("x-orbit-first-sample", "336000"),
        ("x-orbit-frames", "48000"),
        ("x-orbit-sample-rate", "48000"),
        ("x-orbit-channels", "1"),
        ("x-orbit-sha256", SHA256),
    ]
    .into_iter()
    .map(|(name, value)| (name.to_owned(), value.to_owned()))
    .collect();
    if let Some(epoch) = epoch {
        headers.insert("x-orbit-capture-epoch-id".into(), epoch.into());
    }
    headers
}

/// The chunk to a hub that answers `status` and `answer`: what the client
/// returned, what the hub received and the op the request was sent with.
async fn send_chunk(
    status: &'static str,
    answer: Value,
    epoch: Option<&str>,
) -> Result<(Result<()>, Received, Op)> {
    let (base, hub) = hub_answering(status, answer).await?;
    let op = new_op();
    let put = ChunkPut {
        base,
        token: TOKEN.into(),
        receipt: receipt(epoch),
        bytes: AUDIO.to_vec(),
    };
    let result = hub::put_chunk(put, &op).await;
    Ok((result, hub.await??, op))
}

fn manifest() -> Value {
    json!({
        "recording_id": RECORDING,
        "title": "Revisión del huerto comunitario",
        "tracks": [{
            "track": "mic_local",
            "chunk_count": 8,
            "last_sequence": 7,
            "total_frames": 384_000,
        }],
    })
}

/// The closure to a hub that answers `status` and `answer`.
async fn send_closure(status: &'static str, answer: Value) -> Result<(Result<()>, Received, Op)> {
    let (base, hub) = hub_answering(status, answer).await?;
    let op = new_op();
    let result = hub::post_closure(&base, TOKEN, RECORDING, &manifest(), &op).await;
    Ok((result, hub.await??, op))
}

#[tokio::test]
async fn a_chunk_goes_up_with_todays_method_route_headers_and_body() -> Result<()> {
    let durable = json!({"chunk_id": CHUNK, "sha256": SHA256, "durable": true});
    let (result, received, op) = send_chunk("200 OK", durable, Some(EPOCH)).await?;
    result?;
    assert_eq!(received.method, "PUT");
    assert_eq!(received.path, format!("/v1/chunks/{CHUNK}"));
    let bearer = format!("Bearer {TOKEN}");
    assert_eq!(received.header("authorization"), Some(bearer.as_str()));
    assert_eq!(received.header("content-type"), Some("audio/wav"));
    assert_eq!(received.orbit_headers(), chunk_headers(&op, Some(EPOCH)));
    assert_eq!(received.body, AUDIO);
    Ok(())
}

#[tokio::test]
async fn a_chunk_without_a_capture_epoch_sends_no_epoch_header() -> Result<()> {
    let durable = json!({"chunk_id": CHUNK, "sha256": SHA256, "durable": true});
    let (result, received, op) = send_chunk("200 OK", durable, None).await?;
    result?;
    assert_eq!(received.orbit_headers(), chunk_headers(&op, None));
    Ok(())
}

#[tokio::test]
async fn a_chunk_receipt_that_is_not_durable_is_an_error() -> Result<()> {
    for answer in [
        json!({"chunk_id": CHUNK, "sha256": SHA256, "durable": false}),
        json!({"chunk_id": CHUNK, "sha256": SHA256, "durable": "true"}),
        json!({"chunk_id": CHUNK, "sha256": SHA256}),
    ] {
        let (result, _, _) = send_chunk("200 OK", answer.clone(), Some(EPOCH)).await?;
        let error = result
            .err()
            .with_context(|| format!("{answer} confirmed the chunk"))?;
        assert_eq!(error.to_string(), CHUNK_UNCONFIRMED, "{answer}");
    }
    Ok(())
}

#[tokio::test]
async fn a_chunk_receipt_for_other_bytes_or_another_chunk_is_an_error() -> Result<()> {
    for answer in [
        json!({"chunk_id": CHUNK, "sha256": OTHER_SHA256, "durable": true}),
        json!({"chunk_id": CHUNK, "sha256": SHA256.to_ascii_uppercase(), "durable": true}),
        json!({"chunk_id": format!("{RECORDING}-mic_local-8"), "sha256": SHA256, "durable": true}),
    ] {
        let (result, _, _) = send_chunk("200 OK", answer.clone(), Some(EPOCH)).await?;
        let error = result
            .err()
            .with_context(|| format!("{answer} confirmed the chunk"))?;
        assert_eq!(error.to_string(), CHUNK_UNCONFIRMED, "{answer}");
    }
    Ok(())
}

/// What the daemon reads to keep the chunk and say why: the status and the
/// hub's code, in the error chain, with the client's own types.
#[tokio::test]
async fn a_refused_chunk_keeps_its_status_and_the_hubs_code() -> Result<()> {
    let refusal = json!({"code": "chunk_conflict", "message": "Ya existe con otro contenido"});
    let (result, _, _) = send_chunk("409 Conflict", refusal, Some(EPOCH)).await?;
    let error = result.err().context("a 409 confirmed the chunk")?;
    let status = error.downcast_ref::<Refused>().map(|refused| refused.0);
    assert_eq!(status, Some(409), "{error:#}");
    let code = error.downcast_ref::<HubCode>().map(|code| code.0.as_str());
    assert_eq!(code, Some("chunk_conflict"), "{error:#}");
    Ok(())
}

#[tokio::test]
async fn a_closure_goes_up_with_todays_method_route_headers_and_body() -> Result<()> {
    let durable = json!({
        "recording_id": RECORDING,
        "finalized": true,
        "durable": true,
        "capture_complete": false,
        "transcript_ready": false,
    });
    let (result, received, op) = send_closure("200 OK", durable).await?;
    result?;
    assert_eq!(received.method, "POST");
    assert_eq!(
        received.path,
        format!("/v1/recordings/{RECORDING}/finalize")
    );
    let bearer = format!("Bearer {TOKEN}");
    assert_eq!(received.header("authorization"), Some(bearer.as_str()));
    assert_eq!(received.header("content-type"), Some("application/json"));
    let only_op = BTreeMap::from([("x-orbit-op".to_owned(), op.as_str().to_owned())]);
    assert_eq!(received.orbit_headers(), only_op);
    assert_eq!(serde_json::from_slice::<Value>(&received.body)?, manifest());
    Ok(())
}

#[tokio::test]
async fn a_closure_receipt_that_is_not_durable_or_not_finalized_is_an_error() -> Result<()> {
    for answer in [
        json!({"recording_id": RECORDING, "finalized": true, "durable": false}),
        json!({"recording_id": RECORDING, "finalized": false, "durable": true}),
        json!({"recording_id": RECORDING, "finalized": "true", "durable": true}),
        json!({"recording_id": RECORDING, "finalized": true}),
        json!({"recording_id": RECORDING, "durable": true}),
    ] {
        let (result, _, _) = send_closure("200 OK", answer.clone()).await?;
        let error = result
            .err()
            .with_context(|| format!("{answer} confirmed the closure"))?;
        assert_eq!(error.to_string(), CLOSURE_UNCONFIRMED, "{answer}");
    }
    Ok(())
}

#[tokio::test]
async fn a_closure_receipt_for_another_recording_is_an_error() -> Result<()> {
    for answer in [
        json!({"recording_id": OTHER_RECORDING, "finalized": true, "durable": true}),
        json!({"finalized": true, "durable": true}),
    ] {
        let (result, _, _) = send_closure("200 OK", answer.clone()).await?;
        let error = result
            .err()
            .with_context(|| format!("{answer} confirmed the closure"))?;
        assert_eq!(error.to_string(), CLOSURE_UNCONFIRMED, "{answer}");
    }
    Ok(())
}

/// A hub with no room says so with its numbers, which reach the person; the
/// closure is not confirmed.
#[tokio::test]
async fn a_hub_out_of_space_for_a_closure_says_its_numbers() -> Result<()> {
    let full = json!({
        "message": "Cuota",
        "code": "quota_exceeded",
        "refusal": {
            "code": "quota_exceeded",
            "used_bytes": 10_737_000_000_u64,
            "max_bytes": 10_737_418_240_u64,
            "remaining_bytes": 418_240,
        },
    });
    let (result, _, _) = send_closure("507 Insufficient Storage", full).await?;
    let error = result.err().context("a 507 confirmed the closure")?;
    let expected = OutOfSpace {
        code: "quota_exceeded".into(),
        used_bytes: Some(10_737_000_000),
        max_bytes: Some(10_737_418_240),
        free_bytes: Some(418_240),
    };
    assert_eq!(
        error.downcast_ref::<OutOfSpace>(),
        Some(&expected),
        "{error:#}"
    );
    Ok(())
}
