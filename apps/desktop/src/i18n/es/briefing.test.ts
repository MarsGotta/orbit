import { describe, expect, it } from "vitest";
import { briefing } from "./briefing";

describe("Briefing copy", () => {
  it("calls an event with zero attendees «Solo tú»", () => {
    expect(briefing.briefing.peopleCount(0)).toBe("Solo tú");
  });
});
