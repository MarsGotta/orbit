//! The calendar: the values `calendar` owns, mirrored in Swift.
//!
//! The calendar values below are owned by `calendar`, mirrored in Swift and
//! compared by the constants guard (`scripts/tests/test_constants_copies.py`).
//! Registry entries cite them directly; they are not second definitions of
//! those ceilings.
use super::Limit;

pub(super) fn limits() -> Vec<Limit> {
    [read_limits(), write_limits()]
        .into_iter()
        .flatten()
        .collect()
}

fn read_limits() -> Vec<Limit> {
    vec![
        Limit {
            name: "MAX_NATIVE_PAGE_BYTES",
            value: crate::calendar::MAX_NATIVE_PAGE_BYTES as u64,
            unit: "bytes",
            resource: "calendar helper page",
        },
        Limit {
            name: "NATIVE_PAGE_EVENT_BYTES",
            value: crate::calendar::NATIVE_PAGE_EVENT_BYTES as u64,
            unit: "bytes",
            resource: "calendar page event budget",
        },
        Limit {
            name: "EVENTKIT_MAX_QUERY_MS",
            value: crate::calendar::EVENTKIT_MAX_QUERY_MS as u64,
            unit: "milliseconds",
            resource: "EventKit query window",
        },
        Limit {
            name: "MAX_TITLE_BYTES",
            value: crate::calendar::MAX_TITLE_BYTES as u64,
            unit: "bytes",
            resource: "calendar title",
        },
        Limit {
            name: "MAX_URL_BYTES",
            value: crate::calendar::MAX_URL_BYTES as u64,
            unit: "bytes",
            resource: "calendar URL",
        },
        Limit {
            name: "MAX_LABEL_BYTES",
            value: crate::calendar::MAX_LABEL_BYTES as u64,
            unit: "bytes",
            resource: "calendar label",
        },
        Limit {
            name: "MAX_TIMEZONE_BYTES",
            value: crate::calendar::MAX_TIMEZONE_BYTES as u64,
            unit: "bytes",
            resource: "calendar timezone",
        },
        Limit {
            name: "READ_LIFETIME_NS",
            value: crate::calendar::READ_LIFETIME_NS,
            unit: "nanoseconds",
            resource: "calendar read lifetime",
        },
        Limit {
            name: "PERMISSION_LIFETIME_NS",
            value: crate::calendar::PERMISSION_LIFETIME_NS,
            unit: "nanoseconds",
            resource: "calendar permission response lifetime",
        },
        Limit {
            name: "MAX_GRID_DAYS",
            value: crate::calendar::MAX_GRID_DAYS as u64,
            unit: "days",
            resource: "widest N-day calendar grid",
        },
    ]
}

fn write_limits() -> Vec<Limit> {
    vec![
        // The bounds of the calendar writes and of the notes a read carries.
        Limit {
            name: "MAX_NOTES_BYTES",
            value: crate::calendar::MAX_NOTES_BYTES as u64,
            unit: "bytes",
            resource: "calendar event notes",
        },
        Limit {
            name: "MAX_EVENT_ALARMS",
            value: crate::calendar::MAX_EVENT_ALARMS as u64,
            unit: "alarms",
            resource: "alarms of one written calendar event",
        },
        Limit {
            name: "MAX_ALARM_MINUTES",
            value: crate::calendar::MAX_ALARM_MINUTES,
            unit: "minutes",
            resource: "calendar alarm offset before its event",
        },
        Limit {
            name: "WRITE_LIFETIME_NS",
            value: crate::calendar::WRITE_LIFETIME_NS,
            unit: "nanoseconds",
            resource: "calendar write lifetime before its outcome is unknown",
        },
    ]
}
