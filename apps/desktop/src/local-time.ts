import { dateTime } from "./i18n/locale";
export function effectiveTimezone(publishedTimezone: string): string | null {
  if (!publishedTimezone) return null;
  try {
    dateTime({ timeZone: publishedTimezone }).format(0);
    return publishedTimezone;
  } catch {
    return null;
  }
}

const DAY_MS = 86_400_000;

export function localDayIndex(atMs: number, timezone: string): number {
  const parts = new Intl.DateTimeFormat("en-US", {
    day: "numeric",
    month: "numeric",
    year: "numeric",
    timeZone: timezone,
  }).formatToParts(atMs);
  const values = Object.fromEntries(parts.map((part) => [part.type, Number(part.value)]));
  return Date.UTC(values.year ?? 0, (values.month ?? 1) - 1, values.day ?? 1) / DAY_MS;
}

export function isoWeekForDay(day: number): number {
  const date = new Date(day * DAY_MS);
  const weekday = (date.getUTCDay() + 6) % 7;
  const thursday = new Date(
    Date.UTC(date.getUTCFullYear(), date.getUTCMonth(), date.getUTCDate() + 3 - weekday),
  );
  const firstThursday = new Date(Date.UTC(thursday.getUTCFullYear(), 0, 4));
  const offset = (firstThursday.getUTCDay() + 6) % 7;
  const weekOne = firstThursday.getTime() - offset * DAY_MS;
  return Math.floor((thursday.getTime() - weekOne) / (7 * DAY_MS)) + 1;
}
