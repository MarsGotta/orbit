import type { AppCommand } from "../../bridge/generated";
import type { CalendarRangeSnapshot } from "../../bridge/generated/CalendarRangeSnapshot";
import type { CalendarViewKind } from "../../bridge/generated/CalendarViewKind";
import type { CalendarWeekStart } from "../../bridge/generated/CalendarWeekStart";
import { civil, dayOf, startOfDay } from "./date";

export type Range = CalendarRangeSnapshot;

/**
 * The widest grid of N days: the `0` key. It protects only the
 * layout: past fourteen columns a column is narrower than an event title. It
 * is not a read limit; the month and the agenda are wider. app-core refuses a
 * wider grid with the same number (`MAX_GRID_DAYS` in `calendar.rs`);
 * `contracts/constants.json` and `test_constants_copies.py` keep both equal.
 */
export const MAX_GRID_DAYS = 14;

/** What the user picked: a view and, for the N-day grid, how many days. */
export interface ViewChoice {
  days: number;
  view: CalendarViewKind;
}

/**
 * How wide the calendar is: narrow shows one day,
 * medium three, wide the week. The widths are the calendar's own area, not
 * the window's. An unmeasured area (0) is wide, the default view, so nothing
 * is asked for before the first measurement.
 */
export type WidthTier = "narrow" | "medium" | "wide";

const NARROW_BELOW_PX = 560;
const MEDIUM_BELOW_PX = 900;

export function widthTier(width: number): WidthTier {
  if (width <= 0) return "wide";
  if (width < NARROW_BELOW_PX) return "narrow";
  return width < MEDIUM_BELOW_PX ? "medium" : "wide";
}

export function defaultChoice(tier: WidthTier): ViewChoice {
  switch (tier) {
    case "narrow":
      return { days: 1, view: "day" };
    case "medium":
      return { days: 3, view: "days" };
    case "wide":
      return { days: 7, view: "week" };
  }
}

/** The command that shows `choice` around `anchorMs`. */
export function setRange(
  choice: ViewChoice,
  anchorMs: number,
  weekStart: CalendarWeekStart,
): AppCommand {
  return {
    anchor_ms: anchorMs,
    command: "calendar_set_range",
    days: choice.view === "days" ? choice.days : choice.view === "day" ? 1 : 7,
    view: choice.view,
    week_start: weekStart,
  };
}

/** Whether the range on screen already is `choice` from `weekStart`. */
export function showsChoice(range: Range, choice: ViewChoice, weekStart: CalendarWeekStart) {
  if (range.view !== choice.view || range.week_start !== weekStart) return false;
  return choice.view !== "days" || range.days === choice.days;
}

/** Any instant of the period before (`-1`) or after (`1`) the one on screen. */
export function stepAnchor(range: Range, direction: -1 | 1, timezone: string): number {
  const anchorDay = dayOf(range.anchor_ms, timezone);
  const firstDay = dayOf(range.start_ms, timezone);
  switch (range.view) {
    case "day":
    case "days":
    case "week":
      return startOfDay(firstDay + direction * range.days, timezone);
    case "agenda":
      return startOfDay(anchorDay + direction * 7, timezone);
    case "month": {
      const firstOfMonth = anchorDay - civil(anchorDay).date + 1;
      // The day before the 1st is in the month before; 31 days after the 1st
      // can never be past the month after.
      const target = direction < 0 ? firstOfMonth - 1 : firstOfMonth + 31;
      return startOfDay(target - civil(target).date + 1, timezone);
    }
  }
}

export function includes(range: Range, instantMs: number): boolean {
  return instantMs >= range.start_ms && instantMs < range.end_ms;
}
