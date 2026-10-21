//! The current notice and the recent ones, typed. The interface
//! reads every notice it has not seen yet: a turn can say a notice the
//! person must see (a missing permission) and then one that is only
//! recorded (the empty recording that followed), and the second must not
//! hide the first.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Value {
    #[serde(default, deserialize_with = "crate::notice::lenient_notice")]
    pub notice: Option<crate::notice::Notice>,
    /// The recent notices, oldest first (`State::notice_log`).
    #[serde(default, deserialize_with = "crate::notice::lenient_notices")]
    pub notices: Vec<crate::notice::Notice>,
}
