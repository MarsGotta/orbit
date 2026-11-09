import { render } from "@testing-library/react";
import { expect, it } from "vitest";
import type { BriefingSnapshot } from "../../bridge/generated/BriefingSnapshot";
import { isoWeekForDay, localDayIndex } from "../../local-time";
import type { AppView } from "../../state/app-view";
import { localDateLabel } from "./briefing-clock";
import { BriefingSections } from "./BriefingSections";
import { timeLabel } from "../calendar/date";

it("Hoy's agenda formats both local 02:30s of the clock change", () => {
  const briefing: BriefingSnapshot = {
    greeting: { name: null },
    summary: { meetings_today: 2, focus_minutes: 0 },
    next: null,
    sources: {
      agenda: { state: "present" },
      notes: { state: "present" },
      recording: { state: "present" },
    },
    agenda: [Date.UTC(2026, 9, 25, 0, 30), Date.UTC(2026, 9, 25, 1, 30)].map((start_ms, index) => ({
      event_id: `event-${index}`,
      title: "Guardia",
      start_ms,
      color: "accent",
      people_label: "",
      minutes: 30,
    })),
  };
  const snapshot = { briefing, system_timezone: "Europe/Madrid" } as AppView;
  const { container } = render(<BriefingSections briefing={briefing} snapshot={snapshot} />);
  expect([...container.querySelectorAll("time")].map((time) => time.textContent)).toEqual([
    "2:30",
    "2:30",
  ]);
});

it("Hoy's date and week change at local midnight", () => {
  const now = Date.UTC(2026, 8, 15, 22, 30);
  expect(localDateLabel(now, "Europe/Madrid")).toBe("Miércoles 16 de septiembre");
  expect(isoWeekForDay(localDayIndex(now, "Europe/Madrid"))).toBe(38);
});

it("the day the clocks change keeps the local date and week", () => {
  const now = Date.UTC(2026, 9, 25, 22, 30);
  expect(localDateLabel(now, "Europe/Madrid")).toBe("Domingo 25 de octubre");
  expect(isoWeekForDay(localDayIndex(now, "Europe/Madrid"))).toBe(43);
});

it("Hoy's agenda uses the same time format as the calendar", () => {
  const at = Date.UTC(2026, 9, 19, 7, 0);
  const briefing: BriefingSnapshot = {
    greeting: { name: null },
    summary: { meetings_today: 1, focus_minutes: 0 },
    next: null,
    sources: {
      agenda: { state: "present" },
      notes: { state: "present" },
      recording: { state: "present" },
    },
    agenda: [
      {
        event_id: "e",
        title: "Daily",
        start_ms: at,
        color: "accent",
        people_label: "",
        minutes: 30,
      },
    ],
  };
  const snapshot = { briefing, system_timezone: "Europe/Madrid" } as AppView;
  const { container } = render(<BriefingSections briefing={briefing} snapshot={snapshot} />);
  expect(container.querySelector("time")?.textContent).toBe(timeLabel(at, "Europe/Madrid"));
});
