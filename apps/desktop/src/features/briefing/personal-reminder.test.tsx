import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import { PersonalReminder } from "./PersonalReminder";

// «Recordatorio personal» lives in this Mac's webview only. It is keyed by
// the calendar item, which a rename in Calendar does not change; the
// series key is made of the title. Names and events are invented.

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

function reminder(seriesKey: string, calendarItemId: string | null = ITEM) {
  return <PersonalReminder calendarItemId={calendarItemId} seriesKey={seriesKey} />;
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
