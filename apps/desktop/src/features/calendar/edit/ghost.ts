import type { DaySegment } from "../DayColumn";
import { eventSegments } from "../date";
import { es } from "../../../i18n/es";
import type { CalendarEvent } from "../eventLook";
import type { GridPreview } from "./useGridGestures";

/**
 * The gesture in progress drawn as a card of its own on every day it
 * touches, over the cards that stay where they are (P9-22): the card being
 * dragged stays in its place, dimmed.
 */
export function ghostSegments(
  preview: GridPreview | null,
  events: readonly CalendarEvent[],
  days: number[],
  timezone: string,
): Map<number, DaySegment[]> {
  const byDay = new Map<number, DaySegment[]>();
  if (preview === null) return byDay;
  const source = events.find((event) => event.id === preview.eventId);
  const event: CalendarEvent = {
    ...(source ?? { id: "ghost", note: null, title: es.calendar.edit.newEvent }),
    end_ms: preview.span.endMs,
    id: `ghost:${preview.eventId ?? "new"}`,
    start_ms: preview.span.startMs,
    ui: { dragging: true },
  };
  for (const segment of eventSegments(event.start_ms, event.end_ms, days, timezone)) {
    byDay.set(segment.day, [{ ...segment, event }]);
  }
  return byDay;
}
