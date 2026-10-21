import type { AppView as AppSnapshot } from "../../state/app-view";
import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";

import { FIXTURE_WEEK_START_MS } from "../../test-clock";
import { dayOf, eventSegments, rangeDays, startOfDay } from "./date";
import { hourHeight } from "./layout";
import { TimeGrid } from "./TimeGrid";

type CalendarEvent = AppSnapshot["calendar"]["events"][number];

const MADRID = "Europe/Madrid";
const NEW_YORK = "America/New_York";
const MINUTE_MS = 60_000;
const HOUR_MS = 60 * MINUTE_MS;
const DAY_MS = 24 * HOUR_MS;
// Six weeks after the shared fixture anchor is the Sunday on which Madrid's
// clocks go back; twenty-four weeks before it is the Sunday they jump
// 02:00→03:00 (29 March 2026). Both stay owned by test-clock's anchor: the
// literal titles and positions below turn red if the anchor ever moves.
// The grid is given each week at its Madrid midnight — the instant
// the app publishes — while the events keep their UTC instants.
// app-core publishes the week; these weeks begin on the Sunday at
// local midnight, and the grid paints 24 hours at
// 61 px an hour (the floor before the area is measured).
const HOUR_PX = hourHeight(0, 1);
const AUTUMN_UTC_SUNDAY = FIXTURE_WEEK_START_MS + 6 * 7 * DAY_MS;
const WEEK_START_MS = localMidnight(AUTUMN_UTC_SUNDAY + 12 * HOUR_MS, MADRID);
const SPRING_UTC_SUNDAY = FIXTURE_WEEK_START_MS - 24 * 7 * DAY_MS;
const SPRING_WEEK_START_MS = localMidnight(SPRING_UTC_SUNDAY + 12 * HOUR_MS, MADRID);
const NEW_YORK_WEEK_START_MS = localMidnight(FIXTURE_WEEK_START_MS + 14 * HOUR_MS, NEW_YORK);

function localMidnight(instant: number, timezone: string): number {
  return startOfDay(dayOf(instant, timezone), timezone);
}

function week(startMs: number, timezone: string) {
  return {
    anchor_ms: startMs,
    days: 7,
    end_ms: startOfDay(dayOf(startMs, timezone) + 7, timezone),
    start_ms: startMs,
    view: "week" as const,
    week_start: "sunday" as const,
  };
}

const px = (hours: number) => `${hours * HOUR_PX}px`;

function event(id: string, title: string, startMs: number, endMs: number): CalendarEvent {
  return {
    all_day: false,
    attendees: [],
    calendar_id: "work",
    end_ms: endMs,
    id,
    location: null,
    start_ms: startMs,
    status: "confirmed",
    title,
    url: null,
  };
}

function renderGrid(timezone: string, weekStartMs: number, events: CalendarEvent[]) {
  render(
    <TimeGrid
      allDayOpen={false}
      calendars={[
        {
          color: "accent",
          id: "work",
          source_id: "local",
          source_title: "En mi Mac",
          title: "Trabajo",
        },
      ]}
      events={events}
      hourPx={HOUR_PX}
      nowMs={weekStartMs + 12 * HOUR_MS}
      onOpenEvent={() => undefined}
      onToggleAllDay={() => undefined}
      range={week(weekStartMs, timezone)}
      showWeekends
      timezone={timezone}
    />,
  );
}

afterEach(cleanup);

