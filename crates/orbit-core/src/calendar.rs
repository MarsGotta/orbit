//! EventKit wire contract: the local read the daemon assembles, and the
//! writes Mars makes from Orbit (P9-21).
//!
//! REF-10: the macOS calendar permission is the only decision about Mars's
//! agenda. Nothing is selected, confirmed, copied or kept: the helper reads
//! every calendar the permission covers, one window of the local store at a
//! time and in pages, and the daemon joins the pages. These values neither read
//! calendars nor authorize audio, remote sharing, persistence or input.
//!
//! P9-21: the same permission (full access) is the whole decision for writes
//! too. A write Mars makes in Orbit is validated here, purely, before it
//! reaches the helper; there is no confirmation and no second consent.
//!
//! The caller must correlate each native response with its outstanding
//! command. UUIDs and validation of JSON alone do not authenticate a response.
//! Native mach timestamps and the caller's clock have unrelated origins: only
//! durations within each clock domain are compared, never their absolute values.
//!
//! Every ceiling left here is internal: it sizes a message between processes,
//! and Orbit asks for the next page instead of refusing, trimming or warning.

mod conference;
mod events;
mod mailbox;
mod models;
mod pages;
mod validation;
mod wire;
mod writes;

pub use conference::{ConferencePlatform, conference_platform, conference_platform_in};
pub use models::*;
pub use pages::*;
pub use wire::{decode_inventory_page, decode_read_page, decode_status};
pub use writes::*;

/// The widest grid of N days in the app's calendar, the `0` key (P9-20).
/// Protects nothing beyond the layout: past fourteen columns a column is
/// narrower than an event title. It is not a read limit — a month and the
/// agenda are wider and read whole. The interface copies it once
/// (`apps/desktop/src/features/calendar/range.ts`), held to this value by
/// `scripts/tests/test_constants_copies.py`.
pub const MAX_GRID_DAYS: u8 = 14;

/// The largest helper reply the daemon admits for one calendar page or status.
///
/// Protects: the daemon's link to the helper. Its reader drops the whole helper
/// connection — audio included — on any line above `crate::limits::MAX_HELPER_EVENT_LINE_BYTES`
/// (1 MiB), so no calendar reply may come close to it. Why 768 KiB: it leaves a
/// quarter of that line for the response envelope and for JSON escaping of the
/// payload. What it is not: a limit on events, calendars or invitees. The helper
/// fills a page with whole events up to `NATIVE_PAGE_EVENT_BYTES` and says where
/// the next one starts; only a single event whose own fields exceed this could
/// reach it (roughly four thousand invitees), and that one is refused by name.
pub const MAX_NATIVE_PAGE_BYTES: usize = 768 * 1024;

/// How many bytes of events the helper puts in a page before starting the next.
///
/// Soft: a page always carries at least one whole event, so a large event is
/// never split or dropped here. Why 256 KiB: the size of a helper calendar
/// reply before REF-10, known to cross the link without stalling audio; three
/// such pages fit under `MAX_NATIVE_PAGE_BYTES`. Mirrored by
/// `CalendarLimits.pageEventBytes` in Swift; their copies are compared through
/// `contracts/constants.json`.
pub const NATIVE_PAGE_EVENT_BYTES: usize = 256 * 1024;

/// The widest window the helper accepts in one query.
///
/// Protects: completeness. EventKit's `predicateForEvents` silently shortens
/// any range longer than four years to its first four years, so a wider query
/// would lose events without saying so. Why 1,460 days: four years of 365 days
/// is never longer than four calendar years, leap day or not. It is EventKit's
/// limit, not Orbit's: the daemon reads in `READ_WINDOW_MS` windows, far below
/// it. Mirrored by `CalendarLimits.maximumQueryMilliseconds` in the helper.
pub const EVENTKIT_MAX_QUERY_MS: i64 = 1_460 * 86_400_000;

/// How much of a range the daemon asks the helper for at a time.
///
/// The helper enumerates the whole window on every page request, so a window
/// sized to usually fit one page keeps that repetition rare. Why 31 days: a
/// month of a busy agenda (a few hundred events) is one page, and a year is
/// twelve reads. Nobody sees it: any range is read to its end in these windows,
/// and a window with more events than a page is paged, never cut.
pub const READ_WINDOW_MS: i64 = 31 * 86_400_000;

