/**
 * How long the unit suite waits before it calls something a failure (REF-08).
 *
 * A budget here bounds a hang; it must never decide a result. So each one is
 * derived from what it bounds, measured with the machine loaded the way the
 * agents load it, by the rule SYS-10 set for every wait in this repository
 * (`crates/orbit-daemon/tests/track_signal_lifecycle.rs` applies it too): a budget
 * clears the worst observation five times over and the p99 ten times over.
 *
 * Measured on 2026-09-25 on the 32-core Linux server, with the lazy views
 * loaded ahead as `test-setup.ts` does and 24 `yes` burners on top of the other
 * agents. 60 runs of `vitest run` were instrumented, with every budget lifted
 * so that no wait was cut short and the tail was seen whole, at a one-minute
 * load average between 35 and 102. The test durations add 60 uninstrumented
 * runs made to verify it, at a load between 45 and 121, where no test came
 * near its budget, and two runs of `scripts/check.sh ui`. One of those two ran through a stall of the disk and
 * the memory (pressure `full` at 34 % and 19 % over the minute) and gave the
 * worst test, 13 827 ms, where every other run stayed under 8 862 ms; its
 * queries were not instrumented, and none of them ran out. In milliseconds:
 *
 *     wait                                     n     p50    p95    p99   worst
 *     a findBy or waitFor, until it holds  10121     137   1045   1558    2833
 *     one test, hooks included             18910     396   2852   4995   13827
 *     loading the lazy views, per file      3120     303   1155   1674    6081
 *     afterEach(cleanup)                    9300       0      1     11      66
 *
 * The defaults these replace were below their own p99: Testing Library's
 * 1 000 ms for a query and Vitest's 5 000 ms for a test. With the lazy views
 * already loaded and those defaults kept, 15 runs under the same load ran out
 * of one or the other 18 times, in 9 of the runs.
 *
 * Vitest's 10 000 ms for a hook stays: an ordinary hook here is `cleanup` and
 * a line of state, and that default is already 151 times its worst.
 *
 * SIM-00 (2026-10-15) removed the suites that rendered the whole app over the
 * fake bridge, and with them the hook that loaded the lazy views; these
 * numbers were measured on the larger suite and bound the one that remains.
 */

/** SYS-10: a budget clears the worst observation this many times over... */
const REQUIRED_WORST_MARGIN = 5;
/** ...and the p99 this many times over. */
const REQUIRED_P99_MARGIN = 10;

function budgetFor(measured: { p99Ms: number; worstMs: number }): number {
  return Math.max(REQUIRED_P99_MARGIN * measured.p99Ms, REQUIRED_WORST_MARGIN * measured.worstMs);
}

/** A `findBy*` or `waitFor` waiting for what it asks about: 15 580 ms. */
export const QUERY_BUDGET_MS = budgetFor({ p99Ms: 1558, worstMs: 2833 });

/** One test, measured with its hooks, which only makes it longer: 69 135 ms. */
export const TEST_BUDGET_MS = budgetFor({ p99Ms: 4995, worstMs: 13827 });
