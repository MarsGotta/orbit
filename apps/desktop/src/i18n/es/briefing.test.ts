import { describe, expect, it } from "vitest";
import { briefing } from "./briefing";

describe("REF-214 Briefing copy", () => {
  it("calls an event with zero attendees solo", () => {
    expect(briefing.briefing.peopleCount(0)).toBe("solo");
  });
});