/// The longest event title, calendar name, account name or invitee name, in
/// bytes, one event may carry.
///
/// Protects: that no single field is larger than the page it has to travel in.
/// Why 96 KiB: it is `device_bridge::MAX_CALENDAR_PAGE_BYTES`, the largest
/// answer the link to the hub carries; a longer field could never reach Deimos
/// whatever else the event held. Decision 4 of REF-10 (the reviewer's): a field
/// is not a reason to leave an event out while the event fits a page on its
/// own — the 2,048-byte title bound this replaced stopped whole reads for a
/// title a person could paste. What it is not: a limit on events, calendars or
/// invitees, nor a ceiling that stops a read. An event with a field past it,
/// or one that does not fit a page alone, is left out *by name* — its dates,
/// its identifier, the field and its bytes against the bytes that fit
/// (`CalendarOversizedEvent`) — and the rest of the read arrives whole; it is
/// never trimmed. Mirrored by `CalendarLimits.maximumTitleBytes` in Swift;
/// their copies are compared through `contracts/constants.json`.
pub const MAX_TITLE_BYTES: usize = 96 * 1024;
/// The longest event URL or invitee address, in bytes. Same purpose, size, rule
/// and guard as `MAX_TITLE_BYTES`. Mirrored by `CalendarLimits.maximumURLBytes`.
pub const MAX_URL_BYTES: usize = 96 * 1024;
/// The longest identifier (calendar, account, event), in bytes. Not a size
/// ceiling but a shape check: an EventKit identifier is a few dozen bytes, and
/// one past a kilobyte is not an identifier. An event whose own identifier
/// fails it is left out by name like any other; a calendar whose identifier
/// fails it is not listed. Mirrored by `CalendarLimits.maximumLabelBytes`.
pub const MAX_LABEL_BYTES: usize = 1_024;
/// The longest time zone name, in bytes. A shape check like `MAX_LABEL_BYTES`:
/// IANA names are under 40 bytes. Mirrored by
/// `CalendarLimits.maximumTimezoneBytes`.
pub const MAX_TIMEZONE_BYTES: usize = 128;

/// The longest event notes (the description) that travel with an event, in
/// bytes (P9-21).
///
/// Protects: the pages around the event. Notes are free text pasted by
/// anyone who sends an invitation; one event's description must not crowd out
/// the events that share its page — the helper's (`NATIVE_PAGE_EVENT_BYTES`),
/// the view's and above all the bridge page Deimos reads, 96 KiB
/// (`device_bridge::MAX_CALENDAR_PAGE_BYTES`). Why 32 KiB: a video-call
/// invitation with its dial-in boilerplate and an agenda is a few KiB, so
/// this is several times the longest ordinary description, and a third of a
/// bridge page, so an event with long notes still shares its page. What it is
/// not: a reason to leave the event out. On a read, an event whose notes are
/// longer arrives whole without them and says so with `notes_left_out`
/// (their bytes and this limit); on a write, notes longer than this are
/// refused by name with the same two numbers. Mirrored by
/// `CalendarLimits.maximumNotesBytes`, kept equal by hand (SIM-00 removed the
/// guard that compared them).
pub const MAX_NOTES_BYTES: usize = 32 * 1024;

/// How many alarms one event written from Orbit may carry (P9-21).
///
/// Protects: the size of one write command, which is checked whole before it
/// reaches EventKit. Why 16: Calendar offers two alerts per event and a person
/// rarely adds more; sixteen is a shape check, not a product limit, and no
/// read is bounded by it (alarms are not read). Past it the write is refused
/// with the count and this limit. Mirrored by `CalendarLimits.maximumAlarms`.
pub const MAX_EVENT_ALARMS: usize = 16;

