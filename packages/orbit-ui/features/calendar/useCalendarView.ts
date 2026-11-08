import type { AppView as AppSnapshot } from "../../state/app-view";
import { useCallback, useEffect, useState } from "react";
import type { AppCommand } from "../../bridge/generated";
import { useNow } from "../../use-now";
import { useUiStore } from "../../state/ui";
import { effectiveTimezone } from "./date";

export type CalendarEvent = AppSnapshot["calendar"]["events"][number];
type Calendar = AppSnapshot["calendar"];
type Send = (command: AppCommand) => Promise<unknown>;

export function useCalendarView(calendar: Calendar, systemTimezone: string, send: Send) {
  const selectedCalendarEventId = useUiStore((state) => state.selectedCalendarEventId);
  const clearCalendarEvent = useUiStore((state) => state.clearCalendarEvent);
  const nowMs = useNow(60_000);
  const [snapshotPending, setSnapshotPending] = useState(false);
  const [selectedEvent, setSelectedEvent] = useState<CalendarEvent | null>(null);
  useEffect(() => {
    if (selectedCalendarEventId !== null) {
      setSelectedEvent(
        calendar.events.find((event) => event.id === selectedCalendarEventId) ?? null,
      );
    }
  }, [calendar.events, selectedCalendarEventId]);
  const timezone = effectiveTimezone(systemTimezone);
  const refresh = useCallback(
    async (command: AppCommand) => {
      setSnapshotPending(true);
      try {
        await send(command);
      } finally {
        setSnapshotPending(false);
      }
    },
    [send],
  );
  const startMeeting = (event?: CalendarEvent) => {
    const current =
      event ?? calendar.events.find((item) => item.start_ms <= nowMs && item.end_ms > nowMs);
    const reference =
      current?.calendar_item_id != null && current.occurrence_start_ms != null
        ? {
            calendar_item_id: current.calendar_item_id,
            occurrence_start_ms: current.occurrence_start_ms,
          }
        : undefined;
    void send({
      command: "meeting_start",
      ...(current ? { event_id: current.id } : {}),
      ...(reference ? { event: reference } : {}),
    });
    setSelectedEvent(null);
  };
  return {
    nowMs,
    refresh,
    selectedEvent,
    setSelectedEvent: (event: CalendarEvent | null) => {
      setSelectedEvent(event);
      if (event === null) clearCalendarEvent();
    },
    snapshotPending,
    startMeeting,
    timezone,
  };
}
