//! The calendar: what a read of the agenda cannot carry, and a Mac without
//! its calendar backend.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CalendarReadLimit {
    Page {
        max_bytes: u64,
    },
    Fields {
        max_title_bytes: u64,
        max_label_bytes: u64,
        max_timezone_bytes: u64,
    },
}

macro_rules! calendar_refusals {
    ([$next:ident $(, $rest:ident)*] ; $($entries:tt)*) => {
        $next! { [$($rest),*] ; $($entries)*
            CalendarReadLimit = 1 => "calendar_read_limit" { cause: CalendarReadLimit },
                retry Retry::AfterFix;
            CalendarBackendUnavailable = 16 => "calendar_backend_unavailable",
                retry Retry::AfterFix;
        }
    };
}
pub(super) use calendar_refusals;
