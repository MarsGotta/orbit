import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { AppCommand } from "../../bridge/generated";
import type { BriefingReminder } from "../../bridge/generated/BriefingReminder";
import { PersonalReminder } from "./PersonalReminder";

// «Recordatorio personal»: this Mac's webview keeps its copy, keyed by the
// calendar item, which a rename in Calendar does not change (the series key
// is made of the title); the hub keeps it for every Mac of its owner, and the
// latest edit wins. Names and events are invented.

const ITEM = "item-revision-semanal";
const original = window.localStorage;

afterEach(() => {
  cleanup();
  Object.defineProperty(window, "localStorage", { configurable: true, value: original });
  original.clear();
});

function field(): HTMLInputElement {
  return screen.getByLabelText("Recordatorio personal") as HTMLInputElement;
}

function write(text: string) {
  fireEvent.change(field(), { target: { value: text } });
  fireEvent.submit(field().closest("form") as HTMLFormElement);
}

type Send = (command: AppCommand) => Promise<unknown>;

function reminder(
  seriesKey: string,
  calendarItemId: string | null = ITEM,
  hub?: BriefingReminder,
  send: Send = () => Promise.resolve(),
  online = true,
) {
  return (
    <PersonalReminder
      calendarItemId={calendarItemId}
      hub={hub}
      online={online}
      send={send}
      seriesKey={seriesKey}
    />
  );
}

function saves(send: ReturnType<typeof vi.fn>) {
  return send.mock.calls
    .map(([command]) => command as AppCommand)
    .filter((command) => command.command === "reminder_save");
}

describe("the personal reminder", () => {
  it("a reminder the Mac could not keep says that it was not saved, and why", () => {
    const full: Storage = {
      length: 0,
      clear: () => undefined,
      getItem: () => null,
      key: () => null,
      removeItem: () => undefined,
      setItem: () => {
        throw new DOMException("The quota has been exceeded.", "QuotaExceededError");
      },
    };
    Object.defineProperty(window, "localStorage", { configurable: true, value: full });
    render(reminder("revisión semanal"));
    write("Llevar las cifras del trimestre");
    expect(screen.queryByText("✓ Guardado")).toBeNull();
    expect(screen.getByRole("alert").textContent).toContain("No se guardó");
    expect(screen.getByRole("alert").textContent).toContain("no queda espacio");
    // What was written stays on screen.
    expect(field().value).toBe("Llevar las cifras del trimestre");
  });

  it("a saved reminder says so", () => {
    render(reminder("revisión semanal"));
    write("Llevar las cifras del trimestre");
    expect(screen.getByText("✓ Guardado")).toBeTruthy();
    expect(screen.queryByRole("alert")).toBeNull();
  });

  it("a reminder typed without pressing Enter is still there after leaving the briefing", () => {
    const first = render(reminder("revisión semanal"));
    fireEvent.change(field(), { target: { value: "Preguntar por el presupuesto" } });
    first.unmount();
    render(reminder("revisión semanal"));
    expect(field().value).toBe("Preguntar por el presupuesto");
  });

  it("a reminder is still shown after its meeting is renamed in the calendar", () => {
    const view = render(reminder("revisión semanal"));
    write("Llevar las cifras del trimestre");
    // The same calendar item, its title changed in Calendar.
    view.rerender(reminder("revisión semanal q"));
    expect(field().value).toBe("Llevar las cifras del trimestre");
  });

  it("a reminder written before it was keyed by the calendar item is still found", () => {
    window.localStorage.setItem(
      "orbit.briefing.context.local.revisión semanal",
      "Traer el informe de ventas",
    );
    render(reminder("revisión semanal"));
    expect(field().value).toBe("Traer el informe de ventas");
  });

  it("an event without a calendar item keeps its reminder by its series", () => {
    const view = render(reminder("revisión semanal", null));
    write("Reservar la sala");
    view.unmount();
    render(reminder("revisión semanal", null));
    expect(field().value).toBe("Reservar la sala");
  });
});

describe("the personal reminder on every Mac", () => {
  it("asks the hub for it when shown and when the hub comes back", () => {
    const send = vi.fn<Send>().mockResolvedValue(undefined);
    const view = render(reminder("revisión semanal", ITEM, undefined, send, false));
    view.rerender(reminder("revisión semanal", ITEM, undefined, send, true));
    const reads = send.mock.calls.filter(([command]) => command.command === "reminder_read");
    expect(reads.map(([command]) => command)).toEqual([
      { command: "reminder_read", series_key: "revisión semanal" },
      { command: "reminder_read", series_key: "revisión semanal" },
    ]);
  });

  it("shows what was written on the other Mac when it is newer", () => {
    window.localStorage.setItem(`orbit.briefing.reminder.item.${ITEM}`, "Lo de este Mac");
    window.localStorage.setItem(`orbit.briefing.reminder.edited.item.${ITEM}`, "1000");
    const send = vi.fn<Send>().mockResolvedValue(undefined);
    const hub: BriefingReminder = { state: "known", text: "Lo del otro Mac", edited_at_ms: 2_000 };
    const view = render(reminder("revisión semanal", ITEM, hub, send));
    expect(field().value).toBe("Lo del otro Mac");
    expect(saves(send)).toEqual([]);
    // And keeps it here too, for when the hub cannot be reached.
    view.unmount();
    render(reminder("revisión semanal", ITEM, { state: "unknown", cause: "sin hub" }));
    expect(field().value).toBe("Lo del otro Mac");
  });

  it("sends what this Mac wrote while the hub was away, and keeps it", () => {
    window.localStorage.setItem(`orbit.briefing.reminder.item.${ITEM}`, "Escrito sin conexión");
    window.localStorage.setItem(`orbit.briefing.reminder.edited.item.${ITEM}`, "3000");
    const send = vi.fn<Send>().mockResolvedValue(undefined);
    const older: BriefingReminder = { state: "known", text: "Anterior", edited_at_ms: 2_000 };
    render(reminder("revisión semanal", ITEM, older, send));
    expect(field().value).toBe("Escrito sin conexión");
    expect(saves(send)).toEqual([
      {
        command: "reminder_save",
        series_key: "revisión semanal",
        text: "Escrito sin conexión",
        edited_at_ms: 3_000,
      },
    ]);
  });

  it("a hub without it keeps this Mac's copy and is not written to again and again", () => {
    window.localStorage.setItem(`orbit.briefing.reminder.item.${ITEM}`, "Solo aquí");
    const send = vi.fn<Send>().mockResolvedValue(undefined);
    const view = render(
      reminder("revisión semanal", ITEM, { state: "unknown", cause: "HTTP 404" }, send),
    );
    expect(field().value).toBe("Solo aquí");
    expect(saves(send)).toEqual([]);
    // The hub has none: the copy written before it existed goes up once.
    view.rerender(reminder("revisión semanal", ITEM, { state: "none" }, send));
    view.rerender(reminder("revisión semanal", ITEM, { state: "none" }, send));
    expect(saves(send)).toHaveLength(1);
  });

  it("an edit goes to the hub with when it was written", () => {
    vi.useFakeTimers({ now: 5_000, toFake: ["Date"] });
    const send = vi.fn<Send>().mockResolvedValue(undefined);
    render(reminder("revisión semanal", ITEM, { state: "none" }, send));
    write("Reservar la sala");
    vi.useRealTimers();
    expect(saves(send)).toEqual([
      {
        command: "reminder_save",
        series_key: "revisión semanal",
        text: "Reservar la sala",
        edited_at_ms: 5_000,
      },
    ]);
    expect(screen.getByText("✓ Guardado")).toBeTruthy();
  });
});
