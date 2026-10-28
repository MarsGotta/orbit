import { getCopy } from "../../i18n";
import type { AppView as AppSnapshot } from "../../state/app-view";
import type { CSSProperties } from "react";

import { DAY_MS, dayOf } from "./date";

/** What the view adds to an event it draws: a write on its way,
 *  a draft not yet in the calendar, a card being dragged. */
export interface EventUi {
  draft?: boolean;
  dragging?: boolean;
  saving?: boolean;
}

export type CalendarEvent = AppSnapshot["calendar"]["events"][number] & { ui?: EventUi };

/**
 * How a card reads at a glance besides its answer (`responseLabel`):
 * declined, over, or being recorded. Pure: the event and the instant in.
 */
export interface EventLook {
  /** Declined: struck through, at half strength. */
  declined: boolean;
  /** Over: drawn at half strength. */
  past: boolean;
  recording: boolean;
}

export function eventLook(event: CalendarEvent, nowMs: number): EventLook {
  return {
    declined: event.self_response === "declined",
    past: event.end_ms <= nowMs,
    recording: event.recording === "recording",
  };
}

/** All-day events, and timed ones a day long or longer, go in the top lane. */
export function inAllDayLane(event: CalendarEvent): boolean {
  return event.all_day === true || event.end_ms - event.start_ms >= DAY_MS;
}

/** The place, or the link without `https://`, for the card's last line. */
export function placePreview(event: CalendarEvent): string | null {
  if (event.location != null && event.location !== "") return event.location;
  const link = event.conference_url ?? event.url;
  if (link == null || link === "") return null;
  return link.replace(/^https?:\/\//i, "").replace(/\/$/, "");
}

export interface EventFilter {
  hiddenCalendars: ReadonlySet<string>;
  showDeclined: boolean;
}

export function visibleEvents(events: readonly CalendarEvent[], filter: EventFilter) {
  return events.filter(
    (event) =>
      (event.calendar_id == null || !filter.hiddenCalendars.has(event.calendar_id)) &&
      (filter.showDeclined || event.self_response !== "declined"),
  );
}

/** One of the four calendar colours of the design (`--o-c1`…`--o-c4`). */
export type CalendarTone = 1 | 2 | 3 | 4;

const TONE_OF_TOKEN: Record<string, CalendarTone> = { accent: 1, blue: 2, green: 3, orange: 4 };

/**
 * The design has four calendar colours. A calendar arrives with a palette
 * token (`accent`, `blue`, `green`, `orange`) or with its own macOS colour
 * (`#RRGGBB`), which takes the tone nearest in hue: violets and pinks the
 * first, blues the second, greens the third, reds, oranges and yellows the
 * fourth. A grey, or anything unreadable, is the first.
 */
export function calendarTone(color: string | undefined): CalendarTone {
  if (color === undefined) return 1;
  const token = TONE_OF_TOKEN[color];
  if (token !== undefined) return token;
  const match = /^#([0-9a-f]{6})$/i.exec(color);
  if (match === null) return 1;
  const value = Number.parseInt(match[1] ?? "0", 16);
  const [red, green, blue] = [(value >> 16) & 255, (value >> 8) & 255, value & 255].map(
    (channel) => channel / 255,
  ) as [number, number, number];
  const max = Math.max(red, green, blue);
  const min = Math.min(red, green, blue);
  if (max - min < 0.12) return 1;
  const delta = max - min;
  const sector =
    max === red
      ? ((green - blue) / delta + 6) % 6
      : max === green
        ? (blue - red) / delta + 2
        : (red - green) / delta + 4;
  const hue = sector * 60;
  if (hue >= 75 && hue < 180) return 3;
  if (hue >= 180 && hue < 235) return 2;
  if (hue >= 235 && hue < 335) return 1;
  return 4;
}

/** The tone of the calendar `event` belongs to. */
export function toneOf(
  event: CalendarEvent,
  calendars: AppSnapshot["calendar"]["calendars"],
): CalendarTone {
  return calendarTone(calendars.find((item) => item.id === event.calendar_id)?.color);
}

/**
 * The events of each day in `days`, for the month and the agenda: an all-day
 * or multi-day event on every day it covers, a timed one on the day it
 * starts; all-day first, then by start.
 */
export function eventsByDay(
  events: readonly CalendarEvent[],
  days: readonly number[],
  timezone: string,
): Map<number, CalendarEvent[]> {
  const byDay = new Map<number, CalendarEvent[]>(days.map((day) => [day, []]));
  for (const event of events) {
    const first = dayOf(event.start_ms, timezone);
    const last = inAllDayLane(event) ? Math.max(first, dayOf(event.end_ms - 1, timezone)) : first;
    for (const day of days) if (day >= first && day <= last) byDay.get(day)?.push(event);
  }
  for (const list of byDay.values()) {
    list.sort(
      (left, right) =>
        Number(inAllDayLane(right)) - Number(inAllDayLane(left)) || left.start_ms - right.start_ms,
    );
  }
  return byDay;
}

/** The two colours a card, a bar, a dot or a row of `tone` reads. */
export function toneStyle(tone: CalendarTone): CSSProperties {
  return {
    "--event-color": `var(--o-c${tone})`,
    "--event-tint": `var(--o-c${tone}s)`,
  } as CSSProperties;
}

/** «Quizá» or «Sin responder», the label under a card; none otherwise. */
export function responseLabel(
  event: CalendarEvent,
): { kind: "tentative" | "unanswered"; label: string } | undefined {
  if (event.self_response === "tentative")
    return { kind: "tentative", label: getCopy().calendar.selfResponse.tentative };
  if (event.self_response === "needs_action")
    return { kind: "unanswered", label: getCopy().calendar.selfResponse.needs_action };
  return undefined;
}
