//! Small agenda head. Event collections travel in paged reads.
use crate::{
    calendar::{CalendarAuthorization, CalendarWindow},
    refusal::{Permission, Refusal},
    topics::{Knowledge, Pending},
};
use serde::{Deserialize, Serialize};

/// The topic's ceiling, in bytes of one frame. Each ceiling protects one
/// subscriber's frame allocation and actor serialization time, using the
/// measured payloads; none limits stored data or grants permission.
///
/// Two things share it. The read head (window, read id, zone, counts) is
/// below 1 KiB and needs no more: events travel in paged reads, so this is
/// not a calendar limit. The head also carries the outcomes of the writes
/// the user makes from Orbit, each a few hundred bytes with its notice; the daemon puts in
/// the newest that fit in what is left and leaves the older ones out. One
/// left out is not lost: the view counts a write the head never names as
/// unknown after `calendar::WRITE_LIFETIME_NS`, says so and reads again,
/// and it is never sent twice. Not a limit on how many writes the user makes.
pub const MAX_BYTES: usize = 4 * 1024;
/// Published as soon as it changes.
pub const MIN_INTERVAL_MS: u64 = 0;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Value {
    pub invalidation: u64,
    pub calendars: Option<u32>,
    pub read_window: Option<CalendarWindow>,
    pub read_id: Option<String>,
    pub timezone: Option<String>,
    /// How the writes the user made from Orbit ended, newest last. Only
    /// the newest that fit the head's budget travel (the daemon keeps more);
    /// a write missing here says nothing about the store.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub writes: Vec<WriteOutcome>,
}

/// One write's outcome as the agenda head carries it: small on
/// purpose. The event itself travels in the read the daemon starts after
/// every write; this only says where the write is and, once saved, which
/// occurrence it became, so the app can undo it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ts",
    ts(export_to = "../../apps/desktop/src/bridge/generated/")
)]
pub struct WriteOutcome {
    pub request_id: String,
    pub operation: crate::calendar_control::CalendarWriteOperation,
    pub stage: crate::calendar_control::CalendarWriteStage,
    /// The refusal's code (`calendar_not_editable`,
    /// `calendar_write_outcome_unknown`…) when it was refused or its outcome
    /// is unknown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub code: Option<String>,
    /// What the user reads about a refusal or an unknown outcome, with its
    /// numbers (`CalendarWriteRefusal::notice`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub notice: Option<String>,
    /// The occurrence a save left, by the identity a later change or delete
    /// names it with.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub saved: Option<WriteTarget>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ts",
    ts(export_to = "../../apps/desktop/src/bridge/generated/")
)]
pub struct WriteTarget {
    pub calendar_item_id: String,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub occurrence_start_ms: i64,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub start_ms: i64,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub end_ms: i64,
}

/// The local calendar decision. The caller supplies macOS's observation and
/// the user's durable privacy stop; no app setting or selected-calendar list votes.
pub fn access(authorization: CalendarAuthorization, privacy_stopped: bool) -> Knowledge<()> {
    if privacy_stopped {
        return Knowledge::Absent {
            refusal: Refusal::PrivacyStopped { since_ms: None },
        };
    }
    match authorization {
        CalendarAuthorization::FullAccess => Knowledge::Present {
            value: (),
            as_of_ms: None,
        },
        CalendarAuthorization::Restricted
        | CalendarAuthorization::Denied
        | CalendarAuthorization::WriteOnly => Knowledge::Absent {
            refusal: Refusal::PermissionDenied {
                permission: Permission::Calendar,
            },
        },
        CalendarAuthorization::Unavailable => Knowledge::Absent {
            refusal: Refusal::CalendarBackendUnavailable,
        },
        CalendarAuthorization::NotDetermined | CalendarAuthorization::Unknown => {
            Knowledge::Unknown {
                why: Pending::NotObservedYet,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn access_keeps_unknown_absent_and_present_distinct() {
        assert!(matches!(
            access(CalendarAuthorization::Unknown, false),
            Knowledge::Unknown {
                why: Pending::NotObservedYet
            }
        ));
        assert!(matches!(
            access(CalendarAuthorization::NotDetermined, false),
            Knowledge::Unknown {
                why: Pending::NotObservedYet
            }
        ));
        let Knowledge::Absent { refusal } = access(CalendarAuthorization::Unavailable, false)
        else {
            panic!("EventKit unavailable must have a refusal");
        };
        assert_eq!(refusal, Refusal::CalendarBackendUnavailable);
        assert!(!refusal.to_string().contains("Orbit.app no está en marcha"));
        assert!(matches!(
            access(CalendarAuthorization::Denied, false),
            Knowledge::Absent {
                refusal: Refusal::PermissionDenied {
                    permission: Permission::Calendar
                }
            }
        ));
        assert!(matches!(
            access(CalendarAuthorization::FullAccess, true),
            Knowledge::Absent {
                refusal: Refusal::PrivacyStopped { .. }
            }
        ));
        assert!(matches!(
            access(CalendarAuthorization::FullAccess, false),
            Knowledge::Present { .. }
        ));
    }
}