/// The furthest an alarm may sound before its event, in minutes (P9-21).
///
/// A shape check, like `MAX_LABEL_BYTES`: four weeks (40,320 minutes) is past
/// the longest custom alert Calendar offers, so a larger number is a malformed
/// command rather than a reminder. Refused with the value and this limit.
/// Mirrored by `CalendarLimits.maximumAlarmMinutes`.
pub const MAX_ALARM_MINUTES: u64 = 40_320;

/// How long the helper may hold one EventKit write (save or remove) before it
/// answers that the outcome is unknown (P9-21).
///
/// Protects: the single EventKit slot and the person waiting on the change.
/// Why five seconds: a commit to the local store takes milliseconds — the same
/// order as a read (`READ_LIFETIME_NS`) — and syncing with iCloud, Google or
/// Exchange happens after it, never inside it. It is its own constant because
/// what happens when it passes differs from a read: a late read is simply
/// discarded, but a late write may already be committed. So the helper
/// answers `calendar_write_outcome_unknown`, the daemon reads the agenda
/// again, and nothing is sent a second time. What it is not: a retry timer.
/// Mirrored by `CalendarLimits.writeLifetimeNanoseconds`.
pub const WRITE_LIFETIME_NS: u64 = 5_000_000_000;

pub const READ_LIFETIME_NS: u64 = 5_000_000_000;
/// How long a permission request may stay outstanding.
///
/// A read waits for EventKit and five seconds is generous. A permission request
/// waits for a person to find a system dialog, read it and decide, and five
/// seconds silently discarded their answer: the prompt was still on screen when
/// the permit expired, so granting it changed nothing and the state stayed
/// undetermined with no error to show for it.
pub const PERMISSION_LIFETIME_NS: u64 = 120_000_000_000;
pub const CLOCK_DOMAIN: &str = "mach_absolute_time";
pub const MIN_UNIX_MS: i64 = -62_135_596_800_000;
pub const MAX_UNIX_MS: i64 = 253_402_300_799_999;

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum CalendarError {
    #[error("invalid calendar wire data")]
    InvalidWire,
    #[error("calendar message exceeds the native page limit")]
    LimitExceeded,
    #[error("invalid calendar value")]
    InvalidValue,
    #[error("calendar clock domain or interval is invalid")]
    InvalidClock,
    #[error("calendar elapsed time is unavailable or regressed")]
    ClockUnavailable,
    #[error("calendar response expired")]
    Expired,
    #[error("calendar identity or generation changed")]
    ScopeChanged,
    #[error("calendar read permission is unavailable")]
    PermissionRequired,
    #[error("privacy stop forbids calendar delivery")]
    PrivacyStopped,
    #[error("calendar optional data was not authorized")]
    OptionalDataNotAuthorized,
}

/// Facts supplied by the response-owning actor, never deserialized from native
/// data or user input. `None` elapsed means an unavailable/regressed local clock.
/// Measure from BEFORE sending the correlated request, not from receipt. This
/// conservatively bounds response age without treating Rust Instant as mach time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CalendarReadFacts<'a> {
    pub native_instance_id: &'a str,
    pub privacy_generation: u64,
    pub authorization: CalendarAuthorization,
    pub privacy_stopped: bool,
    pub elapsed_since_request_ns: Option<u64>,
}

impl CalendarReadFacts<'_> {
    pub(super) fn authorize(&self, instance: &str, generation: u64) -> Result<(), CalendarError> {
        if self.privacy_stopped {
            return Err(CalendarError::PrivacyStopped);
        }
        if self.authorization != CalendarAuthorization::FullAccess {
            return Err(CalendarError::PermissionRequired);
        }
        if !crate::wire::canonical_uuid(self.native_instance_id)
            || self.native_instance_id != instance
            || self.privacy_generation != generation
        {
            return Err(CalendarError::ScopeChanged);
        }
        elapsed_before(self.elapsed_since_request_ns, READ_LIFETIME_NS)
    }
}

pub(super) fn elapsed_before(
    elapsed: Option<u64>,
    exclusive_limit: u64,
) -> Result<(), CalendarError> {
    let elapsed = elapsed.ok_or(CalendarError::ClockUnavailable)?;
    if elapsed >= exclusive_limit {
        return Err(CalendarError::Expired);
    }
    Ok(())
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod contract_tests;
