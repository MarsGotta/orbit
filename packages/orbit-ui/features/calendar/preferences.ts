import type { CalendarViewKind } from "../../bridge/generated/CalendarViewKind";
import type { CalendarWeekStart } from "../../bridge/generated/CalendarWeekStart";
import { MAX_GRID_DAYS, type ViewChoice, type WidthTier } from "./range";

/**
 * The user's local calendar preferences: the view they chose for each
 * width, the day the week begins, weekends, declined events, zoom and the
 * all-day lane. Kept in this browser's storage only; without it everything
 * works with the defaults for the session.
 */
export interface CalendarPreferences {
  allDayOpen: boolean;
  byTier: Partial<Record<WidthTier, ViewChoice>>;
  showDeclined: boolean;
  showWeekends: boolean;
  weekStart: CalendarWeekStart;
  zoom: number;
}

const STORAGE_KEY = "orbit.calendar.view.local";
export const MIN_ZOOM = 0.5;
export const MAX_ZOOM = 1.5;

export const DEFAULT_PREFERENCES: CalendarPreferences = {
  allDayOpen: false,
  byTier: {},
  showDeclined: true,
  showWeekends: true,
  weekStart: "monday",
  zoom: 1,
};

const VIEWS: readonly CalendarViewKind[] = ["day", "days", "week", "month", "agenda"];
const TIERS: readonly WidthTier[] = ["narrow", "medium", "wide"];

export function clampZoom(zoom: number): number {
  return Math.round(Math.min(MAX_ZOOM, Math.max(MIN_ZOOM, zoom)) * 10) / 10;
}

function choiceFrom(value: unknown): ViewChoice | null {
  if (typeof value !== "object" || value === null) return null;
  const { days, view } = value as Record<string, unknown>;
  if (typeof view !== "string" || !VIEWS.includes(view as CalendarViewKind)) return null;
  if (typeof days !== "number" || !Number.isInteger(days) || days < 1 || days > MAX_GRID_DAYS)
    return null;
  return { days, view: view as CalendarViewKind };
}

function parse(raw: string | null): CalendarPreferences {
  const value: unknown = JSON.parse(raw ?? "{}");
  if (typeof value !== "object" || value === null) return DEFAULT_PREFERENCES;
  const stored = value as Record<string, unknown>;
  const byTier: Partial<Record<WidthTier, ViewChoice>> = {};
  const tiers = (stored.byTier ?? {}) as Record<string, unknown>;
  for (const tier of TIERS) {
    const choice = choiceFrom(tiers[tier]);
    if (choice) byTier[tier] = choice;
  }
  const flag = (key: keyof CalendarPreferences, fallback: boolean) =>
    typeof stored[key] === "boolean" ? (stored[key] as boolean) : fallback;
  return {
    allDayOpen: flag("allDayOpen", DEFAULT_PREFERENCES.allDayOpen),
    byTier,
    showDeclined: flag("showDeclined", DEFAULT_PREFERENCES.showDeclined),
    showWeekends: flag("showWeekends", DEFAULT_PREFERENCES.showWeekends),
    weekStart: stored.weekStart === "sunday" ? "sunday" : "monday",
    zoom: typeof stored.zoom === "number" ? clampZoom(stored.zoom) : 1,
  };
}

export function readPreferences(): CalendarPreferences {
  try {
    return parse(window.localStorage.getItem(STORAGE_KEY));
  } catch {
    return DEFAULT_PREFERENCES;
  }
}

export function writePreferences(preferences: CalendarPreferences): void {
  try {
    window.localStorage.setItem(STORAGE_KEY, JSON.stringify(preferences));
  } catch {
    // Keep the session's choice when storage is unavailable.
  }
}
