//! Topic and keyed-read cache.
use orbit_core::{
    reads::ReadRequest,
    topics::{Knowledge, Page, TopicData, TopicFrame, TopicId},
};
use std::{
    collections::{BTreeMap, VecDeque},
    time::{SystemTime, UNIX_EPOCH},
};

/// Sixteen entries cover the twelve current desktop views plus four concurrent
/// reads. This bounds retained page memory; it is no limit on stored records or
/// on how many pages may be read. Each entry also obeys its registered page cap.
const MAX_CACHED_READS: usize = 16;
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TopicConnection {
    Connecting,
    Ready,
    Ended { cause: String },
}

#[derive(Debug, Clone)]
pub struct CachedTopic {
    pub knowledge: TopicData,
    pub revision: u64,
    pub freshness: TopicFreshness,
    pub received_at_ms: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TopicFreshness {
    Current,
    Stale { since_ms: Option<i64> },
}

#[derive(Debug, Clone)]
struct CachedRead {
    topic: TopicId,
    revision: u64,
    answer: Knowledge<Page<serde_json::Value>>,
}

#[derive(Clone)]
pub struct TopicCache {
    epoch: Option<String>,
    topics: BTreeMap<TopicId, CachedTopic>,
    reads: BTreeMap<String, CachedRead>,
    read_order: VecDeque<String>,
    connection: TopicConnection,
}

impl Default for TopicCache {
    fn default() -> Self {
        Self {
            epoch: None,
            topics: BTreeMap::new(),
            reads: BTreeMap::new(),
            read_order: VecDeque::new(),
            connection: TopicConnection::Connecting,
        }
    }
}

impl TopicCache {
    pub fn connection(&self) -> &TopicConnection {
        &self.connection
    }

    /// Test caches start `Connecting`. A comparison of one topic holds the
    /// subscription ready on both sides so that flip is not counted as the
    /// topic's frames. Production reaches `Ready` through `observe_at`.
    #[cfg(test)]
    pub(crate) fn ready_without_topics(&mut self) {
        self.connection = TopicConnection::Ready;
    }
    pub fn epoch(&self) -> Option<&str> {
        self.epoch.as_deref()
    }
    pub fn topic(&self, id: TopicId) -> Option<&CachedTopic> {
        self.topics.get(&id)
    }

    pub fn observe(&mut self, frame: TopicFrame) {
        let received_at_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .ok()
            .and_then(|time| i64::try_from(time.as_millis()).ok());
        self.observe_at(frame, received_at_ms);
    }

    pub fn observe_at(&mut self, frame: TopicFrame, received_at_ms: Option<i64>) {
        if self.epoch.as_deref() != Some(frame.epoch.as_str()) {
            self.epoch = Some(frame.epoch.clone());
            self.topics.clear();
            self.reads.clear();
            self.read_order.clear();
        }
        let id = frame.id();
        if self
            .topics
            .get(&id)
            .is_none_or(|old| frame.revision > old.revision)
        {
            self.topics.insert(
                id,
                CachedTopic {
                    knowledge: frame.data,
                    revision: frame.revision,
                    freshness: TopicFreshness::Current,
                    received_at_ms,
                },
            );
        } else if let Some(cell) = self.topics.get_mut(&id) {
            cell.freshness = TopicFreshness::Current;
            cell.received_at_ms = received_at_ms;
        }
        if frame.revision > 0 {
            self.connection = TopicConnection::Ready;
        }
    }

    pub fn ended(&mut self, cause: String) {
        self.mark_stale();
        self.connection = TopicConnection::Ended { cause };
    }

    fn mark_stale(&mut self) {
        let since_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .ok()
            .and_then(|time| i64::try_from(time.as_millis()).ok());
        self.mark_stale_at(since_ms);
    }

    fn mark_stale_at(&mut self, since_ms: Option<i64>) {
        for cell in self.topics.values_mut() {
            if matches!(cell.freshness, TopicFreshness::Current) {
                cell.freshness = TopicFreshness::Stale { since_ms };
            }
        }
        self.reads.clear();
        self.read_order.clear();
    }

    pub fn read(
        &self,
        request: &ReadRequest,
        topic: TopicId,
    ) -> Option<&Knowledge<Page<serde_json::Value>>> {
        let key = serde_json::to_string(request).ok()?;
        let cached = self.reads.get(&key)?;
        let current = self.topics.get(&topic)?;
        (cached.topic == topic
            && cached.revision == current.revision
            && matches!(current.freshness, TopicFreshness::Current))
        .then_some(&cached.answer)
    }

