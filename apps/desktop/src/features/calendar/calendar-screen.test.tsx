import { act, cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { AppCommand } from "../../bridge/generated";
import type { CalendarEventSnapshot } from "../../bridge/generated/CalendarEventSnapshot";
import type { WriteOutcome } from "../../bridge/generated/WriteOutcome";
import type { AppView as AppSnapshot } from "../../state/app-view";
import { useUiStore } from "../../state/ui";
import { FIXTURE_WEEK_START_MS } from "../../test-clock";
import CalendarView from "./CalendarView";

// The suite's now is Tuesday 15 September 2026, 14:30 UTC (`test-clock.ts`);
// the week on screen starts on Monday 14.
const HOUR = 3_600_000;
const DAY = 24 * HOUR;
const MONDAY = FIXTURE_WEEK_START_MS + DAY;
const at = (day: number, hours: number, minutes = 0) =>
  MONDAY + day * DAY + hours * HOUR + minutes * 60_000;

type Calendar = AppSnapshot["calendar"];

function event(draft: Partial<CalendarEventSnapshot> & { id: string }): CalendarEventSnapshot {
  return {
    calendar_id: "work",
    calendar_item_id: `item-${draft.id}`,
    occurrence_start_ms: draft.start_ms ?? at(1, 16),
    title: "Evento",
    start_ms: at(1, 16),
    end_ms: at(1, 17),
    can_edit: true,
    availability: "busy",
    alarms_minutes: [],
    ...draft,
  };
}

const review = event({
  id: "review",
  title: "Revisión del tablero",
  start_ms: at(1, 16),
  end_ms: at(1, 16, 45),
  conference_url: "https://meet.example/tablero",
  recurrence: { frequency: "weekly", interval: 1 },
  self_response: "accepted",
  attendees: [
    { name: "Nora Prieto", email: "nora@example.test", response: "accepted" },
    { name: "Teo Ruiz", email: "teo@example.test", response: "tentative" },
  ],
});
const visit = event({
  id: "visit",
  calendar_id: "client",
  title: "Visita del cliente",
  start_ms: at(2, 10),
  end_ms: at(2, 11),
  can_edit: false,
});
const interview = event({
  id: "interview",
  title: "Entrevista de prácticas",
  start_ms: at(3, 12),
  end_ms: at(3, 13),
  self_response: "needs_action",
});
const retro = event({
  id: "retro",
  title: "Retrospectiva",
  start_ms: at(4, 15),
  end_ms: at(4, 16),
  self_response: "tentative",
});
const earlier = event({
  id: "earlier",
  title: "Desayuno de equipo",
  start_ms: at(0, 9),
  end_ms: at(0, 10),
  location: "Café del Puerto",
});

function calendar(change: Partial<Calendar> = {}): Calendar {
  return {
    authorization: "granted",
    calendars: [
      { id: "work", title: "Trabajo", source_id: "local", color: "accent", can_edit: true },
      { id: "client", title: "Cliente", source_id: "local", color: "blue", can_edit: false },
    ],
    window: { start_ms: MONDAY, end_ms: MONDAY + 7 * DAY, timezone: "UTC" },
    range: {
      view: "week",
      days: 7,
      week_start: "monday",
      anchor_ms: at(1, 0),
      start_ms: MONDAY,
      end_ms: MONDAY + 7 * DAY,
    },
    events: [earlier, review, visit, interview, retro],
    reading: { state: "available" },
    stale: false,
    writes: [],
    ...change,
  };
}

function snapshot(change: Partial<Calendar> = {}): AppSnapshot {
  return { calendar: calendar(change), system_timezone: "UTC" } as unknown as AppSnapshot;
}

function setup(change: Partial<Calendar> = {}) {
  const send = vi.fn<(command: AppCommand) => Promise<void>>().mockResolvedValue();
  const view = render(<CalendarView error={null} send={send} snapshot={snapshot(change)} />);
  const rerender = (next: Partial<Calendar>) =>
    view.rerender(
      <CalendarView error={null} send={send} snapshot={snapshot({ ...change, ...next })} />,
    );
  return { rerender, send };
}

const sent = (send: ReturnType<typeof setup>["send"]) =>
  send.mock.calls.map(([command]) => command);
const panel = () => screen.getByRole("complementary", { name: "Detalles del evento" });

beforeEach(() => {
  window.localStorage.clear();
  useUiStore.setState({ selectedCalendarEventId: null });
});
afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

describe("the header", () => {
  it("names the week «Septiembre 2026 · semana 38» and offers the five views", () => {
    setup();
    expect(screen.getByRole("heading", { name: "Septiembre 2026 · semana 38" })).toBeTruthy();
    const views = screen.getByRole("radiogroup", { name: "Vista del calendario" });
    expect(
      within(views)
        .getAllByRole("radio")
        .map((radio) => radio.textContent),
    ).toEqual(["Día", "3 días", "Semana", "Mes", "Agenda"]);
    expect(within(views).getByRole("radio", { name: "Semana" }).getAttribute("aria-checked")).toBe(
      "true",
    );
  });

  it("choosing «Mes» asks for the month around the day on screen", () => {
    const { send } = setup();
    fireEvent.click(screen.getByRole("radio", { name: "Mes" }));
    expect(sent(send)).toContainEqual(
      expect.objectContaining({ command: "calendar_set_range", view: "month" }),
    );
  });

  it("the filter says how many calendars there are, with one dot each", () => {
    setup();
    const filter = screen.getByRole("button", { name: /2 calendarios/ });
    fireEvent.click(filter);
    const choices = screen.getByRole("dialog", { name: "Calendarios visibles" });
    fireEvent.click(within(choices).getByRole("checkbox", { name: "Cliente" }));
    expect(screen.queryByRole("button", { name: /Visita del cliente/ })).toBeNull();
    expect(screen.getByRole("button", { name: /1 de 2 calendarios/ })).toBeTruthy();
  });

  it("names a day «Martes, 15 de septiembre» and three days «15 – 17 de septiembre»", () => {
    setup({
      range: {
        view: "day",
        days: 1,
        week_start: "monday",
        anchor_ms: at(1, 0),
        start_ms: at(1, 0),
        end_ms: at(2, 0),
      },
    });
    expect(screen.getByRole("heading", { name: "Martes, 15 de septiembre" })).toBeTruthy();
    cleanup();
    setup({
      range: {
        view: "days",
        days: 3,
        week_start: "monday",
        anchor_ms: at(1, 0),
        start_ms: at(1, 0),
        end_ms: at(4, 0),
      },
    });
    expect(screen.getByRole("heading", { name: "15 – 17 de septiembre" })).toBeTruthy();
  });
});

describe("the grid", () => {
  it("marks maybe and unanswered events with their label, and a past one", () => {
    setup();
    expect(screen.getByRole("button", { name: /Retrospectiva.*Quizá/ })).toBeTruthy();
    expect(screen.getByRole("button", { name: /Entrevista de prácticas.*Sin responder/ }));
    const past = screen.getByRole("button", { name: /Desayuno de equipo/ });
    expect(past.hasAttribute("data-past")).toBe(true);
  });

  it("draws the present with its time in the hours column", () => {
    setup();
    expect(screen.getByRole("img", { name: "Ahora, 14:30" })).toBeTruthy();
  });

  it("rings the event whose panel is open", () => {
    setup();
    fireEvent.click(screen.getByRole("button", { name: /Revisión del tablero/ }));
    expect(
      screen.getByRole("button", { name: /Revisión del tablero/ }).getAttribute("aria-pressed"),
    ).toBe("true");
  });
});

describe("the event panel", () => {
  it("follows the design's order and says «Sin cambios» until something changes", () => {
    setup();
    fireEvent.click(screen.getByRole("button", { name: /Revisión del tablero/ }));
    const details = panel();
    expect(details.querySelector("header")?.textContent).toContain("Trabajo");
    expect(within(details).getByText("Se repite: cada semana")).toBeTruthy();
    expect(within(details).getByRole("button", { name: "Unirse y grabar" })).toBeTruthy();
    expect(within(details).getByRole("button", { name: /Solo grabar/ })).toBeTruthy();
    expect(
      within(details).getByText("meet.example/tablero · graba hasta 15 min después del final"),
    ).toBeTruthy();
    expect(within(details).getByText("Avisarme")).toBeTruthy();
    expect(within(details).getByRole("radio", { name: "Ocupada" })).toBeTruthy();
    expect(within(details).getByText("Has aceptado")).toBeTruthy();
    expect(within(details).getByText("Invitados · 2")).toBeTruthy();
    expect(within(details).getByText("Va")).toBeTruthy();
    expect(within(details).getByText("Quizá")).toBeTruthy();
    expect(within(details).getByText("Sin cambios")).toBeTruthy();
    expect(within(details).getByRole("button", { name: "Borrar" })).toBeTruthy();
  });

  it("«Unirse y grabar» opens the call and starts recording the event", () => {
    const { send } = setup();
    fireEvent.click(screen.getByRole("button", { name: /Revisión del tablero/ }));
    fireEvent.click(within(panel()).getByRole("button", { name: "Unirse y grabar" }));
    expect(sent(send)).toEqual([
      { command: "open_url", url: "https://meet.example/tablero" },
      {
        command: "meeting_start",
        event_id: "review",
        event: { calendar_item_id: "item-review", occurrence_start_ms: at(1, 16) },
      },
    ]);
  });

  it("a change to a recurring event offers «Solo este» and «Este y los siguientes», then «Guardando…» and «✓ Guardado»", () => {
    const { rerender, send } = setup();
    fireEvent.click(screen.getByRole("button", { name: /Revisión del tablero/ }));
    fireEvent.change(within(panel()).getByRole("textbox", { name: "Ubicación" }), {
      target: { value: "Sala Norte" },
    });
    expect(within(panel()).queryByText("Sin cambios")).toBeNull();
    expect(within(panel()).getByRole("button", { name: "Este y los siguientes" })).toBeTruthy();
    fireEvent.click(within(panel()).getByRole("button", { name: "Solo este" }));
    const save = sent(send).find((command) => command.command === "calendar");
    expect(save).toMatchObject({
      request: {
        action: "save",
        save: { change: { span: "this", changes: { location: "Sala Norte" } } },
      },
    });
    // The panel stays, and says the write is on its way.
    expect(within(panel()).getByText("Guardando…")).toBeTruthy();
    const requestId =
      save?.command === "calendar" && save.request.action === "save" ? save.request.request_id : "";
    const saved: WriteOutcome = { request_id: requestId, operation: "update", stage: "saved" };
    rerender({
      writes: [saved],
      events: [earlier, { ...review, location: "Sala Norte" }, visit, interview, retro],
    });
    expect(within(panel()).getByText("✓ Guardado en tu calendario")).toBeTruthy();
  });

  it("a refused save gives the changes back to the panel", () => {
    const { rerender, send } = setup();
    fireEvent.click(screen.getByRole("button", { name: /Revisión del tablero/ }));
    fireEvent.change(within(panel()).getByRole("textbox", { name: "Ubicación" }), {
      target: { value: "Sala Norte" },
    });
    fireEvent.click(within(panel()).getByRole("button", { name: "Solo este" }));
    const save = sent(send).find((command) => command.command === "calendar");
    const requestId =
      save?.command === "calendar" && save.request.action === "save" ? save.request.request_id : "";
    rerender({ writes: [{ request_id: requestId, operation: "update", stage: "refused" }] });
    expect(within(panel()).getByRole("button", { name: "Este y los siguientes" })).toBeTruthy();
    expect(screen.getByRole("alert").textContent).toContain("El calendario no aplicó el cambio");
  });

  it("an event of a read-only calendar shows the lock and no fields", () => {
    setup();
    fireEvent.click(screen.getByRole("button", { name: /Visita del cliente/ }));
    const details = panel();
    expect(within(details).getByRole("heading", { name: "Visita del cliente" })).toBeTruthy();
    expect(within(details).getByText(/Este calendario es de solo lectura\./)).toBeTruthy();
    expect(within(details).queryByRole("textbox")).toBeNull();
    expect(within(details).queryByRole("button", { name: "Borrar" })).toBeNull();
  });

  it("«Borrar» on a recurring event asks which ones, inside its own menu", () => {
    const { send } = setup();
    fireEvent.click(screen.getByRole("button", { name: /Revisión del tablero/ }));
    fireEvent.click(within(panel()).getByRole("button", { name: "Borrar" }));
    const menu = screen.getByRole("dialog", { name: "Borrar" });
    fireEvent.click(within(menu).getByRole("button", { name: "Este y los siguientes" }));
    expect(sent(send)).toContainEqual(
      expect.objectContaining({
        command: "calendar",
        request: expect.objectContaining({ action: "delete" }),
      }),
    );
  });
});

describe("the month", () => {
  const month = {
    range: {
      view: "month" as const,
      days: 35,
      week_start: "monday" as const,
      anchor_ms: at(1, 0),
      start_ms: MONDAY - 14 * DAY,
      end_ms: MONDAY + 21 * DAY,
    },
  };

  it("picking a day shows its agenda on the right with «Abrir este día»", () => {
    const { send } = setup(month);
    expect(screen.getByRole("heading", { name: "Martes, 15 de septiembre" })).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: /jueves 17 de septiembre/ }));
    const day = screen.getByRole("complementary", { name: "Día elegido" });
    expect(within(day).getByRole("heading", { name: "17 de septiembre" })).toBeTruthy();
    expect(within(day).getByText("Entrevista de prácticas")).toBeTruthy();
    expect(within(day).getByText("Sin responder")).toBeTruthy();
    fireEvent.click(within(day).getByRole("button", { name: "Abrir este día" }));
    expect(sent(send)).toContainEqual(
      expect.objectContaining({ command: "calendar_set_range", view: "day", anchor_ms: at(3, 0) }),
    );
  });

  it("a double click on a day drafts an all-day event", () => {
    setup(month);
    fireEvent.doubleClick(screen.getByRole("button", { name: /sábado 19 de septiembre/ }));
    expect(within(panel()).getByRole("textbox", { name: "Título" })).toBeTruthy();
    expect(
      within(panel()).getByRole("switch", { name: "Todo el día" }).getAttribute("aria-checked"),
    ).toBe("true");
  });
});

