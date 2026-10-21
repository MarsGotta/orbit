import type { AppView as AppSnapshot } from "../../state/app-view";
import { DAY_MS, dayOf } from "./date";

/** What the view adds to an event it draws (P9-22): a write on its way,
 *  a draft not yet in the calendar, a card being dragged. */
export interface EventUi {
  draft?: boolean;
  dragging?: boolean;
  saving?: boolean;
}

export type CalendarEvent = AppSnapshot["calendar"]["events"][number] & { ui?: EventUi };

/**
 * How a card reads at a glance (P9-20): Mars's answer, a
 * meeting that is over, the one icon on the right and a recording in
 * progress. Pure: the event, the instant and the meeting in, a look out.
 */
export interface EventLook {
  declined: boolean;
  /** Over, with guests: drawn as done. Her own events without guests are not. */
  done: boolean;
  icon: "unanswered" | null;
  recording: boolean;
  /** Maybe or not answered yet: a striped card. */
  striped: boolean;
}

export function eventLook(event: CalendarEvent, nowMs: number) {
  const response = event.self_response ?? null;
  const declined = response === "declined";
  const unanswered = response === "needs_action";
  const hasGuests = (event.attendees?.length ?? 0) > 0;
  return {
    declined,
    done: !declined && hasGuests && event.end_ms <= nowMs,
    icon: unanswered ? "unanswered" : null,
    recording: event.recording === "recording",
    striped: unanswered || response === "tentative",
  } satisfies EventLook;
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

export function paletteOf(
  event: CalendarEvent,
  calendars: AppSnapshot["calendar"]["calendars"],
): string {
  return calendars.find((item) => item.id === event.calendar_id)?.color ?? "accent";
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
