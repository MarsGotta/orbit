import { cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { AppCommand } from "../../bridge/generated";
import type { BriefingNextSnapshot } from "../../bridge/generated/BriefingNextSnapshot";
import type { BriefingPreviousActions } from "../../bridge/generated/BriefingPreviousActions";
import type { BriefingSnapshot } from "../../bridge/generated/BriefingSnapshot";
import type { MeetingSnapshot } from "../../bridge/generated/MeetingSnapshot";
import type { AppView } from "../../state/app-view";
import { TEST_NOW_MS } from "../../test-clock";
import BriefingView from "./BriefingView";

// Hoy as the redesign draws it (Main.dc.html): the day, the next meeting
// with what was left open last time, the personal reminder and the day's
// agenda. The suite's clock is Tuesday 15 September 2026, 14:30 UTC.
const MINUTE = 60_000;
const now = TEST_NOW_MS;

const known: BriefingPreviousActions = {
  state: "known",
  recording_id: "rec-earlier",
  note_revision: 4,
  items: [
    { id: "a1", owner: "Marta", text: "prototipo del paso de pago", due: null, done: false },
    { id: "a2", owner: null, text: "medir la disponibilidad", due: null, done: false },
  ],
};

function next(change: Partial<BriefingNextSnapshot> = {}): BriefingNextSnapshot {
  return {
    event_id: "review",
    event: { calendar_item_id: "series-review", occurrence_start_ms: now + 20 * MINUTE },
    title: "Revisión del diseño de reservas",
    start_ms: now + 20 * MINUTE,
    end_ms: now + 65 * MINUTE,
    attendees: [
      { name: "Marta Ibáñez", initial: "M", color: "orange" },
      { name: "Diego Salinas", initial: "D", color: "blue" },
      { name: "Javier Ortega", initial: "J", color: "green" },
    ],
    url: "https://meet.example/revision",
    series_key: "revision del diseno",
    previous: {
      recording_id: "rec-earlier",
      title: "Revisión del diseño de reservas",
      summary: "Se acordó simplificar el paso de pago.",
      open_items: [],
    },
    previous_actions: known,
    recording: "none",
    ...change,
  };
}

function briefing(change: Partial<BriefingSnapshot> = {}): BriefingSnapshot {
  const meeting = next();
  return {
    greeting: { name: "Lucía" },
    summary: { meetings_today: 3, focus_minutes: 165 },
    next: meeting,
    upcoming: [meeting],
    agenda: [
      row("daily", 9, 0, 30, "accent", 5),
      row("review", 14, 50, 45, "accent", 4),
      row("yoga", 19, 0, 60, "green", 0),
    ],
    sources: {
      agenda: { state: "present" },
      notes: { state: "present" },
      recording: { state: "present" },
    },
    ...change,
  };
}

function row(
  id: string,
  hour: number,
  minute: number,
  minutes: number,
  color: string,
  people: number,
) {
  const titles: Record<string, string> = {
    daily: "Daily del equipo",
    review: "Revisión del diseño de reservas",
    yoga: "Yoga",
  };
  return {
    event_id: id,
    start_ms: Date.UTC(2026, 8, 15, hour, minute),
    title: titles[id] ?? id,
    color,
    people_label: "",
    minutes,
    people_count: people,
  };
}

const idle = {
  phase: "idle",
  recording_id: null,
  title: null,
  started_at_ms: null,
  ends_at_ms: null,
  limit_minutes: 60,
  tracks: [],
  recent: [
    {
      recording_id: "rec-daily",
      title: "Daily del equipo",
      started_at_ms: Date.UTC(2026, 8, 15, 9, 0),
      duration_ms: 14 * MINUTE,
      calendar_event_id: "daily",
      transcription: { pending: 0, running: 0, completed: 3, failed: 0 },
      summary: "ready",
    },
  ],
  voice_trigger: { state: "listening" },
} as unknown as MeetingSnapshot;

function snapshot(change: Partial<AppView> = {}): AppView {
  return {
    system_timezone: "UTC",
    calendar: { authorization: "granted", reading: { state: "available" }, stale: false },
    meeting: idle,
    briefing: briefing(),
    ...change,
  } as unknown as AppView;
}

function show(value: AppView, send = vi.fn<(command: AppCommand) => Promise<void>>()) {
  const onRecord = vi.fn();
  const view = render(
    <BriefingView error={null} onRecord={onRecord} send={send} snapshot={value} />,
  );
  return { ...view, onRecord, send };
}

beforeEach(() => {
  window.localStorage.clear();
  window.location.hash = "#/briefing";
});

afterEach(cleanup);

describe("Hoy, a normal day", () => {
  it("says the date and the week, greets and tells the day in one sentence", () => {
    show(snapshot());
    expect(screen.getByText("Martes 15 de septiembre · semana 38")).toBeTruthy();
    expect(screen.getByRole("heading", { level: 1, name: "Buenas tardes, Lucía" })).toBeTruthy();
    expect(
      screen.getByText("Hoy tienes 3 reuniones y 2 h 45 min libres para concentrarte."),
    ).toBeTruthy();
  });

  it("shows the next meeting: when it starts, who goes and its two actions", async () => {
    const { send } = show(snapshot());
    const card = screen.getByRole("region", { name: "Revisión del diseño de reservas" });
    expect(within(card).getByText("Siguiente · empieza en 20 min")).toBeTruthy();
    expect(within(card).getByText("14:50 – 15:35 · videollamada")).toBeTruthy();
    expect(within(card).getByText("Se acordó simplificar el paso de pago.")).toBeTruthy();
    expect(within(card).getByText("Con Marta Ibáñez, Diego Salinas y Javier Ortega")).toBeTruthy();
    await userEvent.click(within(card).getByRole("button", { name: "Unirse y grabar" }));
    expect(send.mock.calls.map(([command]) => command.command)).toEqual([
      "open_url",
      "meeting_start",
    ]);
    await userEvent.click(within(card).getByRole("button", { name: "Abrir nota anterior" }));
    expect(window.location.hash).toBe("#/notes/rec-earlier");
  });

  it("an event without a video call only records", () => {
    show(snapshot({ briefing: briefing({ next: next({ url: null }), upcoming: undefined }) }));
    expect(screen.getByRole("button", { name: "Solo grabar" })).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Unirse y grabar" })).toBeNull();
  });
});

describe("Pendiente de la última vez", () => {
  const withActions = (actions: BriefingPreviousActions | undefined) =>
    snapshot({
      briefing: briefing({ next: next({ previous_actions: actions }), upcoming: undefined }),
    });

  it("while the series is being read it says so, never «nothing pending»", () => {
    show(withActions({ state: "unknown", cause: "series_not_read" }));
    expect(screen.getByText("Leyendo lo pendiente de la última vez…")).toBeTruthy();
    expect(screen.queryAllByRole("checkbox")).toHaveLength(0);
  });

  it("a read that failed says why", () => {
    show(withActions({ state: "unknown", cause: "La serie de la reunión llegó vacía" }));
    expect(screen.getByText("No se pudo leer lo pendiente de la última vez.")).toBeTruthy();
    expect(screen.getByText("La serie de la reunión llegó vacía")).toBeTruthy();
  });

  it("the first meeting of a series has nothing pending", () => {
    show(withActions({ state: "no_previous_meeting" }));
    expect(screen.getByText("Es la primera de esta serie: no hay nada pendiente.")).toBeTruthy();
  });

  it("a note still being written is not «nothing pending»", () => {
    show(
      withActions({
        state: "note_not_written",
        recording_id: "rec-earlier",
        note_state: "generating",
      }),
    );
    expect(screen.getByText("La nota de la última vez aún se está escribiendo.")).toBeTruthy();
  });

  it("a meeting with no note says its tasks are not known", () => {
    show(withActions({ state: "note_not_written", recording_id: "rec-earlier", note_state: null }));
    expect(
      screen.getByText("La última vez no se escribió nota: no se sabe qué quedó pendiente."),
    ).toBeTruthy();
  });

  it("marks a task done with the note's order and never turns back by itself", async () => {
    const send = vi.fn<(command: AppCommand) => Promise<void>>().mockResolvedValue();
    const value = withActions(known);
    const view = show(value, send);
    const task = screen.getByRole("checkbox", { name: "Marta: prototipo del paso de pago" });
    expect(task.getAttribute("aria-checked")).toBe("false");
    await userEvent.click(task);
    expect(send).toHaveBeenCalledWith({
      command: "note_action_toggle",
      recording_id: "rec-earlier",
      action_id: "a1",
      done: true,
    });
    expect(task.getAttribute("aria-checked")).toBe("true");
    expect(screen.getByText("Guardando…")).toBeTruthy();
    // Another frame arrives before the hub confirms: the mark stays.
    view.rerender(
      <BriefingView error={null} onRecord={view.onRecord} send={send} snapshot={{ ...value }} />,
    );
    expect(task.getAttribute("aria-checked")).toBe("true");
    const done = {
      ...known,
      items: known.items.map((item) => (item.id === "a1" ? { ...item, done: true } : item)),
    };
    view.rerender(
      <BriefingView
        error={null}
        onRecord={view.onRecord}
        send={send}
        snapshot={withActions(done)}
      />,
    );
    expect(task.getAttribute("aria-checked")).toBe("true");
    expect(screen.queryByText("Guardando…")).toBeNull();
  });

  it("a refused mark puts back what the hub has", async () => {
    const send = vi.fn<(command: AppCommand) => Promise<void>>().mockRejectedValue(new Error("no"));
    show(withActions(known), send);
    const task = screen.getByRole("checkbox", { name: "medir la disponibilidad" });
    await userEvent.click(task);
    await vi.waitFor(() => expect(task.getAttribute("aria-checked")).toBe("false"));
  });
});

describe("Recordatorio personal", () => {
  it("stays on this Mac, for this series, and never reaches the daemon", async () => {
    const { send, unmount } = show(snapshot());
    const field = screen.getByRole("textbox", { name: "Recordatorio personal" });
    expect(screen.getByText("Solo lo ves tú, en este Mac. No va a la nota.")).toBeTruthy();
    await userEvent.type(field, "Llevar las cifras del piloto");
    fireEvent.blur(field);
    expect(screen.getByText("✓ Guardado")).toBeTruthy();
    expect(send).not.toHaveBeenCalled();
    unmount();
    show(snapshot());
    expect(
      (screen.getByRole("textbox", { name: "Recordatorio personal" }) as HTMLInputElement).value,
    ).toBe("Llevar las cifras del piloto");
  });
});

describe("Agenda de hoy", () => {
  it("draws the day from 8:00 to 20:00 with the mark of now", () => {
    show(snapshot());
    expect(
      screen.getByRole("img", { name: "Tu día de 8:00 a 20:00. Ahora son las 14:30." }),
    ).toBeTruthy();
    expect(screen.getByRole("link", { name: "Ver la semana" }).getAttribute("href")).toBe(
      "#/calendar",
    );
  });

  it("dims what is past, marks the next and says what each one has", () => {
    show(snapshot());
    const agenda = screen.getByRole("region", { name: "Agenda de hoy" });
    const daily = within(agenda).getByRole("button", { name: /Daily del equipo/ });
    const review = within(agenda).getByRole("button", { name: /Revisión del diseño/ });
    expect(daily.dataset.past).toBe("true");
    expect(review.dataset.next).toBe("true");
    expect(within(daily).getByText("Nota lista")).toBeTruthy();
    expect(within(review).getByText("4 personas")).toBeTruthy();
    expect(within(agenda).getByText("Solo tú")).toBeTruthy();
  });
});

describe("Hoy while recording", () => {
  const recording = {
    ...idle,
    phase: "recording",
    recording_id: "rec-now",
    title: "Revisión del diseño de reservas",
    started_at_ms: now - (12 * MINUTE + 34_000),
  } as MeetingSnapshot;
  const value = () =>
    snapshot({
      meeting: recording,
      briefing: briefing({ next: next({ recording: "recording" }), upcoming: undefined }),
    });

  it("the card records: time, private notes, finish and the shortcuts", async () => {
    const { send } = show(value());
    const card = screen.getByRole("region", { name: "Revisión del diseño de reservas" });
    expect(within(card).getByText("Grabando · 12:34")).toBeTruthy();
    expect(within(card).getByText("Durante la reunión")).toBeTruthy();
    expect(within(card).getByText("«Oye Deimos…»")).toBeTruthy();
    expect(within(card).getByText("Terminar desde cualquier app")).toBeTruthy();
    await userEvent.click(within(card).getByRole("button", { name: "Terminar" }));
    expect(send).toHaveBeenCalledWith({ command: "meeting_finish" });
    await userEvent.click(
      within(card).getByRole("button", { name: "Apuntar en mis notas privadas" }),
    );
    expect(window.location.hash).toBe("#/notes/rec-now");
  });

  it("the agenda says which meeting records", () => {
    show(value());
    const agenda = screen.getByRole("region", { name: "Agenda de hoy" });
    const review = within(agenda).getByRole("button", { name: /Revisión del diseño/ });
    expect(within(review).getByText("Grabando")).toBeTruthy();
    // Grabando.dc.html draws the list without the line of the day.
    expect(screen.queryByRole("img", { name: /Tu día/ })).toBeNull();
  });
});

describe("Hoy without the calendar", () => {
  it("asks macOS for access and waits for its answer", async () => {
    const { send } = show(
      snapshot({
        calendar: {
          authorization: "not_determined",
          reading: { state: "unavailable", blocker: "calendar_permission" },
          stale: false,
        } as AppView["calendar"],
        briefing: briefing({
          next: null,
          upcoming: [],
          agenda: [],
          sources: {
            agenda: { state: "absent", cause: "calendar_permission_denied" },
            notes: { state: "present" },
            recording: { state: "present" },
          },
        }),
      }),
    );
    expect(screen.getByText("Conecta tu calendario")).toBeTruthy();
    expect(screen.queryByText(/no tienes reuniones/)).toBeNull();
    await userEvent.click(screen.getByRole("button", { name: "Permitir acceso al calendario" }));
    expect(send).toHaveBeenCalledWith({
      command: "calendar",
      request: { action: "request_permission" },
    });
    expect(screen.getByRole("button", { name: "Esperando a macOS…" })).toBeTruthy();
  });

  it("once denied, the way back is in System Settings", async () => {
    const { send } = show(
      snapshot({
        calendar: {
          authorization: "denied",
          reading: { state: "unavailable", blocker: "calendar_permission" },
          stale: false,
        } as AppView["calendar"],
        briefing: briefing({
          next: null,
          upcoming: [],
          agenda: [],
          sources: {
            agenda: { state: "absent", cause: "calendar_permission_denied" },
            notes: { state: "present" },
            recording: { state: "present" },
          },
        }),
      }),
    );
    await userEvent.click(screen.getByRole("button", { name: "Abrir Ajustes del Sistema" }));
    expect(send).toHaveBeenCalledWith({ command: "open_system_privacy", pane: "Calendars" });
  });
});

describe("Hoy while it reads", () => {
  it("before the first briefing it shows the shape of what comes, never blank", () => {
    const { container } = show(snapshot({ briefing: undefined }));
    expect(screen.getByRole("status").textContent).toBe("Leyendo tu agenda…");
    expect(container.querySelector("[aria-busy='true']")).not.toBeNull();
  });

  it("an agenda not read yet is never «no meetings»", () => {
    show(
      snapshot({
        calendar: {
          authorization: "granted",
          reading: { state: "unknown" },
          stale: false,
        } as AppView["calendar"],
        briefing: briefing({
          next: null,
          upcoming: [],
          agenda: [],
          summary: { meetings_today: 0, focus_minutes: 0 },
          sources: {
            agenda: { state: "unknown", cause: "topic_not_observed" },
            notes: { state: "present" },
            recording: { state: "present" },
          },
        }),
      }),
    );
    expect(screen.getByRole("status").textContent).toBe("Leyendo tu agenda…");
    expect(screen.queryByText(/no tienes reuniones|0 reuniones/)).toBeNull();
  });
});

describe("Hoy with no meetings", () => {
  it("says so and offers to record", async () => {
    const { onRecord } = show(
      snapshot({
        briefing: briefing({
          next: null,
          upcoming: [],
          agenda: [],
          summary: { meetings_today: 0, focus_minutes: 600 },
        }),
      }),
    );
    expect(screen.getByText("Hoy no tienes reuniones.")).toBeTruthy();
    await userEvent.click(screen.getByRole("button", { name: "Grabar reunión" }));
    expect(onRecord).toHaveBeenCalled();
  });

  it("after the last meeting, none is left", () => {
    show(
      snapshot({
        briefing: briefing({
          next: null,
          upcoming: [],
          agenda: [row("daily", 9, 0, 30, "accent", 5)],
        }),
      }),
    );
    expect(screen.getByText("No te quedan reuniones hoy")).toBeTruthy();
  });
});
