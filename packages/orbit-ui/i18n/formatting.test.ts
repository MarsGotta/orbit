import { expect, it } from "vitest";
import { configureCopy, getCopy } from ".";
import { formatClock } from "../features/notes/notes-format";
import { dayAndMonth, hourLabel } from "../features/calendar/date";

it("formats counts, durations and dates with the live language and English region", () => {
  const instant = Date.UTC(2026, 9, 14, 15, 5);
  configureCopy("en", ["en-US"], "Atlas");
  expect(getCopy().briefing.peopleCount(12345)).toBe("12,345 people");
  expect(getCopy().noteScreen.minutes(12345)).toBe("12,345 min");
  expect(
    getCopy().extras.aiRefusalText({
      code: "ai_catalog_size",
      status: 422,
      numbers: { count: 12345, max_count: 20000 },
    }),
  ).toBe("The catalogue has 12,345 models; it must have between 1 and 20,000.");
  expect(formatClock(instant)).toBe("3:05 PM");
  expect(hourLabel(15)).toBe("3:00 PM");
  expect(dayAndMonth(20740)).not.toContain(" de ");
  configureCopy("en", ["en-GB"], "Atlas");
  expect(formatClock(instant)).toBe("15:05");
  expect(getCopy().briefing.dayLine(null)).toBe("Your day from 8:00 to 20:00.");
  configureCopy("es", ["en-GB"], "Atlas");
  expect(getCopy().briefing.peopleCount(12345)).toBe("12.345 personas");
  expect(getCopy().noteScreen.minutes(12345)).toBe("12.345 min");
  expect(formatClock(instant)).toBe("15:05");
});
