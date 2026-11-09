import { describe, expect, it } from "vitest";
import {
  allDayLayout,
  calculateEventBox,
  CASCADE_INDENT_PERCENT,
  hourHeight,
  layoutDayEvents,
} from "./layout";

const HOUR = 60;

describe("24-hour grid", () => {
  it("fits about twelve hours, never less than 61 px an hour, times the zoom", () => {
    expect(hourHeight(0, 1)).toBe(61);
    expect(hourHeight(113 + 12 * 80, 1)).toBe(80);
    expect(hourHeight(113 + 12 * 80, 1.5)).toBe(120);
    expect(hourHeight(0, 0.5)).toBe(30.5);
  });

  it("places an event by its minutes from midnight", () => {
    // 10:45–11:00 at 61 px an hour: 10.75 h down, and the 22 px floor.
    expect(calculateEventBox(10 * HOUR + 45, 11 * HOUR, 61)).toEqual({ height: 22, top: 655.75 });
    expect(calculateEventBox(9 * HOUR, 10 * HOUR, 80)).toEqual({ height: 77, top: 720 });
  });
});

describe("overlaps", () => {
  it("splits the width between events that start close together", () => {
    expect(
      layoutDayEvents([
        { endMinutes: 11 * HOUR, id: "first", startMinutes: 10 * HOUR + 30 },
        { endMinutes: 11 * HOUR + 30, id: "second", startMinutes: 10 * HOUR + 45 },
      ]),
    ).toEqual([
      { depth: 0, id: "first", leftPercent: 0, widthPercent: 50 },
      { depth: 0, id: "second", leftPercent: 50, widthPercent: 50 },
    ]);
  });

  it("three overlapping events share the width in three columns", () => {
    const third = 100 / 3;
    expect(
      layoutDayEvents([
        { endMinutes: 11 * HOUR, id: "a", startMinutes: 10 * HOUR },
        { endMinutes: 10 * HOUR + 45, id: "b", startMinutes: 10 * HOUR + 15 },
        { endMinutes: 11 * HOUR + 30, id: "c", startMinutes: 10 * HOUR + 30 },
      ]),
    ).toEqual([
      { depth: 0, id: "a", leftPercent: 0, widthPercent: third },
      { depth: 0, id: "b", leftPercent: third, widthPercent: third },
      { depth: 0, id: "c", leftPercent: 2 * third, widthPercent: third },
    ]);
  });

  it("one that starts 45 minutes or more after its neighbour overlays it with an indent", () => {
    const [long, late] = layoutDayEvents([
      { endMinutes: 12 * HOUR, id: "long", startMinutes: 9 * HOUR },
      { endMinutes: 11 * HOUR, id: "late", startMinutes: 9 * HOUR + 45 },
    ]);
    expect(long).toEqual({ depth: 0, id: "long", leftPercent: 0, widthPercent: 100 });
    expect(late).toEqual({
      depth: 1,
      id: "late",
      leftPercent: CASCADE_INDENT_PERCENT,
      widthPercent: 100 - CASCADE_INDENT_PERCENT,
    });
  });

  it("44 minutes later still splits the width", () => {
    const placed = layoutDayEvents([
      { endMinutes: 12 * HOUR, id: "long", startMinutes: 9 * HOUR },
      { endMinutes: 11 * HOUR, id: "close", startMinutes: 9 * HOUR + 44 },
    ]);
    expect(placed.map((event) => event.widthPercent)).toEqual([50, 50]);
  });

  it("events that do not overlap keep the whole width", () => {
    const placed = layoutDayEvents([
      { endMinutes: 10 * HOUR, id: "nine", startMinutes: 9 * HOUR },
      { endMinutes: 11 * HOUR, id: "ten", startMinutes: 10 * HOUR },
    ]);
    expect(placed.map((event) => [event.leftPercent, event.widthPercent])).toEqual([
      [0, 100],
      [0, 100],
    ]);
  });
});

describe("all-day lane", () => {
  // Seven columns, days 100 to 106.
  const week = [100, 101, 102, 103, 104, 105, 106];

  it("a three-day event is one bar three columns wide", () => {
    expect(allDayLayout([{ firstDay: 102, id: "trip", lastDay: 104 }], week)).toEqual([
      { clippedEnd: false, clippedStart: false, column: 2, id: "trip", row: 0, span: 3 },
    ]);
  });

  it("one that began before the view is cut at the first column and says so", () => {
    expect(allDayLayout([{ firstDay: 98, id: "congress", lastDay: 100 }], week)).toEqual([
      { clippedEnd: false, clippedStart: true, column: 0, id: "congress", row: 0, span: 1 },
    ]);
  });

  it("bars that share a day go to separate rows; the rest share the first", () => {
    const placed = allDayLayout(
      [
        { firstDay: 98, id: "congress", lastDay: 100 },
        { firstDay: 102, id: "trip", lastDay: 104 },
        { firstDay: 103, id: "holiday", lastDay: 103 },
        { firstDay: 106, id: "sunday", lastDay: 108 },
      ],
      week,
    );
    expect(placed.map((bar) => [bar.id, bar.row, bar.column, bar.span])).toEqual([
      ["congress", 0, 0, 1],
      ["trip", 0, 2, 3],
      ["holiday", 1, 3, 1],
      ["sunday", 0, 6, 1],
    ]);
    expect(placed.at(-1)?.clippedEnd).toBe(true);
  });

  it("without weekends the columns skip them and the bar spans what is left", () => {
    const weekdays = [100, 101, 102, 103, 104];
    expect(allDayLayout([{ firstDay: 103, id: "long", lastDay: 108 }], weekdays)).toEqual([
      { clippedEnd: true, clippedStart: false, column: 3, id: "long", row: 0, span: 2 },
    ]);
    expect(allDayLayout([{ firstDay: 105, id: "weekend", lastDay: 106 }], weekdays)).toEqual([]);
  });
});
