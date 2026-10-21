//! The agenda the app shows: the visible range, read from every calendar.
//!
//! REF-10: the macOS permission is the only decision. There is no selection to
//! restore, nothing to list first and no copy to share, renew or withdraw: the
//! daemon lists the calendars by itself when it learns the permission, and a
//! read of the range covers all of them. What is left here is which period is
//! on screen (P9-20: a day, N days, a week, a month or the agenda) and, from
//! REF-04, the present the projection is made for: the instant and Mars's
//! zone, as values.
use crate::{
    AppSnapshot, CalendarRangeSnapshot, CalendarViewKind, CalendarWeekStart, local_time::LocalZone,
    system_zone::SystemZone,
};
use std::{
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::sync::watch;

pub(crate) const AGENDA_REQUEST_FAILED_PREFIX: &str = "agenda_request_failed:";

/// Where the app reads the present: the instant, and the zone of the Mac it is
/// read on. Both reach the projection as values; a test pins them.
pub trait CalendarClock: Send + Sync {
    fn now_ms(&self) -> i64;
    /// Mars's time zone around `now_ms`: what "hoy" and «10:00» are read in.
    fn local_zone(&self, now_ms: i64) -> LocalZone;
    fn timezone_name(&self) -> Result<String, String>;

    /// An injected event source can advance the zone observer without virtual time.
    fn subscribe_ticks(&self) -> Option<watch::Receiver<()>> {
        None
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalClock {
    pub now_ms: i64,
    pub zone: LocalZone,
    pub timezone_name: Result<String, String>,
}

impl LocalClock {
    pub fn read(clock: &dyn CalendarClock) -> Self {
        let now_ms = clock.now_ms();
        Self {
            now_ms,
            zone: clock.local_zone(now_ms),
            timezone_name: clock.timezone_name(),
        }
    }

    pub(crate) fn from_snapshot(snapshot: &AppSnapshot) -> Self {
        let timezone_name = match &snapshot.system_timezone_error {
            Some(cause) => Err(cause.clone()),
            None => Ok(snapshot.system_timezone.clone()),
        };
        Self {
            now_ms: snapshot.briefing_clock_ms,
            zone: LocalZone::utc(),
            timezone_name,
        }
    }
}

#[derive(Default)]
pub struct SystemCalendarClock {
    zone: SystemZone,
}

impl CalendarClock for SystemCalendarClock {
    fn now_ms(&self) -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .ok()
            .and_then(|duration| i64::try_from(duration.as_millis()).ok())
            .unwrap_or(0)
    }

    fn local_zone(&self, now_ms: i64) -> LocalZone {
        self.zone.around(now_ms)
    }

    fn timezone_name(&self) -> Result<String, String> {
        self.zone.name()
    }
}

/// What the calendar was asked to show, before Mars's zone turns it into
/// instants (P9-20). `days` is read only for [`CalendarViewKind::Days`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RangeRequest {
    pub view: CalendarViewKind,
    pub anchor_ms: i64,
    pub days: u8,
    pub week_start: CalendarWeekStart,
}

impl RangeRequest {
    /// The week of `anchor_ms`, beginning on `week_start`.
    pub fn week(anchor_ms: i64, week_start: CalendarWeekStart) -> Self {
        Self {
            view: CalendarViewKind::Week,
            anchor_ms,
            days: 7,
            week_start,
        }
    }
}

/// The widest grid of N days; registered with its purpose in orbit-core.
pub use orbit_core::calendar::MAX_GRID_DAYS;

/// How far the agenda list reaches either side of its day (P9-20,
/// ±31 days): 63 days on screen, read in three daemon windows.
pub const AGENDA_REACH_DAYS: i64 = 31;

/// The widest read that still stretches the visible range to cover today.
///
/// Protects: the cost of one navigation. The read is the visible range
/// widened to include today so the Briefing keeps its day whatever month the
/// calendar shows; every 31 days of that width is one more EventKit window
/// (`orbit_core::calendar::READ_WINDOW_MS`) and its events crossing IPC on
/// every step. Why 93 days: three such windows, which covers a month view of
/// the month after next, or the agenda a month away, as one read. What it is
/// not: a limit on what the calendar shows. Past it the read is the visible
/// range alone and the Briefing names `agenda_window_excludes_today`.
pub const TODAY_REACH_DAYS: i64 = 93;

/// Why a range request cannot be read, with its numbers.
pub fn validate_range(request: &RangeRequest) -> Result<(), String> {
    if request.view == CalendarViewKind::Days && !(1..=MAX_GRID_DAYS).contains(&request.days) {
        return Err(format!(
            "vista de {} días: se admiten de 1 a {MAX_GRID_DAYS}",
            request.days
        ));
    }
    Ok(())
}

/// The period the calendar shows for `request`, in `zone` (P9-20).
///
/// Every edge is a local midnight (REF-40): a day is 23, 24 or 25 hours long,
/// so nothing here adds a fixed number of milliseconds.
pub fn visible_range(request: &RangeRequest, zone: &LocalZone) -> CalendarRangeSnapshot {
    let day = zone.day(request.anchor_ms);
    let week_start = |of: i64| of - weekday(of, request.week_start);
    let (first, last_exclusive) = match request.view {
        CalendarViewKind::Day => (day, day + 1),
        CalendarViewKind::Days => {
            let days = i64::from(request.days.clamp(1, MAX_GRID_DAYS));
            // A grid as wide as a week or wider lines up with
            // the week; a narrower one begins on the day asked for.
            let first = if days >= 7 { week_start(day) } else { day };
            (first, first + days)
        }
        CalendarViewKind::Week => (week_start(day), week_start(day) + 7),
        CalendarViewKind::Month => {
            let (year, month, date) = civil_from_days(day);
            let first_of_month = day - i64::from(date) + 1;
            let last_of_month = days_from_civil(next_month(year, month), 1) - 1;
            let first = week_start(first_of_month);
            let last = week_start(last_of_month) + 6;
            (first, last + 1)
        }
        CalendarViewKind::Agenda => (day - AGENDA_REACH_DAYS, day + AGENDA_REACH_DAYS + 1),
    };
    CalendarRangeSnapshot {
        view: request.view,
        days: u8::try_from(last_exclusive - first).unwrap_or(u8::MAX),
        week_start: request.week_start,
        anchor_ms: zone.start_of_day(day),
        start_ms: zone.start_of_day(first),
        end_ms: zone.start_of_day(last_exclusive),
    }
}

/// What to ask the daemon for: the visible range, widened to cover today
/// unless that would read more than [`TODAY_REACH_DAYS`] (P9-20).
pub fn read_window(range: &CalendarRangeSnapshot, now_ms: i64, zone: &LocalZone) -> (i64, i64) {
    let today = zone.day(now_ms);
    let first = zone.day(range.start_ms).min(today);
    let last_exclusive = zone.day(range.end_ms.saturating_sub(1)).max(today) + 1;
    if last_exclusive - first > TODAY_REACH_DAYS {
        return (range.start_ms, range.end_ms);
    }
    (zone.start_of_day(first), zone.start_of_day(last_exclusive))
}

/// Whether `window` holds the whole local day of `now_ms` (P9-20).
pub fn covers_today(window: (i64, i64), now_ms: i64, zone: &LocalZone) -> bool {
    let day = zone.day(now_ms);
    window.0 <= zone.start_of_day(day) && window.1 >= zone.start_of_day(day.saturating_add(1))
}

/// What to do about today for the read on record (P9-20, the Briefing's day).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TodayRead {
    /// The read holds today: nothing to do.
    Covered,
    /// A new read of the same range would hold today — the day changed at
    /// local midnight since the last one. Read again.
    ReadAgain,
    /// The range is further than [`TODAY_REACH_DAYS`] from today: return to
    /// today in the same view and read that.
    ReturnToToday,
}