describe("the agenda", () => {
  it("groups by day, marks today with «HOY» and says what a row needs", () => {
    setup({
      range: {
        view: "agenda",
        days: 7,
        week_start: "monday",
        anchor_ms: at(1, 0),
        start_ms: MONDAY,
        end_ms: MONDAY + 7 * DAY,
      },
    });
    expect(screen.getByRole("heading", { name: "Agenda" })).toBeTruthy();
    const today = screen.getByRole("region", { name: "martes 15 de septiembre" });
    expect(within(today).getByText("HOY")).toBeTruthy();
    expect(within(today).getByText("16:00 – 16:45")).toBeTruthy();
    expect(within(today).getByRole("img", { name: "Con videollamada" })).toBeTruthy();
    const thursday = screen.getByRole("region", { name: "jueves 17 de septiembre" });
    expect(within(thursday).getByText("Sin responder · responde en Calendario")).toBeTruthy();
    const monday = screen.getByRole("region", { name: "lunes 14 de septiembre" });
    expect(within(monday).getByText("Café del Puerto")).toBeTruthy();
  });
});

describe("before the calendar can be shown", () => {
  it("without the permission yet it asks for it and waits for macOS", () => {
    const { send } = setup({ authorization: "not_determined", events: [], calendars: [] });
    expect(screen.getByRole("heading", { name: "Conecta tu calendario" })).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Permitir acceso al calendario" }));
    expect(sent(send)).toEqual([
      { command: "calendar", request: { action: "request_permission" } },
    ]);
    expect(screen.getByRole("button", { name: /Esperando a macOS…/ })).toBeTruthy();
  });

  it("while the Mac has not said, it shows the calendar loading, never a blank view", () => {
    setup({ authorization: "unknown", events: [], reading: { state: "unknown" } });
    expect(screen.getByRole("status").textContent).toContain("Leyendo tu calendario…");
    expect(document.querySelector("[aria-busy='true']")).not.toBeNull();
  });

  it("a period still on its way is drawn loading, not empty", () => {
    setup({ stale: true, events: [] });
    expect(screen.getByRole("status").textContent).toContain("Actualizando calendario…");
    expect(document.querySelector("[aria-busy='true']")).not.toBeNull();
  });
});

it("the clock moving keeps a saved confirmation for 1.8 s only", async () => {
  vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
  const { rerender, send } = setup();
  fireEvent.click(screen.getByRole("button", { name: /Desayuno de equipo/ }));
  fireEvent.change(within(panel()).getByRole("textbox", { name: "Ubicación" }), {
    target: { value: "Café de la Plaza" },
  });
  fireEvent.click(within(panel()).getByRole("button", { name: "Guardar" }));
  const save = sent(send).find((command) => command.command === "calendar");
  const requestId =
    save?.command === "calendar" && save.request.action === "save" ? save.request.request_id : "";
  rerender({ writes: [{ request_id: requestId, operation: "update", stage: "saved" }] });
  expect(within(panel()).getByText("✓ Guardado en tu calendario")).toBeTruthy();
  await act(async () => {
    vi.advanceTimersByTime(1_800);
  });
  expect(within(panel()).getByText("Sin cambios")).toBeTruthy();
});
