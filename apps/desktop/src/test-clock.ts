/**
 * The clock the interface suite runs on: a pinned instant and a pinned zone,
 * so no result depends on the day or the place the suite runs. `test-setup.ts`
 * pins `Date.now()` to `TEST_NOW_MS`; `vite.config.ts` sets `TEST_TIMEZONE`.
 */

const MINUTE_MS = 60_000;
const DAY_MS = 24 * 60 * MINUTE_MS;

/** A UTC Sunday midnight: the week the calendar tests lay out (REF-40). */
export const FIXTURE_WEEK_START_MS = 1_789_257_600_000;

/** "Now": the third day of that week at 14:30 UTC, inside the grid's visible
 * band (06:00-22:00). */
export const TEST_NOW_MS = FIXTURE_WEEK_START_MS + 2 * DAY_MS + (14 * 60 + 30) * MINUTE_MS;

/** The zone the suite runs in, so labels that read the ambient zone do not
 * depend on where it ran. */
export const TEST_TIMEZONE = "UTC";
