/**
 * The showcase's instant: Wednesday 21 October 2026, 10:40 in Madrid. Every
 * invented date is placed around it, and the showcase's clock starts there
 * (`clock.ts`), so the agenda, the briefing and the notes agree on «today».
 */
export const ZONE = "Europe/Madrid";

const YEAR = 2026;
const OCTOBER = 9;
const TODAY = 21;
/** Summer time in Madrid ends on Sunday 25 October 2026 at 03:00. */
const SUMMER_TIME_ENDS = 25;

/** The instant of `hours:minutes` in Madrid, `day` days after the showcase's today. */
export function at(day: number, hours: number, minutes = 0): number {
  const date = TODAY + day;
  const offset = date >= SUMMER_TIME_ENDS ? 1 : 2;
  return Date.UTC(YEAR, OCTOBER, date, hours - offset, minutes);
}

export const NOW = at(0, 10, 40);

export const MINUTE = 60_000;
export const HOUR = 60 * MINUTE;