describe("local time in the grid", () => {
  it("places Madrid's 10:00 in row 10 on the day the clocks change", () => {
    renderGrid(MADRID, WEEK_START_MS, [
      event(
        "local-ten",
        "Diez locales",
        AUTUMN_UTC_SUNDAY + 9 * HOUR_MS,
        AUTUMN_UTC_SUNDAY + 10 * HOUR_MS,
      ),
    ]);

    const button = screen.getByRole("button", { name: /Diez locales/ });
    expect(button.style.top).toBe(px(10));
    expect(button.style.height).toBe(`${HOUR_PX - 3}px`);
    expect(button.textContent).toContain("10:00");
  });

  it("draws both parts of an event that crosses midnight whole", () => {
    renderGrid(MADRID, WEEK_START_MS, [
      event(
        "overnight",
        "Guardia nocturna",
        AUTUMN_UTC_SUNDAY + 2 * DAY_MS + (20 * 60 + 30) * MINUTE_MS,
        AUTUMN_UTC_SUNDAY + 3 * DAY_MS + (6 * 60 + 30) * MINUTE_MS,
      ),
    ]);

    const parts = screen.getAllByRole("button", { name: /Guardia nocturna/ });
    expect(parts).toHaveLength(2);
    expect(parts.map((part) => [part.style.top, part.style.height])).toEqual([
      // 21:30→24:00 and 00:00→07:30 CET, the week after the change.
      [px(21.5), `${2.5 * HOUR_PX - 3}px`],
      [px(0), `${7.5 * HOUR_PX - 3}px`],
    ]);
  });

  it("with a zone west of UTC it does not shift the event to the next column", () => {
    // Sunday 10:00 in New York is 14:00 UTC of that same Sunday. The week is
    // published at New York's midnight and the event belongs under
    // the Sunday column, not under Monday's.
    renderGrid(NEW_YORK, NEW_YORK_WEEK_START_MS, [
      event(
        "ny-ten",
        "Diez Nueva York",
        FIXTURE_WEEK_START_MS + 14 * HOUR_MS,
        FIXTURE_WEEK_START_MS + 15 * HOUR_MS,
      ),
    ]);

    const columns = screen.getAllByRole("gridcell");
    const button = screen.getByRole("button", { name: /Diez Nueva York/ });
    expect(columns[0]!.contains(button)).toBe(true);
    expect(button.style.top).toBe(px(10));
    expect(button.textContent).toContain("10:00");
  });

  it("on 29 March 2026 in Madrid: the row is the local hour and the jump is drawn whole", () => {
    // 10:00 in Madrid on the spring-change Sunday is 08:00 UTC; and an
    // event from 01:30 to 04:30 wall clock crosses the 02:00→03:00 jump
    // (00:30Z→02:30Z) and must still be one whole box on its Sunday.
    renderGrid(MADRID, SPRING_WEEK_START_MS, [
      event(
        "spring-ten",
        "Diez locales",
        SPRING_UTC_SUNDAY + 8 * HOUR_MS,
        SPRING_UTC_SUNDAY + 9 * HOUR_MS,
      ),
      event(
        "spring-jump",
        "Cruza el salto",
        SPRING_UTC_SUNDAY + 30 * MINUTE_MS,
        SPRING_UTC_SUNDAY + 150 * MINUTE_MS,
      ),
    ]);

    const columns = screen.getAllByRole("gridcell");
    const ten = screen.getByRole("button", { name: /Diez locales/ });
    expect(columns[0]!.contains(ten)).toBe(true);
    expect(ten.style.top).toBe(px(10));
    expect(ten.textContent).toContain("10:00");

    const jump = screen.getByRole("button", { name: /Cruza el salto/ });
    expect(columns[0]!.contains(jump)).toBe(true);
    // 01:30→04:30 wall clock: 90 to 270 minutes of the day.
    expect(jump.style.top).toBe(px(1.5));
    expect(jump.style.height).toBe(`${3 * HOUR_PX - 3}px`);
  });

  it("an event that ends at local midnight paints no empty piece on the next day", () => {
    // 22:00→24:00 Madrid on the week's first Sunday (the autumn-change one):
    // 21:00Z to 23:00Z, ending on local midnight. One segment, and nothing
    // on Monday.
    const days = rangeDays(week(WEEK_START_MS, MADRID), MADRID);
    const segments = eventSegments(
      AUTUMN_UTC_SUNDAY + 21 * HOUR_MS,
      AUTUMN_UTC_SUNDAY + 23 * HOUR_MS,
      days,
      MADRID,
    );
    expect(segments).toEqual([{ day: days[0], endMinutes: 24 * 60, startMinutes: 22 * 60 }]);
  });

  it("a hundred-year event makes no more segments than the week paints", () => {
    const days = rangeDays(week(WEEK_START_MS, MADRID), MADRID);
    const segments = eventSegments(
      WEEK_START_MS - DAY_MS,
      WEEK_START_MS + 100 * 365 * DAY_MS,
      days,
      MADRID,
    );
    expect(
      segments.length,
      `la rejilla generó ${segments.length} segmentos para un solo evento`,
    ).toBeLessThanOrEqual(7);
    expect(segments.map((segment) => segment.day)).toEqual(days);
  });
});
