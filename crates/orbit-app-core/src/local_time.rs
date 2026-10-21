//! The user's wall clock, as data the projection reads.
//!
//! "Hoy", the cut between one day and the next, the focus window and a label
//! such as «10:00» are read on the clock of the Mac the user is sitting at. Read in
//! UTC, every hour of the Briefing came out two hours early in Madrid in
//! September. The zone is read where the app runs (`crate::system_zone`) and
//! handed to the projection as a value, the way `now_ms` is, so the projection
//! stays a function of what it is given and a test can pin both.

/// Milliseconds in a day of the civil calendar, the unit local days are
/// counted in. A local day itself can be 23 or 25 hours long.
pub(crate) const DAY_MS: i64 = 24 * 60 * 60 * 1_000;
const MINUTE_MS: i64 = 60 * 1_000;

/// From `at_ms` on, the wall clock is `offset_ms` ahead of UTC.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ZoneTransition {
    pub at_ms: i64,
    pub offset_ms: i64,
}

/// A time zone as its offsets from UTC over time.
///
/// Built from the rules the Mac uses (`crate::system_zone`) or, in a test,
/// written out by hand. Before the first transition `initial_offset_ms`
/// applies; after the last one, the last one's offset.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocalZone {
    initial_offset_ms: i64,
    transitions: Vec<ZoneTransition>,
}

impl LocalZone {
    pub fn utc() -> Self {
        Self::fixed(0)
    }

    pub fn fixed(offset_ms: i64) -> Self {
        Self {
            initial_offset_ms: offset_ms,
            transitions: Vec::new(),
        }
    }

    pub fn with_transitions(initial_offset_ms: i64, mut transitions: Vec<ZoneTransition>) -> Self {
        transitions.sort_by_key(|transition| transition.at_ms);
        Self {
            initial_offset_ms,
            transitions,
        }
    }

    /// How far ahead of UTC the wall clock is at `at_ms`.
    pub fn offset_ms_at(&self, at_ms: i64) -> i64 {
        let later = self
            .transitions
            .partition_point(|transition| transition.at_ms <= at_ms);
        later
            .checked_sub(1)
            .and_then(|index| self.transitions.get(index))
            .map_or(self.initial_offset_ms, |transition| transition.offset_ms)
    }

    /// What the wall clock reads at `at_ms`, counted as milliseconds since
    /// 1970-01-01 00:00 of that same clock.
    pub fn wall_ms(&self, at_ms: i64) -> i64 {
        at_ms.saturating_add(self.offset_ms_at(at_ms))
    }

    /// The local calendar day `at_ms` falls on, in days since 1970-01-01.
    pub fn day(&self, at_ms: i64) -> i64 {
        self.wall_ms(at_ms).div_euclid(DAY_MS)
    }

    /// Minutes since local midnight at `at_ms`.
    pub fn minute_of_day(&self, at_ms: i64) -> i64 {
        self.wall_ms(at_ms).rem_euclid(DAY_MS) / MINUTE_MS
    }

    /// The first instant at which the wall clock reads `wall_ms`.
    ///
    /// When the clocks go back a wall time happens twice, and this is the
    /// first. When they go forward a wall time never happens, and this is the
    /// instant the clock jumped past it, which reads that much later.
    /// Candidates are the offsets in force a day either side, so two changes
    /// within two days would be read as one; no zone does that.
    pub fn instant_at(&self, wall_ms: i64) -> i64 {
        let before = self.offset_ms_at(wall_ms.saturating_sub(DAY_MS));
        let after = self.offset_ms_at(wall_ms.saturating_add(DAY_MS));
        [before, after]
            .into_iter()
            .map(|offset| wall_ms.saturating_sub(offset))
            .filter(|candidate| self.wall_ms(*candidate) == wall_ms)
            .min()
            .unwrap_or_else(|| wall_ms.saturating_sub(before))
    }

    /// The instant the local `day` begins.
    pub fn start_of_day(&self, day: i64) -> i64 {
        self.instant_at(day.saturating_mul(DAY_MS))
    }
}

