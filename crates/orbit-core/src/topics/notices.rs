//! The current notice and the recent ones, typed. The interface
//! reads every notice it has not seen yet: a turn can say a notice the
//! person must see (a missing permission) and then one that is only
//! recorded (the empty recording that followed), and the second must not
//! hide the first.
use serde::{Deserialize, Serialize};

/// The topic's ceiling, in bytes of one frame. Each ceiling protects one
/// subscriber's frame allocation and actor serialization time, using the
/// measured payloads; none limits stored data or grants permission.
///
/// One typed notice: a code, an op of at most 64 bytes and data
/// from a closed set of keys whose values are numbers, booleans or
/// tokens of at most 64 bytes; the frame carries it as the notice of the
/// moment and in the log. With all 51 keys filled at their longest it
/// measured 8,348 bytes (`notices_frame_fits_with_every_key_at_its_longest`),
/// about 160 bytes per key added. 12 KiB leaves room for some twenty more
/// keys; it bounds one frame, not how many notices are said.
pub const MAX_BYTES: usize = 12 * 1024;
/// Published as soon as it changes.
pub const MIN_INTERVAL_MS: u64 = 0;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Value {
    #[serde(default, deserialize_with = "crate::notice::lenient_notice")]
    pub notice: Option<crate::notice::Notice>,
    /// The recent notices, oldest first (`State::notice_log`).
    #[serde(default, deserialize_with = "crate::notice::lenient_notices")]
    pub notices: Vec<crate::notice::Notice>,
}
