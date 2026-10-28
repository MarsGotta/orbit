import { describe, expect, it } from "vitest";
import { briefingCopy } from "./briefing";

describe("Briefing copy", () => {
  it("calls an event with zero attendees «Solo tú»", () => {
    expect(briefingCopy("Atlas").briefing.peopleCount(0)).toBe("Solo tú");
  });
});
