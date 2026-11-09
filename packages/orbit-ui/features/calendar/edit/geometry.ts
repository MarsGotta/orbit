/**
 * Where a gesture puts an event. Pure: civil days,
 * minutes of the day and instants in, instants out. Every time is a wall time
 * of `timezone` turned into an instant through `instantAtWallMs`, so a day of
 * 23 or 25 hours places 9:00 at 9:00.
 */
import { DAY_MS, dayOf, eventMinutes, instantAtWallMs, startOfDay } from "../date";

const MINUTE_MS = 60_000;
const DAY_MINUTES = 24 * 60;
/** Every gesture lands on a quarter of an hour. */
export const SNAP_MINUTES = 15;
/** A click makes an event this long by default. */
export const CLICK_MINUTES = 30;
/** No gesture makes an event shorter than this. */
export const MIN_MINUTES = 15;

export interface Span {
  endMs: number;
  startMs: number;
}

export function floorQuarter(minutes: number): number {
  return Math.floor(minutes / SNAP_MINUTES) * SNAP_MINUTES;
}

export function roundQuarter(minutes: number): number {
  return Math.round(minutes / SNAP_MINUTES) * SNAP_MINUTES;
}

function clampDay(minutes: number): number {
  return Math.min(DAY_MINUTES, Math.max(0, minutes));
}

/** The instant the wall clock reads `minutes` into civil `day`. */
export function atWall(day: number, minutes: number, timezone: string): number {
  return instantAtWallMs(day * DAY_MS + minutes * MINUTE_MS, timezone);
}

/** The minute of the day at `y` pixels from midnight in a grid of `hourPx`. */
export function minutesAtY(y: number, hourPx: number): number {
  return clampDay((y / hourPx) * 60);
}

/**
 * A click in a free slot: from the quarter at or above the click, thirty
 * minutes, cut where the day's next event starts, never under fifteen and
 * never past midnight.
 */
export function clickSlot(
  day: number,
  minutes: number,
  timezone: string,
  nextStarts: readonly number[],
): Span {
  const start = Math.min(floorQuarter(clampDay(minutes)), DAY_MINUTES - MIN_MINUTES);
  const startMs = atWall(day, start, timezone);
  const end = Math.min(start + CLICK_MINUTES, DAY_MINUTES);
  let endMs = atWall(day, end, timezone);
  const next = Math.min(...nextStarts.filter((instant) => instant > startMs));
  if (next < endMs) endMs = Math.max(next, atWall(day, start + MIN_MINUTES, timezone));
  return { endMs, startMs };
}

/** A drag down (or up) one day: both ends to the nearest quarter, fifteen
 *  minutes at least, inside the day. */
export function dragSlot(day: number, from: number, to: number, timezone: string): Span {
  let start = roundQuarter(clampDay(Math.min(from, to)));
  let end = roundQuarter(clampDay(Math.max(from, to)));
  if (end - start < MIN_MINUTES) end = start + MIN_MINUTES;
  if (end > DAY_MINUTES) {
    end = DAY_MINUTES;
    start = end - MIN_MINUTES;
  }
  return { endMs: atWall(day, end, timezone), startMs: atWall(day, start, timezone) };
}

/**
 * A card dragged `dayDelta` columns and `minuteDelta` minutes: its start
 * lands on the nearest quarter and it keeps its length to the millisecond.
 */
export function movedSpan(
  event: Span,
  dayDelta: number,
  minuteDelta: number,
  timezone: string,
): Span {
  const day = dayOf(event.startMs, timezone) + dayDelta;
  const minutes = roundQuarter(eventMinutes(event.startMs, timezone) + minuteDelta);
  const startMs = atWall(day, minutes, timezone);
  return { endMs: startMs + (event.endMs - event.startMs), startMs };
}

/** The bottom edge dragged to `minutes` of `day`: the nearest quarter,
 *  fifteen minutes after the start at least. */
export function resizedEnd(event: Span, day: number, minutes: number, timezone: string): Span {
  const endMs = atWall(day, roundQuarter(clampDay(minutes)), timezone);
  return {
    endMs: Math.max(endMs, event.startMs + MIN_MINUTES * MINUTE_MS),
    startMs: event.startMs,
  };
}

/**
 * Whole days, first to last, as Orbit writes an all-day event: from the
 * first day's midnight to the last second of the last day. Calendar reads
 * an all-day end as the day it falls on, so the end never spills into the
 * next day whichever way the store counts it.
 */
export function allDaySpan(firstDay: number, lastDay: number, timezone: string): Span {
  const [first, last] = firstDay <= lastDay ? [firstDay, lastDay] : [lastDay, firstDay];
  return { endMs: startOfDay(last + 1, timezone) - 1_000, startMs: startOfDay(first, timezone) };
}

/** The civil days an all-day (or day-long) event covers. */
export function coveredDays(event: Span, timezone: string): { first: number; last: number } {
  const first = dayOf(event.startMs, timezone);
  return { first, last: Math.max(first, dayOf(event.endMs - 1, timezone)) };
}

/** A bar moved `delta` days, or one of its sides dragged to `day`. */
export function shiftedDays(
  event: Span,
  change: { kind: "move"; delta: number } | { kind: "start" | "end"; day: number },
  timezone: string,
): Span {
  const { first, last } = coveredDays(event, timezone);
  if (change.kind === "move")
    return allDaySpan(first + change.delta, last + change.delta, timezone);
  if (change.kind === "start") return allDaySpan(Math.min(change.day, last), last, timezone);
  return allDaySpan(first, Math.max(change.day, first), timezone);
}
