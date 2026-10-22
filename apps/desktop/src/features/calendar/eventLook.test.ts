import { expect, it } from "vitest";
import { calendarTone } from "./eventLook";

it("a calendar's palette token keeps its colour among the design's four", () => {
  expect(["accent", "blue", "green", "orange"].map(calendarTone)).toEqual([1, 2, 3, 4]);
});

it("a calendar's own macOS colour takes the tone nearest in hue", () => {
  // Calendar's own violet, blue, green, red, yellow, and a grey.
  expect(
    ["#8E44AD", "#1D9BF6", "#63DA38", "#FF2968", "#FFCC00", "#A2845E"].map(calendarTone),
  ).toEqual([1, 2, 3, 4, 4, 4]);
  expect(calendarTone("#8E8E93")).toBe(1);
  expect(calendarTone("not a colour")).toBe(1);
  expect(calendarTone(undefined)).toBe(1);
});
