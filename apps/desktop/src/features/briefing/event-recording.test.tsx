import { act, cleanup, fireEvent, render, renderHook, screen } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import type { AppCommand } from "../../bridge/generated";
import type { BriefingNextSnapshot } from "../../bridge/generated/BriefingNextSnapshot";
import type { CalendarSnapshot } from "../../bridge/generated/CalendarSnapshot";
import { eventLook, type CalendarEvent } from "../calendar/eventLook";
import { useCalendarView } from "../calendar/useCalendarView";
import { NextMeetingCard } from "./NextMeetingCard";

import { TEST_NOW_MS } from "../../test-clock";
const now = TEST_NOW_MS;
const event: CalendarEvent = {
  id: "event-1",
  calendar_item_id: "committee",
  occurrence_start_ms: now - 20 * 60_000,
  title: "Comité",
  start_ms: now - 20 * 60_000,
  end_ms: now + 40 * 60_000,
  url: null,
};
const meeting = {
  event_id: event.id,
  event: {
    calendar_item_id: event.calendar_item_id,
    occurrence_start_ms: event.occurrence_start_ms,
  },
  title: event.title,
  start_ms: event.start_ms,
  end_ms: event.end_ms,
  attendees: [],
  url: null,
  series_key: "comité",
  previous: null,
  recording: "none",
} as BriefingNextSnapshot;

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

it("Hoy and Calendario send the same order for an event that began twenty minutes ago", () => {
  const fromToday = vi.fn<(command: AppCommand) => void>();
  const fromCalendar = vi.fn<(command: AppCommand) => Promise<void>>().mockResolvedValue();
  const calendar = { events: [event] } as CalendarSnapshot;
  const hook = renderHook(() => useCalendarView(calendar, "Europe/Madrid", fromCalendar));
  render(
    <NextMeetingCard
      meeting={meeting}
      nowMs={now}
      online
      send={fromToday}
      timezone="Europe/Madrid"
    />,
  );
  // No video call to join: the card only records («Solo grabar»).
  fireEvent.click(screen.getByRole("button", { name: /Solo grabar/ }));
  act(() => hook.result.current.startMeeting(event));
  // The card also asks the hub for its reminder; the recording order is the other one.
  const todayOrder = fromToday.mock.calls
    .map(([command]) => command)
    .find((command) => command.command === "meeting_start");
  const calendarOrder = fromCalendar.mock.calls[0]![0];
  // Same payload: only app-core may choose the recording's duration.
  expect(todayOrder).toEqual(calendarOrder);
  expect(todayOrder).toEqual({
    command: "meeting_start",
    event_id: event.id,
    event: {
      calendar_item_id: event.calendar_item_id,
      occurrence_start_ms: event.occurrence_start_ms,
    },
  });
});

it("the calendar recognises the recording state app-core projects", () => {
  const projected = { ...event, recording: "recording" as const };
  expect(eventLook(projected, now).recording).toBe(true);
});

it("the open event keeps its reference when another read arrives", () => {
  const send = vi.fn<(command: AppCommand) => Promise<void>>().mockResolvedValue();
  const calendar = { events: [event] } as CalendarSnapshot;
  const hook = renderHook(({ calendar }) => useCalendarView(calendar, "Europe/Madrid", send), {
    initialProps: { calendar },
  });
  act(() => hook.result.current.setSelectedEvent(event));
  hook.rerender({ calendar: { ...calendar, events: [{ ...event, id: "new-observation" }] } });
  const opened = hook.result.current.selectedEvent;
  expect(opened?.id).toBe(event.id);
  act(() => hook.result.current.startMeeting(opened ?? undefined));
  expect(send).toHaveBeenCalledWith({
    command: "meeting_start",
    event_id: event.id,
    event: {
      calendar_item_id: event.calendar_item_id,
      occurrence_start_ms: event.occurrence_start_ms,
    },
  });
});
