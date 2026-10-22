import { act, cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { AppCommand } from "../../bridge/generated";
import type { WriteOutcome } from "../../bridge/generated/WriteOutcome";
import type { AppView as AppSnapshot } from "../../state/app-view";
import { useUiStore } from "../../state/ui";
import { FIXTURE_WEEK_START_MS } from "../../test-clock";
import CalendarView from "./CalendarView";

// What is written in the event panel stays on this Mac until the calendar
// has it: leaving the view, Orbit closing or falling, and a refusal never
// take it. Events and places are invented.

const HOUR = 3_600_000;
const DAY = 24 * HOUR;
const MONDAY = FIXTURE_WEEK_START_MS + DAY;
const at = (day: number, hours: number) => MONDAY + day * DAY + hours * HOUR;

type Calendar = AppSnapshot["calendar"];

function calendar(change: Partial<Calendar> = {}): Calendar {
  return {
    authorization: "granted",
    calendars: [
      {
        id: "work",
        title: "Trabajo",
        source_id: "local",
        color: "accent",
        can_edit: true,
        is_default: true,
      },
    ],
    window: { start_ms: MONDAY, end_ms: MONDAY + 7 * DAY, timezone: "UTC" },
    range: {
      view: "month",
      days: 35,
      week_start: "monday",
      anchor_ms: at(1, 0),
      start_ms: MONDAY - 14 * DAY,
      end_ms: MONDAY + 21 * DAY,
    },
    events: [],
    reading: { state: "available" },
    stale: false,
    writes: [],
    ...change,
  } as Calendar;
}

function snapshot(change: Partial<Calendar> = {}): AppSnapshot {
  return { calendar: calendar(change), system_timezone: "UTC" } as unknown as AppSnapshot;
}

function setup(View: typeof CalendarView = CalendarView) {
  const send = vi.fn<(command: AppCommand) => Promise<void>>().mockResolvedValue();
  const view = render(<View error={null} send={send} snapshot={snapshot()} />);
  const rerender = (next: Partial<Calendar>) =>
    view.rerender(<View error={null} send={send} snapshot={snapshot(next)} />);
  return { rerender, send, unmount: view.unmount };
}

const panel = () => screen.getByRole("complementary", { name: "Detalles del evento" });
const titleBox = () => within(panel()).getByRole("textbox", { name: "Título" }) as HTMLInputElement;
const notesBox = () =>
  within(panel()).getByRole("textbox", { name: "Notas del evento" }) as HTMLTextAreaElement;

function draftOnSaturday() {
  fireEvent.doubleClick(screen.getByRole("button", { name: /sábado 19 de septiembre/ }));
  fireEvent.change(titleBox(), { target: { value: "Revisión con la gestoría" } });
  fireEvent.change(notesBox(), { target: { value: "Llevar los papeles del alquiler" } });
}

beforeEach(() => {
  window.localStorage.clear();
  useUiStore.setState({ selectedCalendarEventId: null });
});
afterEach(cleanup);

describe("the event panel keeps what was written", () => {
  it("an event typed and not saved yet is back in the panel when the person returns to the calendar", () => {
    const first = setup();
    draftOnSaturday();
    // The person goes to Notas: the calendar view unmounts.
    first.unmount();
    setup();
    expect(titleBox().value).toBe("Revisión con la gestoría");
    expect(notesBox().value).toBe("Llevar los papeles del alquiler");
  });

  it("after Orbit closed or fell, the draft comes back and the panel says so", async () => {
    const first = setup();
    draftOnSaturday();
    first.unmount();
    // A new run of the webview: only this Mac's local storage remains.
    vi.resetModules();
    const reopened = (await import("./CalendarView")).default;
    setup(reopened);
    expect(titleBox().value).toBe("Revisión con la gestoría");
    expect(within(panel()).getByRole("status").textContent).toContain(
      "Recuperado lo que escribiste y no se llegó a guardar",
    );
  });

  it("a new event the calendar refuses comes back to the panel, with its notes", () => {
    const { rerender, send } = setup();
    draftOnSaturday();
    fireEvent.click(within(panel()).getByRole("button", { name: "Crear evento" }));
    const create = send.mock.calls
      .map(([command]) => command)
      .find((command) => command.command === "calendar");
    const requestId =
      create?.command === "calendar" && create.request.action === "save"
        ? create.request.request_id
        : "";
    expect(requestId).not.toBe("");
    const refused: WriteOutcome = {
      request_id: requestId,
      operation: "create",
      stage: "refused",
      code: "calendar_not_editable",
      notice: "El calendario «Trabajo» no admite cambios desde este Mac.",
    };
    act(() => rerender({ writes: [refused] }));
    expect(titleBox().value).toBe("Revisión con la gestoría");
    expect(notesBox().value).toBe("Llevar los papeles del alquiler");
    expect(screen.getByRole("alert").textContent).toContain("El calendario no aplicó el cambio");
  });

  it("a new event the calendar saved is no longer kept as a draft", () => {
    const { rerender, send, unmount } = setup();
    draftOnSaturday();
    fireEvent.click(within(panel()).getByRole("button", { name: "Crear evento" }));
    const create = send.mock.calls
      .map(([command]) => command)
      .find((command) => command.command === "calendar");
    const requestId =
      create?.command === "calendar" && create.request.action === "save"
        ? create.request.request_id
        : "";
    act(() =>
      rerender({ writes: [{ request_id: requestId, operation: "create", stage: "saved" }] }),
    );
    unmount();
    setup();
    expect(screen.queryByRole("complementary", { name: "Detalles del evento" })).toBeNull();
  });

  it("a draft let go of on purpose is not brought back", () => {
    const first = setup();
    draftOnSaturday();
    // ⌫ outside the panel drops a draft without writing it.
    fireEvent.keyDown(window, { key: "Delete" });
    first.unmount();
    setup();
    expect(screen.queryByRole("complementary", { name: "Detalles del evento" })).toBeNull();
  });
});
