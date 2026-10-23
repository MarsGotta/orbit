//! The calendar: the agenda's reads and writes, and the Mac's time zone.

macro_rules! calendar_notices {
    ([$next:ident $(, $rest:ident)*] ; $($entries:tt)*) => {
        $next! { [$($rest),*] ; $($entries)*
            /// A read of the agenda could not be started.
            AgendaReadFailed = 146 => Silent,
            /// The system could not give the Mac's time zone (`cause`, its kind).
            TimezoneFailed = 147 => Silent,
            /// A calendar write was refused or its outcome is unknown (`code`,
            /// `stage`): the daemon only records it in the agenda topic.
            CalendarWriteFailed = 148 => Silent,
        }
    };
}
pub(super) use calendar_notices;
