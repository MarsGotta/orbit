//! Pure wire projection: a response that does not fit is refused with its
//! size, never cut. Evidence and authority stay exact.
use super::Reply;
use anyhow::{Context, Result};
use orbit_core::{
    limits::MAX_COMMAND_RESPONSE_BYTES,
    protocol::{Response, TopicEvent, TopicResponse},
    refusal::{Refusal, Resource},
    topics::{Knowledge, TOPIC_ENVELOPE_BYTES, TopicData, TopicFrame, transport_max_bytes},
};
use serde::Serialize;
use std::io::{self, Write};

pub(super) const MAX_VOICE_NOTE_RESPONSE_FRAME_BYTES: usize = 12 * 1024 * 1024;

pub(super) fn encode_reply(reply: Reply) -> Result<Vec<u8>> {
    match reply {
        Reply::Command(response) => encode_response(response),
        Reply::VoiceNoteAudio(response) => {
            encode_with_limit(&response, MAX_VOICE_NOTE_RESPONSE_FRAME_BYTES)
        }
        Reply::Topics(response) => encode_topic_response(response),
    }
}

fn encode_topic_response(response: TopicResponse) -> Result<Vec<u8>> {
    let limit = response.read.map_or(transport_max_bytes(), |read| {
        read.max_bytes() + TOPIC_ENVELOPE_BYTES
    });
    encode_with_limit(&bound_topic_response(response), limit)
}

/// The producer applies the same page budget before either transport sees it.
/// Size includes the existing JSON envelope and newline so socket refusals keep
/// their exact measurements even when the consumer reads the typed value.
pub fn bound_topic_response(mut response: TopicResponse) -> TopicResponse {
    let limit = response.read.map_or(transport_max_bytes(), |read| {
        read.max_bytes() + TOPIC_ENVELOPE_BYTES
    });
    if let Some(refusal) = size_refusal(&response, limit, "read.response") {
        response.answer = Some(Knowledge::Absent { refusal });
    }
    response
}

pub(super) fn encode_topic_frame(frame: &TopicFrame) -> Result<Vec<u8>> {
    let frame = bound_topic_frame(frame.clone());
    let limit = frame.id().max_bytes() + TOPIC_ENVELOPE_BYTES;
    encode_with_limit(&TopicEvent::Changed { frame }, limit)
}

/// Bound the published watch value, without a JSON round trip or truncation.
pub fn bound_topic_frame(mut frame: TopicFrame) -> TopicFrame {
    let limit = frame.id().max_bytes() + TOPIC_ENVELOPE_BYTES;
    let event = TopicEvent::Changed {
        frame: frame.clone(),
    };
    if let Some(refusal) = size_refusal(&event, limit, "topic.frame") {
        frame.data = TopicData::absent(frame.id(), refusal);
    }
    frame
}

fn size_refusal(value: &impl Serialize, limit: usize, field: &str) -> Option<Refusal> {
    match json_size(value).and_then(|size| size.checked_add(1).context("IPC size overflow")) {
        Ok(size) if size > limit => Some(Refusal::TooLarge {
            resource: Resource::IpcResponse,
            bytes: size as u64,
            max_bytes: limit as u64,
        }),
        Ok(_) => None,
        Err(error) => Some(Refusal::DaemonReplyInvalid {
            field: field.into(),
            issue: orbit_core::refusal::DaemonReplyIssue::Decode {
                error: error.to_string(),
            },
        }),
    }
}

fn encode_with_limit(value: &impl Serialize, limit: usize) -> Result<Vec<u8>> {
    let size = json_size(value)?
        .checked_add(1)
        .context("IPC size overflow")?;
    if size > limit {
        return Err(Refusal::TooLarge {
            resource: Resource::IpcResponse,
            bytes: size as u64,
            max_bytes: limit as u64,
        }
        .into());
    }
    let mut bytes = serde_json::to_vec(value)?;
    bytes.push(b'\n');
    Ok(bytes)
}

/// Counts exact JSON bytes without allocating a serialization buffer. Escaping
/// and UTF-8 are handled by the same serializer that writes the final frame.
#[derive(Default)]
struct JsonCount(usize);

impl Write for JsonCount {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0 = self
            .0
            .checked_add(bytes.len())
            .ok_or_else(|| io::Error::other("JSON size overflow"))?;
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn json_size(value: &impl Serialize) -> Result<usize> {
    let mut count = JsonCount::default();
    serde_json::to_writer(&mut count, value)?;
    Ok(count.0)
}

/// Includes the envelope and final newline in the existing 1 MiB limit. This
/// takes the already detached oneshot response; no caller state is mutated.
pub(super) fn encode_response(mut response: Response) -> Result<Vec<u8>> {
    let bytes = json_size(&response)?
        .checked_add(1)
        .context("IPC size overflow")?;
    if bytes > MAX_COMMAND_RESPONSE_BYTES
        && matches!(
            response.refusal,
            Some(Refusal::InvalidParameter {
                parameter: orbit_core::refusal::Parameter::Identifier,
                ..
            })
        )
    {
        // An invalid identity near the input ceiling cannot also fit its refusal.
        // Keep its measured size in the cause; no valid correlation ID is lost.
        response.id.clear();
        return encode_with_limit(&response, MAX_COMMAND_RESPONSE_BYTES);
    }
    if bytes > MAX_COMMAND_RESPONSE_BYTES {
        let refusal = Refusal::TooLarge {
            resource: Resource::IpcResponse,
            bytes: bytes as u64,
            max_bytes: MAX_COMMAND_RESPONSE_BYTES as u64,
        };
        response = Response {
            v: response.v,
            id: response.id,
            ok: false,
            status: None,
            error: Some(refusal.to_string()),
            refusal: Some(refusal),
            epoch: None,
            revisions: Some(std::collections::BTreeMap::new()),
        };
    }
    encode_with_limit(&response, MAX_COMMAND_RESPONSE_BYTES)
}
