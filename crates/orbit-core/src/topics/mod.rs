//! Fixed, independently versioned facts published by the daemon.
use crate::refusal::Refusal;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub mod agenda;
pub mod assistant;
pub mod control;
pub mod notes;
pub mod notices;
pub mod permissions;
pub mod recording;
pub mod status;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
#[must_use]
pub enum Knowledge<T> {
    Unknown { why: Pending },
    Absent { refusal: Refusal },
    Present { value: T, as_of_ms: Option<i64> },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Pending {
    NotObservedYet,
    HelperStarting,
    Reading {
        since_ms: i64,
    },
    Transcription {
        completed: u64,
        pending: u64,
        failed: u64,
        queued: bool,
    },
    DaemonStarting,
}

macro_rules! topics {
    ($($variant:ident => $module:ident;)+) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(rename_all = "snake_case")]
        pub enum TopicId { $($variant,)+ }

        impl TopicId {
            pub const ALL: &'static [Self] = &[$(Self::$variant,)+];

            pub const fn max_bytes(self) -> usize {
                match self { $(Self::$variant => $module::MAX_BYTES,)+ }
            }

            pub const fn min_interval_ms(self) -> u64 {
                match self { $(Self::$variant => $module::MIN_INTERVAL_MS,)+ }
            }
        }

        #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
        #[serde(tag = "topic", content = "knowledge", rename_all = "snake_case")]
        pub enum TopicData { $($variant(Knowledge<$module::Value>),)+ }

        impl TopicData {
            pub fn id(&self) -> TopicId {
                match self { $(Self::$variant(_) => TopicId::$variant,)+ }
            }

            pub fn not_observed(id: TopicId) -> Self {
                match id {
                    $(TopicId::$variant => Self::$variant(Knowledge::Unknown {
                        why: Pending::NotObservedYet,
                    }),)+
                }
            }

            pub fn absent(id: TopicId, refusal: Refusal) -> Self {
                match id {
                    $(TopicId::$variant => Self::$variant(Knowledge::Absent { refusal }),)+
                }
            }
        }
    };
}

// Each topic's ceiling and redraw interval are its domain's, beside its
// value (`MAX_BYTES`, `MIN_INTERVAL_MS`); large collections belong in paged reads.
topics! {
    Status => status;
    Permissions => permissions;
    Recording => recording;
    Agenda => agenda;
    Notes => notes;
    Assistant => assistant;
    Control => control;
    Notices => notices;
}

/// Four per second protects the actor from ~94 audio events per second on two
/// tracks. A meter changes perceptibly at 250 ms; this is no audio intake limit.
pub const RECORDING_INTERVAL_MS: u64 = TopicId::Recording.min_interval_ms();

pub type TopicSet = BTreeSet<TopicId>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TopicFrame {
    pub epoch: String,
    pub revision: u64,
    pub data: TopicData,
}

impl TopicFrame {
    pub fn id(&self) -> TopicId {
        self.data.id()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionOrigin {
    pub session_id: String,
    pub agent_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Page<T> {
    pub items: Vec<T>,
    pub next: Option<Cursor>,
    pub total: u64,
    pub revision: u64,
    /// The daemon's confirmed storage version when this page's local content
    /// was read. Other topic pages omit it until they use that read contract.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub store_version: Option<u64>,
    /// Only conversation message reads carry this durable origin. Keeping it
    /// here avoids repeating it in the 32 KiB session list. Older peers omit it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handed_over_from: Option<Box<SessionOrigin>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Cursor {
    StartAndId {
        start_ms: i64,
        id: String,
    },
    DescendingDateAndId {
        date_ms: i64,
        id: String,
    },
    Segment {
        index: u64,
        revision: u64,
        state_sha256: String,
    },
    Id {
        id: String,
    },
    Index {
        index: u64,
    },
    Name {
        name: String,
    },
    ByteRange {
        offset: u64,
    },
}

/// 4 KiB covers the JSON envelope, IDs and revision (fixed fields are under
/// 512 bytes with a UUID epoch and 128-byte correlation ID), with room for
/// encoding growth. It protects frame allocation, not stored data or the
/// legacy State frame. Every topic and read adds this same wire overhead.
pub const TOPIC_ENVELOPE_BYTES: usize = 4 * 1024;

/// A subscription carries one live topic, at most 96 KiB. Its separate
/// 4 KiB envelope protects framing, not durable collection length or reads.
pub const fn transport_max_bytes() -> usize {
    let mut max = 0;
    let mut index = 0;
    while index < TopicId::ALL.len() {
        let bytes = TopicId::ALL[index].max_bytes();
        if bytes > max {
            max = bytes;
        }
        index += 1;
    }
    max + TOPIC_ENVELOPE_BYTES
}

/// A keyed read can carry a 640 KiB page of a conversation containing one fully
/// escaped 96 KiB answer. This frame bound is separate from live topics and
/// still below the legacy 1 MiB IPC frame; it is not a retention limit.
pub const fn read_transport_max_bytes() -> usize {
    crate::reads::max_page_bytes() + TOPIC_ENVELOPE_BYTES
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn topic_and_read_transport_fit_the_ipc_frame() {
        assert!(transport_max_bytes() < crate::MAX_FRAME_BYTES);
        assert!(read_transport_max_bytes() < crate::MAX_FRAME_BYTES);
    }

    #[test]
    fn unknown_is_serialized_as_a_cause_not_a_negative_fact() -> Result<(), serde_json::Error> {
        let value = TopicData::not_observed(TopicId::Agenda);
        let json = serde_json::to_value(value)?;
        assert_eq!(json["knowledge"]["state"], "unknown");
        assert_eq!(json["knowledge"]["why"], "not_observed_yet");
        assert!(json["knowledge"].get("value").is_none());
        Ok(())
    }
}
