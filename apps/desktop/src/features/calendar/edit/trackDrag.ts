import type { MouseEvent as ReactMouseEvent } from "react";
import type { CalendarEvent } from "../eventLook";
import type { Span } from "./geometry";

export interface GestureEdit {
  editable: (event: CalendarEvent) => boolean;
  onBlocked: (event: CalendarEvent) => void;
  onChange: (event: CalendarEvent, span: Span) => void;
  onCreate: (span: Span) => void;
  /** Whether a click in free space deselected an event. */
  onDeselect: () => boolean;
}

/** The click a drag ends with is not a click on what it ended over. */
function swallowNextClick(): void {
  const stop = (event: Event) => {
    event.stopPropagation();
    event.preventDefault();
  };
  window.addEventListener("click", stop, { capture: true, once: true });
  window.setTimeout(() => window.removeEventListener("click", stop, { capture: true }), 0);
}

/** Follow a calendar press until release, then create, move or resize once. */
export function trackDrag({
  down,
  event,
  edit,
  horizontal,
  onClick,
  preview,
  spanAt,
}: {
  down: ReactMouseEvent<HTMLElement>;
  event: CalendarEvent | undefined;
  edit: () => GestureEdit | undefined;
  horizontal: boolean;
  onClick: (edit: GestureEdit, span: Span) => void;
  preview: (span: Span | null) => void;
  spanAt: (pointer: MouseEvent) => Span;
}) {
  // Below four pixels the press is a click. The all-day lane measures only x.
  const thresholdPx = 4;
  let moved = false;
  const move = (pointer: MouseEvent) => {
    const dx = pointer.clientX - down.clientX;
    const dy = pointer.clientY - down.clientY;
    const travel = horizontal ? Math.abs(dx) : Math.hypot(dx, dy);
    if (!moved && travel < thresholdPx) return;
    moved = true;
    if (event !== undefined && edit()?.editable(event) !== true) return;
    preview(spanAt(pointer));
  };
  const up = (pointer: MouseEvent) => {
    window.removeEventListener("mousemove", move);
    window.removeEventListener("mouseup", up);
    preview(null);
    const options = edit();
    if (options === undefined) return;
    const span = spanAt(pointer);
    if (!moved) return onClick(options, span);
    swallowNextClick();
    if (event === undefined) return options.onCreate(span);
    if (!options.editable(event)) return options.onBlocked(event);
    if (span.startMs !== event.start_ms || span.endMs !== event.end_ms)
      options.onChange(event, span);
  };
  window.addEventListener("mousemove", move);
  window.addEventListener("mouseup", up);
}
