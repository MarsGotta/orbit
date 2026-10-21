//! The zone the Mac is set to, read through the operating system.
//!
//! This is the one place app-core looks at the machine's zone. It runs in the
//! app's process on the user's Mac, so the zone it reads is theirs: `chrono::Local`
//! follows `/etc/localtime`, which macOS rewrites when they change the zone in
//! System Settings or it follows their location, and it notices within a second.
//! What it reads is turned into a [`LocalZone`] value and handed to the
//! projection beside `now_ms`; the projection never reads anything itself.

use crate::local_time::{DAY_MS, LocalZone, sample_zone};
use chrono::{DateTime, Local, Offset, TimeZone};
use std::sync::{Mutex, PoisonError};

/// How far back the zone's changes are known exactly. Orbit is younger than
/// ten years, so every recording, note and conversation it holds has its date
/// cut with the offset that was really in force.
///
/// Neither this nor [`KNOWN_AFTER_MS`] limits what can be shown: an instant
/// outside the span is still labelled, with the offset at the nearest end of
/// it, which can be an hour off across a daylight-saving change and no more.
const KNOWN_BEFORE_MS: i64 = 10 * 366 * DAY_MS;
/// How far ahead the zone's changes are known exactly: past anything the
/// projection labels, which is today's agenda. The calendar week is labelled
/// by the interface, in the zone of the Mac's own web view.
const KNOWN_AFTER_MS: i64 = 2 * 366 * DAY_MS;
const HOUR_MS: i64 = 60 * 60 * 1_000;

/// Keeps the last zone read, for as long as it can still be the same one.
///
/// Reading it means asking the system about every day of twelve years, which
/// is cheap once and wasteful on every snapshot. It is read again when the
/// hour changes, so the known span moves with the clock, and as soon as the
/// offset in force now differs from the kept zone's, which is what changing
/// the zone on the Mac looks like.
#[derive(Default)]
pub struct SystemZone {
    kept: Mutex<Option<KeptZone>>,
}

struct KeptZone {
    hour: i64,
    zone: LocalZone,
}

impl SystemZone {
    pub fn name(&self) -> Result<String, String> {
        iana_time_zone::get_timezone().map_err(timezone_failure_cause)
    }

    pub fn around(&self, now_ms: i64) -> LocalZone {
        let hour = now_ms.div_euclid(HOUR_MS);
        let current = system_offset_ms(now_ms);
        let mut kept = self.kept.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(zone) = kept
            .as_ref()
            .filter(|kept| kept.hour == hour && kept.zone.offset_ms_at(now_ms) == current)
            .map(|kept| kept.zone.clone())
        {
            return zone;
        }
        let zone = sample_zone(now_ms, KNOWN_BEFORE_MS, KNOWN_AFTER_MS, system_offset_ms);
        *kept = Some(KeptZone {
            hour,
            zone: zone.clone(),
        });
        zone
    }
}

fn timezone_failure_cause(error: iana_time_zone::GetTimezoneError) -> String {
    match error {
        iana_time_zone::GetTimezoneError::FailedParsingString => "timezone_parse_failed".into(),
        iana_time_zone::GetTimezoneError::OsError => "timezone_os_failed".into(),
        iana_time_zone::GetTimezoneError::IoError(error) => format!("timezone_io_failed:{error}"),
    }
}

/// The Mac's offset from UTC at `at_ms`, from the system's zone rules. An
/// instant chrono cannot represent (hundreds of thousands of years away) reads
/// as UTC; nothing the projection labels is there.
fn system_offset_ms(at_ms: i64) -> i64 {
    DateTime::from_timestamp_millis(at_ms).map_or(0, |instant| {
        i64::from(
            Local
                .offset_from_utc_datetime(&instant.naive_utc())
                .fix()
                .local_minus_utc(),
        ) * 1_000
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn operating_system_failure_has_a_stable_cause() {
        assert_eq!(
            timezone_failure_cause(iana_time_zone::GetTimezoneError::OsError),
            "timezone_os_failed"
        );
        assert_eq!(
            timezone_failure_cause(iana_time_zone::GetTimezoneError::FailedParsingString),
            "timezone_parse_failed"
        );
    }

    // The host's zone is whatever the machine running the suite has, so these
    // only check what holds in any zone. That the Mac's zone is read right is
    // evidence only the Mac can give.
    #[test]
    fn the_kept_zone_agrees_with_the_system_now() {
        let zone = SystemZone::default();
        let now = 1_792_890_000_000;
        let read = zone.around(now);
        assert_eq!(read.offset_ms_at(now), system_offset_ms(now));
        assert_eq!(zone.around(now + 1_000), read, "the same hour reuses it");
    }
}