/// Reads a zone's changes out of `offset_at`, one day at a time, over the
/// `before_ms` before `now_ms` and the `after_ms` after it.
///
/// Where the offset differs between two consecutive days, the change is
/// searched for down to the millisecond, so a transition lands where the
/// rules put it. A day holding two changes would be read as one; no zone in
/// the tz database does that.
pub(crate) fn sample_zone(
    now_ms: i64,
    before_ms: i64,
    after_ms: i64,
    offset_at: impl Fn(i64) -> i64,
) -> LocalZone {
    let start = now_ms.saturating_sub(before_ms);
    let end = now_ms.saturating_add(after_ms);
    let initial = offset_at(start);
    let mut transitions = Vec::new();
    let (mut previous_at, mut previous) = (start, initial);
    while previous_at < end {
        let at = previous_at.saturating_add(DAY_MS).min(end);
        let offset = offset_at(at);
        if offset != previous {
            let (mut unchanged, mut changed) = (previous_at, at);
            while changed - unchanged > 1 {
                let middle = unchanged + (changed - unchanged) / 2;
                if offset_at(middle) == previous {
                    unchanged = middle;
                } else {
                    changed = middle;
                }
            }
            transitions.push(ZoneTransition {
                at_ms: changed,
                offset_ms: offset,
            });
        }
        (previous_at, previous) = (at, offset);
    }
    LocalZone::with_transitions(initial, transitions)
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOUR_MS: i64 = 60 * MINUTE_MS;
    // Europe/Madrid in 2026 as the tz database has it: CET (+1), CEST (+2)
    // from 29 March 01:00 UTC to 25 October 01:00 UTC.
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

    fn madrid_rule(at_ms: i64) -> i64 {
        if (SPRING_2026..AUTUMN_2026).contains(&at_ms) {
            2 * HOUR_MS
        } else {
            HOUR_MS
        }
    }

    #[test]
    fn the_offset_changes_exactly_at_each_transition() {
        let zone = madrid_2026();
        assert_eq!(zone.offset_ms_at(SPRING_2026 - 1), HOUR_MS);
        assert_eq!(zone.offset_ms_at(SPRING_2026), 2 * HOUR_MS);
        assert_eq!(zone.offset_ms_at(AUTUMN_2026 - 1), 2 * HOUR_MS);
        assert_eq!(zone.offset_ms_at(AUTUMN_2026), HOUR_MS);
        assert_eq!(LocalZone::utc().offset_ms_at(AUTUMN_2026), 0);
    }

    #[test]
    fn a_wall_time_that_happens_twice_is_its_first_instant() {
        let zone = madrid_2026();
        // 02:30 on 25 October: 00:30 UTC in CEST and 01:30 UTC in CET.
        let half_past_two = zone.wall_ms(AUTUMN_2026 - HOUR_MS / 2);
        assert_eq!(zone.instant_at(half_past_two), AUTUMN_2026 - HOUR_MS / 2);
        assert_eq!(zone.wall_ms(AUTUMN_2026 + HOUR_MS / 2), half_past_two);
    }

    #[test]
    fn a_wall_time_that_never_happens_is_where_the_clock_jumped_past_it() {
        let zone = madrid_2026();
        // 02:30 on 29 March does not exist: at 02:00 CET the clock reads 03:00.
        let two_o_clock = zone.wall_ms(SPRING_2026 - 1) + 1;
        let half_past_two = two_o_clock + HOUR_MS / 2;
        assert_eq!(zone.instant_at(two_o_clock), SPRING_2026);
        assert_eq!(zone.instant_at(half_past_two), SPRING_2026 + HOUR_MS / 2);
        assert_eq!(
            zone.minute_of_day(zone.instant_at(half_past_two)),
            3 * 60 + 30
        );
    }

    #[test]
    fn the_day_the_clocks_go_back_has_twenty_five_hours() {
        let zone = madrid_2026();
        let sunday = zone.day(AUTUMN_2026);
        let length = zone.start_of_day(sunday + 1) - zone.start_of_day(sunday);
        assert_eq!(length, 25 * HOUR_MS);
        let spring = zone.day(SPRING_2026);
        let length = zone.start_of_day(spring + 1) - zone.start_of_day(spring);
        assert_eq!(length, 23 * HOUR_MS);
    }

    #[test]
    fn sampling_finds_each_change_to_the_millisecond() {
        let now = SPRING_2026 + 100 * DAY_MS;
        let zone = sample_zone(now, 400 * DAY_MS, 400 * DAY_MS, madrid_rule);
        assert_eq!(zone, madrid_2026());
    }

    #[test]
    fn sampling_a_zone_without_changes_has_none() {
        let zone = sample_zone(AUTUMN_2026, 30 * DAY_MS, 30 * DAY_MS, |_| -3 * HOUR_MS);
        assert_eq!(zone, LocalZone::fixed(-3 * HOUR_MS));
    }
}
