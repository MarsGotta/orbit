import type { AppView as AppSnapshot } from "../../state/app-view";
import { useEffect, useMemo, useState } from "react";

type CalendarItem = AppSnapshot["calendar"]["calendars"][number];

const storageKey = "orbit.calendar.hidden.local";

function readHiddenIds(): string[] {
  try {
    const value: unknown = JSON.parse(window.localStorage.getItem(storageKey) ?? "[]");
    return Array.isArray(value) ? value.filter((id): id is string => typeof id === "string") : [];
  } catch {
    return [];
  }
}

export function useCalendarVisibility(calendars: CalendarItem[]) {
  const [hiddenIds, setHiddenIds] = useState(readHiddenIds);
  const knownIds = useMemo(() => new Set(calendars.map((item) => item.id)), [calendars]);
  const visibleHiddenIds = useMemo(
    () => hiddenIds.filter((id) => knownIds.has(id)),
    [hiddenIds, knownIds],
  );
  const hidden = useMemo(() => new Set(hiddenIds), [hiddenIds]);

  useEffect(() => {
    // An empty catalog can be an unobserved reading, not a removal.
    if (calendars.length > 0 && visibleHiddenIds.length !== hiddenIds.length) {
      setHiddenIds(visibleHiddenIds);
    }
  }, [calendars.length, hiddenIds, visibleHiddenIds]);

  useEffect(() => {
    if (calendars.length === 0) return;
    try {
      window.localStorage.setItem(storageKey, JSON.stringify(visibleHiddenIds));
    } catch {
      // Keep the current session's choice when storage is unavailable.
    }
  }, [calendars.length, visibleHiddenIds]);

  const toggle = (id: string) => {
    setHiddenIds((current) =>
      current.includes(id) ? current.filter((item) => item !== id) : [...current, id],
    );
  };
  const showAll = () => setHiddenIds([]);
  return { hidden, showAll, toggle };
}