/// Pure: the read on record, the range on screen, the instant and the zone.
pub fn today_read(
    read: (i64, i64),
    range: &CalendarRangeSnapshot,
    now_ms: i64,
    zone: &LocalZone,
) -> TodayRead {
    if covers_today(read, now_ms, zone) {
        TodayRead::Covered
    } else if covers_today(read_window(range, now_ms, zone), now_ms, zone) {
        TodayRead::ReadAgain
    } else {
        TodayRead::ReturnToToday
    }
}

/// Days since the week began, for a local day counted from 1970-01-01, which
/// was a Thursday.
fn weekday(day: i64, week_start: CalendarWeekStart) -> i64 {
    match week_start {
        CalendarWeekStart::Monday => (day + 3).rem_euclid(7),
        CalendarWeekStart::Sunday => (day + 4).rem_euclid(7),
    }
}

fn next_month(year: i64, month: u32) -> (i64, u32) {
    if month == 12 {
        (year + 1, 1)
    } else {
        (year, month + 1)
    }
}

/// The civil date of a day counted from 1970-01-01 (proleptic Gregorian;
/// H. Hinnant's `civil_from_days`).
fn civil_from_days(day: i64) -> (i64, u32, u32) {
    let shifted = day + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let date = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    (
        year,
        u32::try_from(month).unwrap_or(1),
        u32::try_from(date).unwrap_or(1),
    )
}