    pub fn remember_read(
        &mut self,
        request: &ReadRequest,
        topic: TopicId,
        answer: Knowledge<Page<serde_json::Value>>,
    ) -> bool {
        if matches!(answer, Knowledge::Unknown { .. }) {
            return false;
        }
        let Some(revision) = self.topics.get(&topic).and_then(|cell| {
            matches!(cell.freshness, TopicFreshness::Current).then_some(cell.revision)
        }) else {
            return false;
        };
        let Ok(key) = serde_json::to_string(request) else {
            return false;
        };
        let Ok(bytes) = serde_json::to_vec(&answer) else {
            return false;
        };
        if bytes.len() > request.id().max_bytes() {
            return false;
        }
        if !self.reads.contains_key(&key) {
            if self.reads.len() >= MAX_CACHED_READS
                && let Some(oldest) = self.read_order.pop_front()
            {
                self.reads.remove(&oldest);
            }
            self.read_order.push_back(key.clone());
        }
        self.reads.insert(
            key,
            CachedRead {
                topic,
                revision,
                answer,
            },
        );
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(epoch: &str, revision: u64) -> TopicFrame {
        TopicFrame {
            epoch: epoch.into(),
            revision,
            data: TopicData::not_observed(TopicId::Notes),
        }
    }

    #[test]
    fn epoch_change_discards_all_facts_and_reads() {
        let mut cache = TopicCache::default();
        cache.observe(frame("one", 3));
        let read = ReadRequest::NotesPage { cursor: None };
        assert!(cache.remember_read(
            &read,
            TopicId::Notes,
            Knowledge::Present {
                value: Page {
                    items: Vec::new(),
                    next: None,
                    total: 0,
                    revision: 3,
                    store_version: None,
                    handed_over_from: None,
                },
                as_of_ms: None,
            }
        ));
        assert!(cache.read(&read, TopicId::Notes).is_some());
        cache.observe(frame("two", 1));
        assert_eq!(cache.epoch(), Some("two"));
        assert_eq!(
            cache.topic(TopicId::Notes).map(|cell| cell.revision),
            Some(1)
        );
        assert!(cache.read(&read, TopicId::Notes).is_none());
    }

    #[test]
    fn a_stale_revision_never_replaces_a_newer_fact() {
        let mut cache = TopicCache::default();
        cache.observe(frame("one", 5));
        cache.observe(frame("one", 4));
        assert_eq!(
            cache.topic(TopicId::Notes).map(|cell| cell.revision),
            Some(5)
        );
    }

    #[test]
    fn a_pending_read_is_not_cached_as_an_empty_result() {
        let mut cache = TopicCache::default();
        cache.observe(frame("one", 1));
        let read = ReadRequest::NotesPage { cursor: None };
        assert!(!cache.remember_read(
            &read,
            TopicId::Notes,
            Knowledge::Unknown {
                why: orbit_core::topics::Pending::NotObservedYet,
            }
        ));
        assert!(cache.read(&read, TopicId::Notes).is_none());
    }

    #[test]
    fn ended_daemon_leaves_topics_stale_and_cannot_validate_reads() {
        let mut cache = TopicCache::default();
        cache.observe(TopicFrame {
            epoch: "one".into(),
            revision: 3,
            data: TopicData::absent(
                TopicId::Notes,
                orbit_core::refusal::Refusal::HelperNotRunning,
            ),
        });
        let read = ReadRequest::NotesPage { cursor: None };
        assert!(cache.remember_read(
            &read,
            TopicId::Notes,
            Knowledge::Present {
                value: Page {
                    items: Vec::new(),
                    next: None,
                    total: 0,
                    revision: 3,
                    store_version: None,
                    handed_over_from: None,
                },
                as_of_ms: None,
            },
        ));
        assert!(cache.read(&read, TopicId::Notes).is_some());
        cache.ended("disco: 3 escrituras pendientes".into());
        assert!(matches!(cache.connection(), TopicConnection::Ended { .. }));
        assert!(cache.topic(TopicId::Notes).is_some());
        assert!(matches!(
            cache.topic(TopicId::Notes).map(|cell| &cell.freshness),
            Some(TopicFreshness::Stale { .. })
        ));
        assert!(cache.read(&read, TopicId::Notes).is_none());
        cache.observe(TopicFrame {
            epoch: "one".into(),
            revision: 3,
            data: TopicData::absent(
                TopicId::Notes,
                orbit_core::refusal::Refusal::HelperNotRunning,
            ),
        });
        assert!(matches!(
            cache.topic(TopicId::Notes).map(|cell| &cell.freshness),
            Some(TopicFreshness::Current)
        ));
        assert!(cache.read(&read, TopicId::Notes).is_none());
    }

    #[test]
    fn stale_without_clock_does_not_invent_a_date() {
        let mut cache = TopicCache::default();
        cache.observe(frame("one", 3));
        cache.mark_stale_at(None);
        assert!(matches!(
            cache.topic(TopicId::Notes).map(|cell| &cell.freshness),
            Some(TopicFreshness::Stale { since_ms: None })
        ));
    }

    #[test]
    fn read_cache_evicts_the_first_inserted_entry() {
        let mut cache = TopicCache::default();
        cache.observe(frame("one", 1));
        let answer = || Knowledge::Present {
            value: Page {
                items: Vec::new(),
                next: None,
                total: 0,
                revision: 1,
                store_version: None,
                handed_over_from: None,
            },
            as_of_ms: None,
        };
        let request = |name: String| ReadRequest::NoteDetail {
            id: name,
            cursor: None,
        };
        let first = request("z-first".into());
        assert!(cache.remember_read(&first, TopicId::Notes, answer()));
        for index in 0..15 {
            assert!(cache.remember_read(
                &request(format!("a-{index:02}")),
                TopicId::Notes,
                answer()
            ));
        }
        let newest = request("newest".into());
        assert!(cache.remember_read(&newest, TopicId::Notes, answer()));
        assert!(cache.read(&first, TopicId::Notes).is_none());
        assert!(cache.read(&newest, TopicId::Notes).is_some());
    }
}
