import { render } from "@testing-library/react";
import { expect, it } from "vitest";
import type { BriefingSnapshot } from "../../bridge/generated/BriefingSnapshot";
import { isoWeekForDay, localDayIndex } from "../../local-time";
import type { AppView } from "../../state/app-view";
import { localDateLabel } from "./briefing-clock";
import { BriefingSections } from "./BriefingSections";
import { timeLabel } from "../calendar/date";

it("la agenda de Hoy formatea las dos 02:30 locales del cambio de hora", () => {
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

it("la fecha y la semana de Hoy cambian a medianoche local", () => {
  const now = Date.UTC(2026, 8, 15, 22, 30);
  expect(localDateLabel(now, "Europe/Madrid")).toBe("Miércoles 16 de septiembre");
  expect(isoWeekForDay(localDayIndex(now, "Europe/Madrid"))).toBe(38);
});

it("el día que cambia la hora conserva la fecha y la semana locales", () => {
  const now = Date.UTC(2026, 9, 25, 22, 30);
  expect(localDateLabel(now, "Europe/Madrid")).toBe("Domingo 25 de octubre");
  expect(isoWeekForDay(localDayIndex(now, "Europe/Madrid"))).toBe(43);
});

it("la agenda de Hoy usa el mismo formato horario que el calendario", () => {
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