/// The day, counted from 1970-01-01, of a civil date (`days_from_civil`).
fn days_from_civil((year, month): (i64, u32), date: u32) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year.rem_euclid(400);
    let month = i64::from(month);
    let month_index = if month > 2 { month - 3 } else { month + 9 };
    let day_of_year = (153 * month_index + 2) / 5 + i64::from(date) - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

pub(crate) struct CalendarCoordinator {
    clock: Arc<dyn CalendarClock>,
    request: RangeRequest,
}

impl CalendarCoordinator {
    /// P9-20: the calendar opens on this week, beginning on Monday; the
    /// interface asks for another view, or Sunday, when Mars chose one.
    pub(crate) fn new(clock: Arc<dyn CalendarClock>) -> Self {
        let now_ms = clock.now_ms();
        Self {
            clock,
            request: RangeRequest::week(now_ms, CalendarWeekStart::Monday),
        }
    }

    pub(crate) fn now_ms(&self) -> i64 {
        self.clock.now_ms()
    }

    /// The present a projection is made for: the instant and Mars's zone.
    pub(crate) fn local_now(&self) -> (i64, LocalZone) {
        let now_ms = self.clock.now_ms();
        (now_ms, self.clock.local_zone(now_ms))
    }

    /// P9-20 (after REF-40 A2): the interface asks for any instant of the
    /// period to show; the period itself is decided here, in the injected
    /// zone, so stepping across a daylight-saving change cannot land an hour
    /// off the midnight it means.
    pub(crate) fn set_range(&mut self, request: RangeRequest) -> Result<(), String> {
        validate_range(&request)?;
        self.request = request;
        Ok(())
    }

    pub(crate) fn local_clock(&self) -> LocalClock {
        LocalClock::read(self.clock.as_ref())
    }

    /// The period on screen.
    pub(crate) fn range(&self) -> CalendarRangeSnapshot {
        let zone = self.clock.local_zone(self.clock.now_ms());
        visible_range(&self.request, &zone)
    }

    /// What today needs from the read on record (see [`today_read`]).
    pub(crate) fn today_read(&self, read: (i64, i64)) -> TodayRead {
        let now_ms = self.clock.now_ms();
        let zone = self.clock.local_zone(now_ms);
        today_read(read, &visible_range(&self.request, &zone), now_ms, &zone)
    }

    /// The same view, around now.
    pub(crate) fn return_to_today(&mut self) {
        self.request.anchor_ms = self.clock.now_ms();
    }

    /// What the daemon is asked to read: the period on screen and today.
    pub(crate) fn read_window(&self) -> (i64, i64) {
        let now_ms = self.clock.now_ms();
        let zone = self.clock.local_zone(now_ms);
        read_window(&visible_range(&self.request, &zone), now_ms, &zone)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::local_time::ZoneTransition;
    use std::sync::{
        Mutex, PoisonError,
        atomic::{AtomicI64, Ordering},
    };

    struct FixedClock(i64);

    impl CalendarClock for FixedClock {
        fn now_ms(&self) -> i64 {
            self.0
        }

        // These tests read no wall-clock label; UTC keeps them where they were.
        fn local_zone(&self, _now_ms: i64) -> LocalZone {
            LocalZone::utc()
        }

        fn timezone_name(&self) -> Result<String, String> {
            Ok("UTC".into())
        }
    }

    struct AdjustableClock(AtomicI64);

    impl CalendarClock for AdjustableClock {
        fn now_ms(&self) -> i64 {
            self.0.load(Ordering::Relaxed)
        }

        fn local_zone(&self, _now_ms: i64) -> LocalZone {
            LocalZone::utc()
        }

        fn timezone_name(&self) -> Result<String, String> {
            Ok("UTC".into())
        }
    }

    #[test]
    fn visible_week_starts_on_sunday_with_an_injected_clock() {
        let mut state = CalendarCoordinator::new(Arc::new(FixedClock(1_789_469_100_000)));
        assert_eq!(
            state.set_range(RangeRequest::week(
                1_789_469_100_000,
                CalendarWeekStart::Sunday
            )),
            Ok(())
        );
        assert_eq!(
            (state.range().start_ms, state.range().end_ms),
            (1_789_257_600_000, 1_789_862_400_000)
        );
    }

    /// P9-20: with nothing chosen the calendar opens on this week, from
    /// Monday (Tuesday 15 September 2026 → Monday the 14th, UTC).
    #[test]
    fn p9_20_the_calendar_opens_on_this_week_from_monday() {
        let state = CalendarCoordinator::new(Arc::new(FixedClock(1_789_469_100_000)));
        assert_eq!(
            (state.range().start_ms, state.range().end_ms),
            (1_789_344_000_000, 1_789_948_800_000)
        );
        assert_eq!(state.range().view, CalendarViewKind::Week);
        assert_eq!(state.range().week_start, CalendarWeekStart::Monday);
    }

    #[tokio::test]
    async fn snapshot_publishes_the_clock_zone_before_any_agenda_read()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let paths = orbit_ipc::Paths::resolve(Some(directory.path().to_path_buf()))?;
        let client = crate::DaemonClient::connect_with_clock(
            paths,
            Arc::new(FixedClock(1_790_593_200_000)),
            crate::client_fixture::idle(),
        );
        let published = serde_json::to_value(client.composed_snapshot())?;
        assert_eq!(published["system_timezone"], "UTC");
        assert_eq!(published["calendar"]["window"]["timezone"], "");
        Ok(())
    }

