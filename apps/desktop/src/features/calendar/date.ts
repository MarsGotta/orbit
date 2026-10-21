import { isoWeekForDay } from "../../local-time";

export const DAY_MS = 24 * 60 * 60 * 1_000;
const MINUTE_MS = 60_000;

// Days are counted from 1970-01-01 on the wall clock of the zone the daemon
// publishes (REF-40): a day is 23, 24 or 25 hours long, so nothing here adds a
// fixed number of milliseconds to an instant. app-core decides which days a
// view shows (P9-20); this only names and places them.

/** The civil day `timestamp` falls on in `timezone`. */
export function dayOf(timestamp: number, timezone: string): number {
  return zonedPosition(timestamp, timezone).day;
}

/** The first instant of civil `day` in `timezone` (REF-40). */
export function startOfDay(day: number, timezone: string): number {
  return instantAtWallDay(day, timezone);
}

export interface CivilDate {
  /** 1–31 */
  date: number;
  /** 1–12 */
  month: number;
  /** 0 = Monday … 6 = Sunday */
  weekday: number;
  year: number;
}

export function civil(day: number): CivilDate {
  const date = new Date(day * DAY_MS);
  return {
    date: date.getUTCDate(),
    month: date.getUTCMonth() + 1,
    weekday: (date.getUTCDay() + 6) % 7,
    year: date.getUTCFullYear(),
  };
}

export function isWeekend(day: number): boolean {
  return civil(day).weekday >= 5;
}

/** The days a range covers, as civil days of `timezone`. */
export function rangeDays(range: { days: number; start_ms: number }, timezone: string): number[] {
  const first = dayOf(range.start_ms, timezone);
  return Array.from({ length: Math.max(range.days, 0) }, (_, index) => first + index);
}

/** «Octubre 2026». */
export function monthTitle(day: number): string {
  const value = new Intl.DateTimeFormat("es-ES", {
    month: "long",
    timeZone: "UTC",
    year: "numeric",
  }).format(day * DAY_MS);
  return (value.charAt(0).toUpperCase() + value.slice(1)).replace(" de ", " ");
}

/** The ISO week number of a civil day. */
export function weekNumber(day: number): number {
  return isoWeekForDay(day);
}

/** «lun» and «14» for a day header. */
export function dayHeading(day: number): { name: string; number: number } {
  const name = new Intl.DateTimeFormat("es-ES", { timeZone: "UTC", weekday: "short" })
    .format(day * DAY_MS)
    .replace(".", "");
  return { name, number: civil(day).date };
}

/** «14 oct». */
export function shortDate(day: number): { month: string; number: number } {
  const month = new Intl.DateTimeFormat("es-ES", { month: "short", timeZone: "UTC" })
    .format(day * DAY_MS)
    .replace(".", "");
  return { month, number: civil(day).date };
}

export function timezoneCity(timezone: string): string {
  const city = timezone.split("/").at(-1)?.replaceAll("_", " ");
  return city != null && city !== "" ? city : timezone;
}

export { effectiveTimezone } from "../../local-time";

interface ZonedPosition {
  day: number;
  minutes: number;
  seconds: number;
}

function zonedPosition(timestamp: number, timezone: string): ZonedPosition {
  const parts = new Intl.DateTimeFormat("en-US", {
    day: "numeric",
    hour: "numeric",
    hourCycle: "h23",
    minute: "numeric",
    month: "numeric",
    second: "numeric",
    timeZone: timezone,
    year: "numeric",
  }).formatToParts(timestamp);
  const values = Object.fromEntries(parts.map((part) => [part.type, Number(part.value)]));
  return {
    day: Date.UTC(values.year ?? 0, (values.month ?? 1) - 1, values.day ?? 1) / DAY_MS,
    minutes: (values.hour ?? 0) * 60 + (values.minute ?? 0),
    seconds: values.second ?? 0,
  };
}

// The wall clock at `instant`, counted from the civil midnight of 1970-01-01
// of that same clock. Every offset in the tz database of 2026 is whole
// minutes; the seconds keep the comparison below exact anyway.
function wallMsAt(instant: number, timezone: string): number {
  const position = zonedPosition(instant, timezone);
  return position.day * DAY_MS + position.minutes * MINUTE_MS + position.seconds * 1_000;
}

function offsetAtInstant(instant: number, timezone: string): number {
  return wallMsAt(instant, timezone) - instant;
}

// The first instant at which the wall clock reads midnight of `day` (REF-40).
// When the clocks go back a wall time happens twice and this is the first;
// when they go forward a midnight may never happen and this is where the
// clock jumped past it. Candidates come from the offsets in force a day
// either side, the same way app-core's `LocalZone::instant_at` reads it.
function instantAtWallDay(day: number, timezone: string): number {
  return instantAtWallMs(day * DAY_MS, timezone);
}

/** The first instant the wall clock of `timezone` reads `wallMs` (counted
 *  from its own 1970-01-01 midnight); past a jump forward, where the jump
 *  lands. Any wall time, not only a midnight (P9-22). */
export function instantAtWallMs(wallMs: number, timezone: string): number {
  const before = offsetAtInstant(wallMs - DAY_MS, timezone);
  const after = offsetAtInstant(wallMs + DAY_MS, timezone);
  const candidates = [wallMs - before, wallMs - after].filter(
    (candidate) => wallMsAt(candidate, timezone) === wallMs,
  );
  return candidates.length > 0 ? Math.min(...candidates) : wallMs - before;
}

export function eventMinutes(timestamp: number, timezone: string): number {
  return zonedPosition(timestamp, timezone).minutes;
}

export interface EventSegment {
  day: number;
  endMinutes: number;
  startMinutes: number;
}

/** The pieces of an event on each civil day of `days` it touches. */
export function eventSegments(
  startMs: number,
  endMs: number,
  days: readonly number[],
  timezone: string,
): EventSegment[] {
  const start = zonedPosition(startMs, timezone);
  const end = zonedPosition(endMs, timezone);
  // An event ending on local midnight ends on the day it ends on; painting a
  // zero-length box on the next day would only occlude that column.
  const lastDay = end.minutes === 0 && end.day > start.day ? end.day - 1 : end.day;
  // Only the days the grid paints: an event far longer than the view (a
  // hundred-year one measured 36,502 segments) is cut to the columns that
  // exist, and days outside the view are dropped, not accumulated.
  return days
    .filter((day) => day >= start.day && day <= Math.max(start.day, lastDay))
    .map((day) => ({
      day,
      endMinutes: day === end.day ? end.minutes : 24 * 60,
      startMinutes: day === start.day ? start.minutes : 0,
    }));
}

/** «9:00»: 24-hour clock, no leading zero on the hour. */
export function hourLabel(hour: number): string {
  return `${hour}:00`;
}

/** «9:00», «10:45»: the clock Mars reads, in the zone the daemon publishes. */
export function timeLabel(timestamp: number, timezone: string): string {
  return new Intl.DateTimeFormat("es-ES", {
    hour: "numeric",
    hourCycle: "h23",
    minute: "2-digit",
    timeZone: timezone,
  }).format(timestamp);
}
