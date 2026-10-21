import { type MouseEvent as ReactMouseEvent, useRef, useState } from "react";
import type { CalendarEvent } from "../eventLook";
import { allDaySpan, movedSpan, shiftedDays, type Span } from "./geometry";

import { trackDrag, type GestureEdit } from "./trackDrag";

type Mode = "create" | "move" | "start" | "end";

export type LaneEdit = GestureEdit;

export interface LanePreview {
  eventId: string | null;
  span: Span;
}

interface Latest {
  days: readonly number[];
  edit: LaneEdit | undefined;
  events: readonly CalendarEvent[];
  timezone: string;
}

/** The column under `x`: the lane's columns share its width equally. */
function dayAt(lane: HTMLElement, x: number, days: readonly number[]): number | null {
  const box = lane.getBoundingClientRect();
  if (days.length === 0 || box.width <= 0) return days[0] ?? null;
  const index = Math.floor(((x - box.left) / box.width) * days.length);
  return days[Math.min(days.length - 1, Math.max(0, index))] ?? null;
}

function spanFor(
  mode: Mode,
  from: number,
  to: number,
  event: CalendarEvent | undefined,
  zone: string,
) {
  if (mode === "create" || event === undefined) return allDaySpan(from, to, zone);
  const span = { endMs: event.end_ms, startMs: event.start_ms };
  if (mode !== "move") return shiftedDays(span, { day: to, kind: mode }, zone);
  if (event.all_day === true) return shiftedDays(span, { delta: to - from, kind: "move" }, zone);
  return movedSpan(span, to - from, 0, zone);
}

function begin(
  down: ReactMouseEvent<HTMLElement>,
  lane: HTMLElement,
  latest: { current: Latest },
  setPreview: (preview: LanePreview | null) => void,
) {
  const options = latest.current;
  if (down.button !== 0 || options.edit === undefined) return;
  const target = down.target as HTMLElement;
  const bar = target.closest<HTMLElement>("[data-event-id]");
  const event = options.events.find((item) => item.id === bar?.dataset.eventId);
  if ((bar !== null && event === undefined) || event?.ui?.saving === true) return;
  const from = dayAt(lane, down.clientX, options.days);
  if (from === null) return;
  const side = target.closest<HTMLElement>("[data-resize]")?.dataset.resize;
  const mode: Mode = event ? (side === "start" || side === "end" ? side : "move") : "create";
  down.preventDefault();
  const spanAt = (pointer: MouseEvent) =>
    spanFor(
      mode,
      from,
      dayAt(lane, pointer.clientX, latest.current.days) ?? from,
      event,
      latest.current.timezone,
    );
  trackDrag({
    down,
    event,
    edit: () => latest.current.edit,
    horizontal: true,
    spanAt,
    preview: (span) => setPreview(span === null ? null : { eventId: event?.id ?? null, span }),
    onClick: (edit, span) => {
      if (mode === "create" && !edit.onDeselect()) edit.onCreate(span);
    },
  });
}

/**
 * Drag in the all-day lane (P9-22): across free days it drafts an all-day
 * event by days; a bar moves by days and stretches from either side.
 */
export function useLaneGestures(
  edit: LaneEdit | undefined,
  events: readonly CalendarEvent[],
  days: readonly number[],
  timezone: string,
) {
  const [preview, setPreview] = useState<LanePreview | null>(null);
  const laneRef = useRef<HTMLDivElement>(null);
  const latest = useRef<Latest>({ days, edit, events, timezone });
  latest.current = { days, edit, events, timezone };
  const onMouseDown = (down: ReactMouseEvent<HTMLDivElement>) => {
    if (laneRef.current !== null) begin(down, laneRef.current, latest, setPreview);
  };
  return { laneRef, onMouseDown, preview };
}

/** The lane's bars with the gesture in progress drawn in its place. */
export function withLanePreview(
  events: readonly CalendarEvent[],
  preview: LanePreview | null,
  newTitle: string,
): CalendarEvent[] {
  if (preview === null) return [...events];
  const { endMs, startMs } = preview.span;
  if (preview.eventId === null) {
    const ghost = { all_day: true, end_ms: endMs, id: "ghost:new", note: null, start_ms: startMs };
    return [...events, { ...ghost, title: newTitle, ui: { draft: true } }];
  }
  return events.map((event) =>
    event.id === preview.eventId
      ? { ...event, end_ms: endMs, start_ms: startMs, ui: { dragging: true } }
      : event,
  );
}
