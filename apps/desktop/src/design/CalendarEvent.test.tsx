import { expect, it } from "vitest";
import { eventLines } from "./CalendarEvent";

it("a 30-minute event has one line of title and no time; a longer one shows its time", () => {
  // 30 minutes in a 48 px hour is 24 px.
  expect(eventLines(24, false)).toEqual({ titleLines: 1, showTime: false });
  expect(eventLines(40, false)).toEqual({ titleLines: 1, showTime: true });
  expect(eventLines(72, false)).toEqual({ titleLines: 3, showTime: true });
  expect(eventLines(72, true)).toEqual({ titleLines: 2, showTime: true });
});

it("in a list nothing is cut", () => {
  expect(eventLines(undefined, true)).toEqual({ titleLines: undefined, showTime: true });
});
