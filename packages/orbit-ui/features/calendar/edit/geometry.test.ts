import { describe, expect, it } from "vitest";
import { TEST_NOW_MS } from "../../../test-clock";
import { dayOf, startOfDay, timeLabel } from "../date";
import {
  allDaySpan,
  atWall,
  clickSlot,
  coveredDays,
  dragSlot,
  minutesAtY,
  movedSpan,
  resizedEnd,
  shiftedDays,
} from "./geometry";

const MADRID = "Europe/Madrid";
const UTC = "UTC";
/** The suite's pinned day: no clock change in either zone. */
const DAY = dayOf(TEST_NOW_MS, UTC);
const at = (hour: number, minute = 0) => atWall(DAY, hour * 60 + minute, UTC);
const labels = (span: { endMs: number; startMs: number }, zone = UTC) =>
  `${timeLabel(span.startMs, zone)}-${timeLabel(span.endMs, zone)}`;

describe("a click in free time", () => {
  it("starts at the quarter hour below and lasts 30 min", () => {
    expect(labels(clickSlot(DAY, 9 * 60 + 22, UTC, []))).toBe("9:15-9:45");
    expect(labels(clickSlot(DAY, 9 * 60, UTC, []))).toBe("9:00-9:30");
  });

  it("is cut short at the day's next event, never below 15 min", () => {
    expect(labels(clickSlot(DAY, 9 * 60 + 1, UTC, [at(9, 20)]))).toBe("9:00-9:20");
    expect(labels(clickSlot(DAY, 9 * 60 + 1, UTC, [at(9, 5)]))).toBe("9:00-9:15");
    // An event that already started does not cut the new one.
    expect(labels(clickSlot(DAY, 9 * 60 + 1, UTC, [at(8, 30), at(10)]))).toBe("9:00-9:30");
  });

  it("does not go past midnight", () => {
    const late = clickSlot(DAY, 23 * 60 + 50, UTC, []);
    expect(labels(late)).toBe("23:45-0:00");
    expect(late.endMs).toBe(startOfDay(DAY + 1, UTC));
  });

  it("in Madrid puts 9:00 at 9:00 on the wall clock", () => {
    expect(labels(clickSlot(DAY, 9 * 60, MADRID, []), MADRID)).toBe("9:00-9:30");
  });
});

describe("dragging in the grid", () => {
  it("rounds each end to the nearest quarter hour", () => {
    expect(labels(dragSlot(DAY, 9 * 60 + 8, 10 * 60 + 52, UTC))).toBe("9:15-10:45");
    expect(labels(dragSlot(DAY, 11 * 60, 10 * 60 + 7, UTC))).toBe("10:00-11:00");
  });

  it("lasts at least 15 min and does not leave the day", () => {
    expect(labels(dragSlot(DAY, 9 * 60 + 1, 9 * 60 + 4, UTC))).toBe("9:00-9:15");
    expect(labels(dragSlot(DAY, 23 * 60 + 58, 24 * 60, UTC))).toBe("23:45-0:00");
  });

  it("reads the minutes from the position in pixels", () => {
    expect(minutesAtY(64 * 9.5, 64)).toBe(9 * 60 + 30);
    expect(minutesAtY(-20, 64)).toBe(0);
    expect(minutesAtY(64 * 30, 64)).toBe(24 * 60);
  });
});

describe("moving and stretching", () => {
  const event = { endMs: at(10, 0), startMs: at(9, 5) };

  it("moving leaves the start on a quarter hour and keeps the duration", () => {
    const moved = movedSpan(event, 0, 52, UTC);
    expect(labels(moved)).toBe("10:00-10:55");
    expect(moved.endMs - moved.startMs).toBe(event.endMs - event.startMs);
  });

  it("moving to another day changes column", () => {
    const moved = movedSpan({ endMs: at(9, 30), startMs: at(9) }, 2, 0, UTC);
    expect(dayOf(moved.startMs, UTC)).toBe(DAY + 2);
    expect(labels(moved)).toBe("9:00-9:30");
  });

  it("stretching the bottom edge goes to the nearest quarter, at least 15 min", () => {
    expect(labels(resizedEnd({ endMs: at(9, 30), startMs: at(9) }, DAY, 11 * 60 + 6, UTC))).toBe(
      "9:00-11:00",
    );
    expect(labels(resizedEnd({ endMs: at(9, 30), startMs: at(9) }, DAY, 8 * 60, UTC))).toBe(
      "9:00-9:15",
    );
  });
});

describe("all-day, by days", () => {
  it("from midnight of the first day to the last second of the last", () => {
    const span = allDaySpan(DAY + 2, DAY, MADRID);
    expect(span.startMs).toBe(startOfDay(DAY, MADRID));
    expect(span.endMs).toBe(startOfDay(DAY + 3, MADRID) - 1_000);
    expect(coveredDays(span, MADRID)).toEqual({ first: DAY, last: DAY + 2 });
  });

  it("a bar moves by days and stretches by its sides", () => {
    const bar = allDaySpan(DAY, DAY + 1, UTC);
    expect(coveredDays(shiftedDays(bar, { delta: 3, kind: "move" }, UTC), UTC)).toEqual({
      first: DAY + 3,
      last: DAY + 4,
    });
    expect(coveredDays(shiftedDays(bar, { day: DAY - 2, kind: "start" }, UTC), UTC)).toEqual({
      first: DAY - 2,
      last: DAY + 1,
    });
    expect(coveredDays(shiftedDays(bar, { day: DAY - 5, kind: "end" }, UTC), UTC)).toEqual({
      first: DAY,
      last: DAY,
    });
  });

  it("reads an end at exclusive midnight the same way", () => {
    expect(
      coveredDays({ endMs: startOfDay(DAY + 1, UTC), startMs: startOfDay(DAY, UTC) }, UTC),
    ).toEqual({ first: DAY, last: DAY });
  });
});
