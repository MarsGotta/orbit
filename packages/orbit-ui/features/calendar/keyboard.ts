import { MAX_GRID_DAYS, type ViewChoice } from "./range";

/**
 * What a key does in the calendar. No ⌥ shortcuts: on a Spanish
 * keyboard ⌥ writes characters.
 *
 * `0` is fourteen days, so the zoom does not take it bare: `+`
 * and `-` (with or without ⌘) step the zoom and `⌘0` resets it. Any other
 * key held with ⌘, Ctrl or ⌥ belongs to someone else (the palette, the
 * system) and is left alone.
 */
export type CalendarKeyAction =
  | { kind: "view"; choice: ViewChoice }
  | { kind: "step"; direction: -1 | 1 }
  | { kind: "today" }
  | { kind: "zoom"; delta: number }
  | { kind: "zoomReset" };

export interface KeyPress {
  altKey: boolean;
  ctrlKey: boolean;
  key: string;
  metaKey: boolean;
}

const ZOOM_STEP = 0.1;

function zoomAction(key: string): CalendarKeyAction | null {
  if (key === "+" || key === "=") return { delta: ZOOM_STEP, kind: "zoom" };
  if (key === "-") return { delta: -ZOOM_STEP, kind: "zoom" };
  return null;
}

export function calendarKeyAction(press: KeyPress): CalendarKeyAction | null {
  if (press.altKey || press.ctrlKey) return null;
  if (press.metaKey) return press.key === "0" ? { kind: "zoomReset" } : zoomAction(press.key);
  const key = press.key.length === 1 ? press.key.toLowerCase() : press.key;
  if (/^[1-9]$/.test(key)) {
    const days = Number(key);
    return { choice: days === 1 ? { days: 1, view: "day" } : { days, view: "days" }, kind: "view" };
  }
  switch (key) {
    case "0":
      return { choice: { days: MAX_GRID_DAYS, view: "days" }, kind: "view" };
    case "d":
      return { choice: { days: 1, view: "day" }, kind: "view" };
    case "s":
      return { choice: { days: 7, view: "week" }, kind: "view" };
    case "m":
      return { choice: { days: 7, view: "month" }, kind: "view" };
    case "a":
      return { choice: { days: 7, view: "agenda" }, kind: "view" };
    case "ArrowLeft":
    case "k":
      return { direction: -1, kind: "step" };
    case "ArrowRight":
    case "j":
      return { direction: 1, kind: "step" };
    case "h":
      return { kind: "today" };
    default:
      return zoomAction(key);
  }
}

/** Inputs that take no text: a key there still commands the calendar. */
const NOT_TEXT = ["button", "checkbox", "color", "file", "radio", "range", "reset", "submit"];

/** Typing in a field is writing, not commanding the calendar. */
export function isTypingTarget(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  if (target.isContentEditable || ["SELECT", "TEXTAREA"].includes(target.tagName)) return true;
  return target instanceof HTMLInputElement && !NOT_TEXT.includes(target.type);
}
