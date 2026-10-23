//! What changed in the meetings and notes. The meetings live on the
//! hub; this head only says when the app should read them again: the list
//! when `revision` moves, an open note when its own counter moves.
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// The topic's ceiling, in bytes of one frame. Each ceiling protects one
/// subscriber's frame allocation and actor serialization time, using the
/// measured payloads; none limits stored data or grants permission.
///
/// Three counters and at most 32 named recordings, about 2.3 KiB
/// with every number at its widest (`the_head_with_every_name_fits_its_
/// frame`). 8 KiB catches growth; the meetings themselves are read from
/// the hub.
pub const MAX_BYTES: usize = 8 * 1024;
/// 1 s bounds list rereads, not how often the hub announces.
pub const MIN_INTERVAL_MS: u64 = 1_000;

/// Recordings whose change this head names one by one. The hub says which
/// recording changed (`recording.changed`); a meeting being recorded changes
/// every 20–30 s, and a day holds a handful of meetings, so 32 cover far more
/// than are open on screen. It protects the topic frame (8 KiB) from a hub
/// that names many recordings; past it the oldest leave and their changes
/// fold into `floor`, which is never read as «unchanged».
pub const MAX_NAMED_RECORDINGS: usize = 32;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Value {
    /// Moves with every change: one the hub announced, a recording that
    /// started, ended or finished uploading on this Mac, or private notes the
    /// hub confirmed. The list is read again when it moves.
    pub revision: u64,
    /// The revision of the last change of each recording named here.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub recordings: BTreeMap<String, u64>,
    /// The newest change of a recording no longer named: any recording not
    /// in `recordings` may have changed as late as this.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub floor: u64,
    /// Moves when a template or assignment order is answered, so
    /// the app reads the library again.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub library_revision: u64,
    /// The recording being recorded on this Mac now: the list is read again
    /// when one starts or ends.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recording: Option<String>,
}

fn is_zero(value: &u64) -> bool {
    *value == 0
}

impl Value {
    /// A change that only the list shows.
    pub fn touch(&mut self) {
        self.revision = self.revision.saturating_add(1);
    }

    /// A change of `recording`: the list and that note.
    pub fn touch_recording(&mut self, recording: &str) {
        self.touch();
        self.recordings.insert(recording.to_owned(), self.revision);
        while self.recordings.len() > MAX_NAMED_RECORDINGS {
            let Some((oldest, at)) = self
                .recordings
                .iter()
                .min_by_key(|(_, at)| **at)
                .map(|(id, at)| (id.clone(), *at))
            else {
                break;
            };
            self.recordings.remove(&oldest);
            self.floor = self.floor.max(at);
        }
    }

    /// The latest change `recording` may have had; `0` when none.
    pub fn changed_at(&self, recording: &str) -> u64 {
        self.recordings
            .get(recording)
            .copied()
            .unwrap_or(self.floor)
    }

    pub fn touch_library(&mut self) {
        self.library_revision = self.library_revision.saturating_add(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_change_names_its_recording_and_moves_the_list() {
        let mut value = Value::default();
        value.touch_recording("a");
        value.touch();
        value.touch_recording("b");
        assert_eq!(value.revision, 3);
        assert_eq!(value.changed_at("a"), 1);
        assert_eq!(value.changed_at("b"), 3);
        assert_eq!(value.changed_at("never"), 0);
    }

    #[test]
    fn a_recording_no_longer_named_is_unknown_never_unchanged() {
        let mut value = Value::default();
        for index in 0..=MAX_NAMED_RECORDINGS {
            value.touch_recording(&format!("r{index}"));
        }
        assert_eq!(value.recordings.len(), MAX_NAMED_RECORDINGS);
        assert!(!value.recordings.contains_key("r0"));
        assert_eq!(value.floor, 1);
        assert_eq!(value.changed_at("r0"), 1);
        assert_eq!(value.changed_at("never-named"), 1);
    }

    #[test]
    fn the_head_with_every_name_fits_its_frame() -> Result<(), serde_json::Error> {
        let mut value = Value::default();
        for index in 0..MAX_NAMED_RECORDINGS * 4 {
            value.touch_recording(&format!("00000000-0000-4000-8000-{index:012x}"));
        }
        value.revision = u64::MAX;
        value.floor = u64::MAX;
        value.library_revision = u64::MAX;
        value.recording = Some("00000000-0000-4000-8000-000000000000".into());
        for at in value.recordings.values_mut() {
            *at = u64::MAX;
        }
        let bytes = serde_json::to_vec(&super::super::TopicData::Notes(
            super::super::Knowledge::Present {
                value,
                as_of_ms: Some(i64::MAX),
            },
        ))?
        .len();
        assert!(
            bytes <= super::super::TopicId::Notes.max_bytes(),
            "{bytes} bytes"
        );
        Ok(())
    }
}