    struct FailedZoneClock;

    impl CalendarClock for FailedZoneClock {
        fn now_ms(&self) -> i64 {
            1_790_593_200_000
        }

        fn local_zone(&self, _now_ms: i64) -> LocalZone {
            LocalZone::utc()
        }

        fn timezone_name(&self) -> Result<String, String> {
            Err("timezone_os_failed".into())
        }
    }

    #[tokio::test]
    async fn failed_system_zone_keeps_its_cause_in_the_snapshot()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let paths = orbit_ipc::Paths::resolve(Some(directory.path().to_path_buf()))?;
        let client = crate::DaemonClient::connect_with_clock(
            paths,
            Arc::new(FailedZoneClock),
            crate::client_fixture::idle(),
        );
        let published = client.composed_snapshot();
        assert_eq!(published.system_timezone, "");
        assert_eq!(
            published.system_timezone_error.as_deref(),
            Some("timezone_os_failed")
        );
        Ok(())
    }

    fn agenda_frame(
        revision: u64,
        knowledge: orbit_core::topics::Knowledge<orbit_core::topics::agenda::Value>,
    ) -> orbit_core::topics::TopicFrame {
        orbit_core::topics::TopicFrame {
            epoch: "calendar-test".into(),
            revision,
            data: orbit_core::topics::TopicData::Agenda(knowledge),
        }
    }

    fn granted_agenda() -> orbit_core::topics::Knowledge<orbit_core::topics::agenda::Value> {
        orbit_core::topics::Knowledge::Present {
            value: orbit_core::topics::agenda::Value {
                invalidation: 0,
                calendars: Some(0),
                read_window: None,
                read_id: None,
                timezone: None,
                writes: Vec::new(),
            },
            as_of_ms: None,
        }
    }

    /// REF-10 A1: a granted agenda head proves the macOS permission. With no
    /// status head the bridge capability remains unknown, never available.
    #[test]
    fn ref10_a1_a_granted_topic_is_known_and_an_unknown_one_waits() {
        use orbit_core::topics::{Knowledge, Pending};
        let snapshot = crate::AppSnapshot::default();
        let mut topics = crate::topics::TopicCache::default();
        topics.observe_at(
            agenda_frame(
                1,
                Knowledge::Unknown {
                    why: Pending::NotObservedYet,
                },
            ),
            Some(1),
        );
        let unknown = crate::features::agenda::project(&crate::features::FeatureInput::new(
            &snapshot, &topics,
        ));
        assert_eq!(unknown.reading.state, crate::CalendarReadingState::Unknown);
        assert_eq!(unknown.reading.blocker, None);
        topics.observe_at(agenda_frame(2, granted_agenda()), Some(2));
        let granted = crate::features::agenda::project(&crate::features::FeatureInput::new(
            &snapshot, &topics,
        ));
        assert_eq!(
            granted.authorization,
            crate::CalendarAuthorizationSnapshot::Granted
        );
        assert_eq!(granted.reading.state, crate::CalendarReadingState::Unknown);
        assert_eq!(
            granted.reading.detail.as_deref(),
            Some("status_not_observed_yet")
        );
    }

    /// Another topic does not evict the agenda permission that its own topic
    /// already reported.
    #[test]
    fn an_unrelated_topic_leaves_the_agenda_permission_known() {
        use orbit_core::topics::{TopicData, TopicFrame, TopicId};
        let snapshot = crate::AppSnapshot::default();
        let mut topics = crate::topics::TopicCache::default();
        topics.observe_at(agenda_frame(1, granted_agenda()), Some(1));
        topics.observe_at(
            TopicFrame {
                epoch: "calendar-test".into(),
                revision: 2,
                data: TopicData::not_observed(TopicId::Notes),
            },
            Some(2),
        );
        let view = crate::features::agenda::project(&crate::features::FeatureInput::new(
            &snapshot, &topics,
        ));
        assert_eq!(
            view.authorization,
            crate::CalendarAuthorizationSnapshot::Granted
        );
    }

    /// The daemon's refusal reaches the agenda view with its cause.
    #[test]
    fn a_failed_read_names_the_daemons_cause() {
        let snapshot = crate::AppSnapshot::default();
        let mut topics = crate::topics::TopicCache::default();
        topics.observe_at(
            agenda_frame(
                1,
                orbit_core::topics::Knowledge::Absent {
                    refusal: orbit_core::refusal::Refusal::CalendarBackendUnavailable,
                },
            ),
            Some(1),
        );
        let view = crate::features::agenda::project(&crate::features::FeatureInput::new(
            &snapshot, &topics,
        ));
        assert_eq!(view.reading.state, crate::CalendarReadingState::Unavailable);
        assert!(view.reading.detail.as_deref().is_some_and(|cause| {
            serde_json::from_str::<serde_json::Value>(cause)
                .is_ok_and(|v| v["code"] == "calendar_backend_unavailable")
        }));
    }

    /// A clock pinned to one instant in a zone written out by hand, the way
    /// REF-40 pins Mars's: the zone reaches the decision as a value.
    struct ZonedClock {
        now_ms: i64,
        zone: LocalZone,
    }

    impl ZonedClock {
        fn at(now_ms: i64, zone: LocalZone) -> Self {
            Self { now_ms, zone }
        }

        /// REF-40 pinned Sunday weeks; P9-20 keeps them as Mars's choice.
        fn coordinator(now_ms: i64, zone: LocalZone) -> CalendarCoordinator {
            let mut coordinator = CalendarCoordinator::new(Arc::new(Self::at(now_ms, zone)));
            sunday_week(&mut coordinator, now_ms);
            coordinator
        }
    }

    impl CalendarClock for ZonedClock {
        fn now_ms(&self) -> i64 {
            self.now_ms
        }

        fn local_zone(&self, _now_ms: i64) -> LocalZone {
            self.zone.clone()
        }

        fn timezone_name(&self) -> Result<String, String> {
            Ok("Europe/Madrid".into())
        }
    }

    fn sunday_week(coordinator: &mut CalendarCoordinator, anchor_ms: i64) {
        assert_eq!(
            coordinator.set_range(RangeRequest::week(anchor_ms, CalendarWeekStart::Sunday)),
            Ok(())
        );
    }

    struct RecordingZoneClock {
        now_ms: i64,
        zone: LocalZone,
        requests: Mutex<Vec<i64>>,
    }

    impl CalendarClock for RecordingZoneClock {
        fn now_ms(&self) -> i64 {
            self.now_ms
        }

        fn local_zone(&self, at_ms: i64) -> LocalZone {
            self.requests
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(at_ms);
            self.zone.clone()
        }

        fn timezone_name(&self) -> Result<String, String> {
            Ok("Europe/Madrid".into())
        }
    }

    const HOUR_MS: i64 = 3_600_000;
    // Europe/Madrid 2026: CET (+1), CEST (+2) from 29 March 01:00 UTC to
    // 25 October 01:00 UTC.
    const SPRING_2026: i64 = 1_774_746_000_000;
    const AUTUMN_2026: i64 = 1_792_890_000_000;

    fn madrid_2026() -> LocalZone {
        LocalZone::with_transitions(
            HOUR_MS,
            vec![
                ZoneTransition {
                    at_ms: AUTUMN_2026,
                    offset_ms: HOUR_MS,
                },
                ZoneTransition {
                    at_ms: SPRING_2026,
                    offset_ms: 2 * HOUR_MS,
                },
            ],
        )
    }

    /// America/New_York 2026: EST (−5), EDT (−4) from 8 March 07:00 UTC to
    /// 1 November 06:00 UTC.
    fn new_york_2026() -> LocalZone {
        LocalZone::with_transitions(
            -5 * HOUR_MS,
            vec![
                ZoneTransition {
                    at_ms: 1_772_953_200_000,
                    offset_ms: -4 * HOUR_MS,
                },
                ZoneTransition {
                    at_ms: 1_793_512_800_000,
                    offset_ms: -5 * HOUR_MS,
                },
            ],
        )
    }

    /// REF-40 A1: an event on Sunday at 00:30 in Madrid was in no week at all.
    ///
    /// The window was cut in UTC, where that Sunday begins at 02:00 Madrid
    /// time, so the first two local hours of every Sunday were before the
    /// week the app asked to read. The zone is the clock's own, injected.
    #[test]
    fn ref40_a1_a_madrid_sunday_half_past_midnight_is_inside_the_week() {
        // Sunday 20 September 2026, noon in Madrid (CEST, +2): the week on
        // screen has to have begun at 00:00 Madrid (Saturday 22:00 UTC) for
        // the 00:30 event (Saturday 22:30 UTC) to be in it.
        let coordinator = ZonedClock::coordinator(1_789_905_600_000, madrid_2026());
        let (start_ms, end_ms) = (coordinator.range().start_ms, coordinator.range().end_ms);
        assert_eq!(start_ms, 1_789_855_200_000);
        assert!(
            (start_ms..end_ms).contains(&1_789_857_000_000),
            "the 00:30 event at 1_789_857_000_000 fell outside the window \
             {start_ms}..{end_ms}"
        );
    }

    /// REF-40 A2: Madrid, through both daylight-saving changes. A local week
    /// is 167, 168 or 169 hours long; a fixed seven days cuts the last hour
    /// of the Saturday of the week the clocks go back.
    #[test]
    fn ref40_a2_madrid_weeks_run_from_midnight_to_midnight_across_both_changes() {
        let mut coordinator = ZonedClock::coordinator(1_789_905_600_000, madrid_2026());
        // The week the clocks go back (25 October 2026): 25 hours of Sunday,
        // 169 hours of week, ending at next Sunday's midnight, not an hour
        // before it.
        sunday_week(&mut coordinator, 1_792_879_200_000);
        let range = coordinator.range();
        assert_eq!(
            (range.start_ms, range.end_ms),
            (1_792_879_200_000, 1_793_487_600_000),
            "the autumn week must span local midnight to local midnight (169 h)"
        );
        // The week the clocks jump forward (29 March 2026): 23 hours of
        // Sunday, 167 hours of week.
        sunday_week(&mut coordinator, 1_774_738_800_000);
        let range = coordinator.range();
        assert_eq!(
            (range.start_ms, range.end_ms),
            (1_774_738_800_000, 1_775_340_000_000),
            "the spring week must span local midnight to local midnight (167 h)"
        );
    }

    /// REF-40 A2: a zone west of UTC. Cut in UTC, the week began at 20:00 of
    /// the Saturday before, in New York.
    #[test]
    fn ref40_a2_a_new_york_sunday_begins_at_midnight_new_york() {
        // Sunday 4 October 2026, 01:00 in New York (EDT, −4): the week began
        // at 00:00 New York (04:00 UTC), not at 00:00 UTC, which is still
        // Saturday evening there.
        let coordinator = ZonedClock::coordinator(1_791_090_000_000, new_york_2026());
        assert_eq!(
            coordinator.range().start_ms,
            1_791_086_400_000,
            "the week must begin at midnight in New York, not at midnight UTC"
        );
    }

    /// REF-40 A2: the interface steps whole weeks from the week it sees,
    /// which across a daylight-saving change lands an hour off the Sunday it
    /// means. The week asked for is the one the instant belongs to.
    #[test]
    fn ref40_a2_an_instant_an_hour_into_the_week_still_means_that_week() {
        let mut coordinator = ZonedClock::coordinator(1_791_090_000_000, new_york_2026());
        sunday_week(&mut coordinator, 1_791_086_400_000 + 7 * DAY_MS + 3_600_000);
        assert_eq!(
            coordinator.range().start_ms,
            1_791_086_400_000 + 7 * DAY_MS,
            "an instant an hour into the week still means that week's midnight"
        );
    }

    #[test]
    fn ref40_a2_week_navigation_and_snapshots_read_the_zone_around_now() {
        let now_ms = 1_789_905_600_000;
        let clock = Arc::new(RecordingZoneClock {
            now_ms,
            zone: madrid_2026(),
            requests: Mutex::new(Vec::new()),
        });
        let mut coordinator = CalendarCoordinator::new(clock.clone());
        clock
            .requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clear();

        sunday_week(&mut coordinator, AUTUMN_2026);
        let range = coordinator.range();
        assert_eq!(
            (range.start_ms, range.end_ms),
            (1_792_879_200_000, 1_793_487_600_000)
        );
        // The read covers the week on screen and today, from the same zone.
        let _ = coordinator.read_window();
        assert_eq!(
            *clock
                .requests
                .lock()
                .unwrap_or_else(PoisonError::into_inner),
            vec![now_ms, now_ms],
            "both decisions must use the current zone, so SystemZone can reuse its cached sample"
        );
    }

    // ------------------------------------------------------------ P9-20

    const DAY_MS: i64 = 86_400_000;
    /// Wednesday 14 October 2026, 10:00 UTC (12:00 in Madrid, CEST).
    const OCTOBER_14: i64 = 1_791_972_000_000;

    fn request(view: CalendarViewKind, anchor_ms: i64, days: u8) -> RangeRequest {
        RangeRequest {
            view,
            anchor_ms,
            days,
            week_start: CalendarWeekStart::Monday,
        }
    }

    fn span(range: &CalendarRangeSnapshot) -> (i64, i64, u8) {
        (range.start_ms, range.end_ms, range.days)
    }

    #[test]
    fn p9_20_a7_day_and_n_days_begin_on_the_day_asked_for() {
        let utc = LocalZone::utc();
        let midnight = OCTOBER_14 - 10 * 3_600_000;
        let day = visible_range(&request(CalendarViewKind::Day, OCTOBER_14, 9), &utc);
        assert_eq!(span(&day), (midnight, midnight + DAY_MS, 1));
        assert_eq!(day.anchor_ms, midnight);
        let three = visible_range(&request(CalendarViewKind::Days, OCTOBER_14, 3), &utc);
        assert_eq!(span(&three), (midnight, midnight + 3 * DAY_MS, 3));
        // Fourteen days (the 0 key) line up with the week, like a week does.
        let fortnight = visible_range(&request(CalendarViewKind::Days, OCTOBER_14, 14), &utc);
        assert_eq!(
            span(&fortnight),
            (midnight - 2 * DAY_MS, midnight + 12 * DAY_MS, 14)
        );
    }

    #[test]
    fn p9_20_a7_a_week_begins_on_monday_or_on_sunday() {
        let utc = LocalZone::utc();
        let monday = OCTOBER_14 - 10 * 3_600_000 - 2 * DAY_MS;
        let week = visible_range(&request(CalendarViewKind::Week, OCTOBER_14, 0), &utc);
        assert_eq!(span(&week), (monday, monday + 7 * DAY_MS, 7));
        let sunday_week = visible_range(
            &RangeRequest::week(OCTOBER_14, CalendarWeekStart::Sunday),
            &utc,
        );
        assert_eq!(
            span(&sunday_week),
            (monday - DAY_MS, monday + 6 * DAY_MS, 7)
        );
    }

    #[test]
    fn p9_20_a7_a_month_is_its_weeks_whole() {
        let utc = LocalZone::utc();
        // October 2026 begins on a Thursday and ends on a Saturday: from
        // Monday 28 September to Sunday 1 November, five weeks.
        let october = visible_range(&request(CalendarViewKind::Month, OCTOBER_14, 0), &utc);
        let monday_28 = OCTOBER_14 - 10 * 3_600_000 - 16 * DAY_MS;
        assert_eq!(span(&october), (monday_28, monday_28 + 35 * DAY_MS, 35));
        // August 2026 begins on a Saturday and ends on a Monday: six weeks.
        let august = visible_range(
            &request(CalendarViewKind::Month, OCTOBER_14 - 60 * DAY_MS, 0),
            &utc,
        );
        assert_eq!(august.days, 42);
        assert_eq!(utc.day(august.start_ms), utc.day(OCTOBER_14) - 79);
        // Leap years: February 2028 has 29 days, from Tuesday 1 to Tuesday
        // 29, so its weeks run from Monday 31 January to Sunday 5 March.
        let february_2028 = visible_range(
            &request(CalendarViewKind::Month, 1_833_000_000_000, 0),
            &utc,
        );
        assert_eq!(february_2028.days, 35);
        assert_eq!(
            civil_from_days(utc.day(february_2028.start_ms)),
            (2028, 1, 31)
        );
        assert_eq!(
            civil_from_days(utc.day(february_2028.end_ms) - 1),
            (2028, 3, 5)
        );
    }

    #[test]
    fn p9_20_a7_a_madrid_month_ends_at_the_cet_midnight_after_the_change() {
        // October 2026 in Madrid: the clocks go back on Sunday the 25th, so
        // the month's weeks end at 00:00 CET on Monday 2 November.
        let october = visible_range(
            &request(CalendarViewKind::Month, OCTOBER_14, 0),
            &madrid_2026(),
        );
        assert_eq!(
            october.start_ms, 1_790_546_400_000,
            "00:00 CEST, 28 September"
        );
        assert_eq!(october.end_ms, 1_793_574_000_000, "00:00 CET, 2 November");
        assert_eq!(october.days, 35);
    }

    #[test]
    fn p9_20_a7_the_agenda_reaches_31_days_either_side() {
        let utc = LocalZone::utc();
        let midnight = OCTOBER_14 - 10 * 3_600_000;
        let agenda = visible_range(&request(CalendarViewKind::Agenda, OCTOBER_14, 0), &utc);
        assert_eq!(
            span(&agenda),
            (midnight - 31 * DAY_MS, midnight + 32 * DAY_MS, 63)
        );
    }

    #[test]
    fn p9_20_a7_the_read_covers_today_unless_it_is_too_far() {
        let utc = LocalZone::utc();
        let midnight = OCTOBER_14 - 10 * 3_600_000;
        // November's weeks (26 October – 6 December) widened back to today.
        let november = visible_range(
            &request(CalendarViewKind::Month, OCTOBER_14 + 30 * DAY_MS, 0),
            &utc,
        );
        assert_eq!(
            read_window(&november, OCTOBER_14, &utc),
            (midnight, november.end_ms)
        );
        // A past day: from it to the end of today.
        let last_month = visible_range(
            &request(CalendarViewKind::Day, OCTOBER_14 - 30 * DAY_MS, 0),
            &utc,
        );
        assert_eq!(
            read_window(&last_month, OCTOBER_14, &utc),
            (last_month.start_ms, midnight + DAY_MS)
        );
        // Today in view: the read is the view.
        let week = visible_range(&request(CalendarViewKind::Week, OCTOBER_14, 0), &utc);
        assert_eq!(
            read_window(&week, OCTOBER_14, &utc),
            (week.start_ms, week.end_ms)
        );
        // Past TODAY_REACH_DAYS, the visible range alone.
        let far = visible_range(
            &request(CalendarViewKind::Week, OCTOBER_14 + 120 * DAY_MS, 0),
            &utc,
        );
        assert_eq!(
            read_window(&far, OCTOBER_14, &utc),
            (far.start_ms, far.end_ms)
        );
        let edge = visible_range(
            &request(
                CalendarViewKind::Day,
                midnight + (TODAY_REACH_DAYS - 1) * DAY_MS,
                0,
            ),
            &utc,
        );
        assert_eq!(
            read_window(&edge, OCTOBER_14, &utc),
            (midnight, edge.end_ms),
            "exactly TODAY_REACH_DAYS days still reach today"
        );
    }

    #[test]
    fn p9_20_a7_a_grid_wider_than_fourteen_days_is_refused_with_its_numbers() {
        let mut coordinator = CalendarCoordinator::new(Arc::new(FixedClock(OCTOBER_14)));
        for days in [0, 15] {
            let refused = coordinator.set_range(request(CalendarViewKind::Days, OCTOBER_14, days));
            assert_eq!(
                refused,
                Err(format!("vista de {days} días: se admiten de 1 a 14"))
            );
        }
        assert_eq!(
            coordinator.range().view,
            CalendarViewKind::Week,
            "unchanged"
        );
        assert_eq!(
            coordinator.set_range(request(CalendarViewKind::Month, OCTOBER_14, 0)),
            Ok(()),
            "days counts only for the N-day grid"
        );
    }

    #[test]
    fn p9_20_3a_after_local_midnight_the_week_is_read_again_to_hold_the_new_day() {
        // Sunday 20 September 2026, 23:30 UTC: the Monday week ends at midnight.
        let sunday_night = 1_789_947_000_000;
        let clock = Arc::new(AdjustableClock(AtomicI64::new(sunday_night)));
        let coordinator = CalendarCoordinator::new(clock.clone());
        let read = coordinator.read_window();
        assert_eq!(coordinator.today_read(read), TodayRead::Covered);
        // Monday 00:30: the read ends where the new day begins.
        clock.0.store(sunday_night + 3_600_000, Ordering::Relaxed);
        assert_eq!(read.1, sunday_night + 1_800_000);
        assert_eq!(coordinator.today_read(read), TodayRead::ReadAgain);
        let again = coordinator.read_window();
        assert_eq!(
            again,
            (read.0, read.1 + DAY_MS),
            "the same week, widened to Monday"
        );
        assert_eq!(coordinator.today_read(again), TodayRead::Covered);
    }

    #[test]
    fn p9_20_3b_a_range_beyond_reach_returns_to_today_in_the_same_view() {
        let clock = Arc::new(AdjustableClock(AtomicI64::new(OCTOBER_14)));
        let mut coordinator = CalendarCoordinator::new(clock);
        let far = OCTOBER_14 + 200 * DAY_MS;
        assert_eq!(
            coordinator.set_range(request(CalendarViewKind::Month, far, 0)),
            Ok(())
        );
        let read = coordinator.read_window();
        assert_eq!(coordinator.today_read(read), TodayRead::ReturnToToday);
        coordinator.return_to_today();
        assert_eq!(coordinator.range().view, CalendarViewKind::Month);
        assert!((coordinator.range().start_ms..coordinator.range().end_ms).contains(&OCTOBER_14));
        assert_eq!(
            coordinator.today_read(coordinator.read_window()),
            TodayRead::Covered
        );
    }
}
