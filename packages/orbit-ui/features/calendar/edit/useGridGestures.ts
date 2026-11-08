import { type MouseEvent as ReactMouseEvent, useRef, useState } from "react";
import type { CalendarEvent } from "../eventLook";
import { clickSlot, dragSlot, minutesAtY, movedSpan, resizedEnd, type Span } from "./geometry";

import { trackDrag, type GestureEdit } from "./trackDrag";

type Mode = "create" | "move" | "resize";

export interface GridPreview {
  /** The card being moved or stretched; none while drawing a new one. */
  eventId: string | null;
  mode: Mode;
  span: Span;
}

export type GridEdit = GestureEdit;

interface Point {
  day: number;
  minutes: number;
}

/** The day and minute under the pointer, from the columns' own boxes. */
function locate(body: HTMLElement, x: number, y: number, hourPx: number, clamp: boolean) {
  const columns = [...body.querySelectorAll<HTMLElement>("[data-day]")];
  const boxes = columns.map((column) => ({ box: column.getBoundingClientRect(), column }));
  let hit = boxes.find(({ box }) => x >= box.left && x < box.right);
  if (!hit && clamp) hit = x < (boxes[0]?.box.left ?? 0) ? boxes[0] : boxes.at(-1);
  if (!hit) return null;
  const top = body.getBoundingClientRect().top;
  return { day: Number(hit.column.dataset.day), minutes: minutesAtY(y - top, hourPx) };
}

function spanFor(
  mode: Mode,
  from: Point,
  to: Point,
  event: CalendarEvent | undefined,
  zone: string,
) {
  if (mode === "create" || event === undefined)
    return dragSlot(from.day, from.minutes, to.minutes, zone);
  const original = { endMs: event.end_ms, startMs: event.start_ms };
  if (mode === "resize") return resizedEnd(original, from.day, to.minutes, zone);
  return movedSpan(original, to.day - from.day, to.minutes - from.minutes, zone);
}

interface Latest {
  edit: GridEdit | undefined;
  events: readonly CalendarEvent[];
  hourPx: number;
  timezone: string;
}

type SetPreview = (update: (now: GridPreview | null) => GridPreview | null) => void;

/** A press in the grid, followed until the button is released. */
function begin(
  down: ReactMouseEvent<HTMLDivElement>,
  body: HTMLElement,
  latest: { current: Latest },
  setPreview: SetPreview,
) {
  const options = latest.current;
  if (down.button !== 0 || options.edit === undefined) return;
  const target = down.target as HTMLElement;
  const card = target.closest<HTMLElement>("[data-event-id]");
  const event = options.events.find((item) => item.id === card?.dataset.eventId);
  if ((card !== null && event === undefined) || event?.ui?.saving === true) return;
  const from = locate(body, down.clientX, down.clientY, options.hourPx, false);
  if (from === null) return;
  const mode: Mode = event ? (target.closest("[data-resize]") ? "resize" : "move") : "create";
  if (mode === "create") down.preventDefault();
  const spanAt = (pointer: MouseEvent) => {
    const to = locate(body, pointer.clientX, pointer.clientY, latest.current.hourPx, true) ?? from;
    return spanFor(mode, from, to, event, latest.current.timezone);
  };
  trackDrag({
    down,
    event,
    edit: () => latest.current.edit,
    horizontal: false,
    spanAt,
    preview: (span) =>
      setPreview((now) => {
        if (span === null) return null;
        return now?.span.startMs === span.startMs && now.span.endMs === span.endMs
          ? now
          : { eventId: event?.id ?? null, mode, span };
      }),
    onClick: (edit) => {
      if (mode !== "create" || edit.onDeselect()) return;
      const { events, timezone } = latest.current;
      edit.onCreate(
        clickSlot(
          from.day,
          from.minutes,
          timezone,
          events.map((item) => item.start_ms),
        ),
      );
    },
  });
}

/**
 * Click and drag in the time grid: a click in free time drafts an
 * event, a drag draws one, a drag of a card moves it (to another day too) and
 * a drag of its bottom edge stretches it. A card the user cannot change does not
 * move: the gesture ends in a notice with the cause.
 */
export function useGridGestures(
  edit: GridEdit | undefined,
  events: readonly CalendarEvent[],
  hourPx: number,
  timezone: string,
) {
  const [preview, setPreview] = useState<GridPreview | null>(null);
  const bodyRef = useRef<HTMLDivElement>(null);
  const latest = useRef<Latest>({ edit, events, hourPx, timezone });
  latest.current = { edit, events, hourPx, timezone };
  const onMouseDown = (down: ReactMouseEvent<HTMLDivElement>) => {
    if (bodyRef.current !== null) begin(down, bodyRef.current, latest, setPreview);
  };
  return { bodyRef, onMouseDown, preview };
}
