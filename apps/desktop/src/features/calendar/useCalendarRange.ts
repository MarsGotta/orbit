import { useCallback, useEffect, useRef, useState } from "react";
import type { AppCommand } from "../../bridge/generated";
import type { CalendarWeekStart } from "../../bridge/generated/CalendarWeekStart";
import {
  clampZoom,
  type CalendarPreferences,
  readPreferences,
  writePreferences,
} from "./preferences";
import {
  defaultChoice,
  type Range,
  setRange,
  showsChoice,
  stepAnchor,
  type ViewChoice,
  widthTier,
} from "./range";

type Send = (command: AppCommand) => Promise<void>;

function choiceKey(choice: ViewChoice, weekStart: CalendarWeekStart): string {
  return `${choice.view}:${choice.days}:${weekStart}`;
}

/** The calendar area's size, from the first measurement on (0 before it). */
function useElementSize(element: HTMLElement | null) {
  const [size, setSize] = useState({ height: 0, width: 0 });
  useEffect(() => {
    if (element === null || typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver((entries) => {
      const rect = entries[0]?.contentRect;
      if (rect) setSize({ height: rect.height, width: rect.width });
    });
    observer.observe(element);
    return () => observer.disconnect();
  }, [element]);
  return size;
}

/**
 * Which period is on screen and how to change it (P9-20). app-core decides
 * the range; this asks for it: the view Mars chose for this width (or the
 * width's default), the day her week begins, and steps through periods.
 * A request is sent once per choice, so a range that has not arrived yet is
 * not asked for again on every render.
 */
export function useCalendarRange(range: Range, send: Send, timezone: string) {
  const [preferences, setPreferences] = useState(readPreferences);
  const [element, attach] = useState<HTMLElement | null>(null);
  const size = useElementSize(element);
  const tier = widthTier(size.width);
  const wanted = preferences.byTier[tier] ?? defaultChoice(tier);
  const wantedKey = choiceKey(wanted, preferences.weekStart);
  // The choice last asked for, so a range that has not arrived yet is not
  // asked for again, and a day opened from the month is not undone.
  const asked = useRef<string | null>(null);
  const anchor = range.anchor_ms > 0 ? range.anchor_ms : Date.now();
  const update = useCallback((next: Partial<CalendarPreferences>) => {
    setPreferences((current) => {
      const merged = { ...current, ...next };
      writePreferences(merged);
      return merged;
    });
  }, []);
  useEffect(() => {
    if (asked.current === wantedKey) return;
    asked.current = wantedKey;
    if (!showsChoice(range, wanted, preferences.weekStart))
      void send(setRange(wanted, anchor, preferences.weekStart));
    // Only a new width or a new preference asks again; see `asked`.
  }, [wantedKey]);
  const onScreen = { days: range.days, view: range.view };
  return {
    attach,
    /** Shows `choice` around the day on screen and keeps it for this width. */
    choose: (choice: ViewChoice) => {
      asked.current = choiceKey(choice, preferences.weekStart);
      update({ byTier: { ...preferences.byTier, [tier]: choice } });
      void send(setRange(choice, anchor, preferences.weekStart));
    },
    /** One day, from the month or the agenda; not kept as her choice. */
    openDay: (dayStartMs: number) =>
      void send(setRange({ days: 1, view: "day" }, dayStartMs, preferences.weekStart)),
    preferences,
    size,
    step: (direction: -1 | 1) =>
      void send(setRange(onScreen, stepAnchor(range, direction, timezone), range.week_start)),
    today: () => void send(setRange(onScreen, Date.now(), range.week_start)),
    update,
    zoomBy: (delta: number) => update({ zoom: clampZoom(preferences.zoom + delta) }),
  };
}

export type CalendarRangeControls = ReturnType<typeof useCalendarRange>;
