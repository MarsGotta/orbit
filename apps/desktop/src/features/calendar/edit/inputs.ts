/**
 * The panel's date and time fields (P9-22): instants shown and read as the
 * wall clock of the zone on screen, whatever zone the webview runs in.
 */
import { civil, DAY_MS, dayOf, eventMinutes } from "../date";

const pad = (value: number) => String(value).padStart(2, "0");

/** «2026-09-16», the value of an `<input type="date">`. */
export function dateValue(instant: number, timezone: string): string {
  const { date, month, year } = civil(dayOf(instant, timezone));
  return `${year}-${pad(month)}-${pad(date)}`;
}

/** «09:30», the value of an `<input type="time">`. */
export function timeValue(instant: number, timezone: string): string {
  const minutes = eventMinutes(instant, timezone);
  return `${pad(Math.floor(minutes / 60))}:${pad(minutes % 60)}`;
}

/** The civil day of a date field, or `null` while it is not a whole date. */
export function dayOfValue(value: string): number | null {
  const match = /^(\d{4})-(\d{2})-(\d{2})$/.exec(value);
  if (!match) return null;
  const day = Date.UTC(Number(match[1]), Number(match[2]) - 1, Number(match[3])) / DAY_MS;
  return Number.isSafeInteger(day) ? day : null;
}

/** The minute of the day of a time field, or `null` while it is incomplete. */
export function minutesOfValue(value: string): number | null {
  const match = /^(\d{2}):(\d{2})/.exec(value);
  if (!match) return null;
  const minutes = Number(match[1]) * 60 + Number(match[2]);
  return minutes < 24 * 60 ? minutes : null;
}

/** A time as typed in a 24-hour field: «9», «9:30», «09.30», «930» or
 *  «0930». `null` when it is not a time of the day. */
export function parseTime(value: string): number | null {
  const match = /^(\d{1,2})(?:[:.h]?(\d{2}))?$/.exec(value.trim());
  if (!match) return null;
  const hours = Number(match[1]);
  const minutes = Number(match[2] ?? "0");
  return hours < 24 && minutes < 60 ? hours * 60 + minutes : null;
}
