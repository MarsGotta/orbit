import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import { TEST_NOW_MS } from "../../../test-clock";
import { dayOf, startOfDay } from "../date";
import type { CalendarEvent } from "../eventLook";
import { atWall } from "./geometry";
import { useGridGestures, type GridEdit } from "./useGridGestures";
import { useLaneGestures } from "./useLaneGestures";

const DAY = dayOf(TEST_NOW_MS, "UTC");
const at = (hour: number) => atWall(DAY, hour * 60, "UTC");
const event: CalendarEvent = { id: "event", title: "Reunión", start_ms: at(9), end_ms: at(10) };

function edits(): GridEdit {
  return {
    editable: vi.fn(() => true),
    onBlocked: vi.fn(),
    onChange: vi.fn(),
    onCreate: vi.fn(),
    onDeselect: vi.fn(() => false),
  };
}

function Grid({ edit, events = [] }: { edit: GridEdit; events?: CalendarEvent[] }) {
  const gesture = useGridGestures(edit, events, 60, "UTC");
  return (
    <div data-testid="grid" onMouseDown={gesture.onMouseDown} ref={gesture.bodyRef}>
      <div data-day={DAY} data-testid="column">
        <div data-event-id="event" data-testid="card">
          <span data-resize="end">Estirar</span>
        </div>
      </div>
      <output>
        {gesture.preview ? JSON.stringify(gesture.preview.span) : "Sin previsualización"}
      </output>
    </div>
  );
}

function Lane({ edit, events = [] }: { edit: GridEdit; events?: CalendarEvent[] }) {
  const gesture = useLaneGestures(edit, events, [DAY, DAY + 1, DAY + 2], "UTC");
  return (
    <div data-testid="lane" onMouseDown={gesture.onMouseDown} ref={gesture.laneRef}>
      <div data-event-id="event" data-testid="card">
        <span data-resize="start">Inicio</span>
        <span data-resize="end">Fin</span>
      </div>
      <output>
        {gesture.preview ? JSON.stringify(gesture.preview.span) : "Sin previsualización"}
      </output>
    </div>
  );
}

function box(element: HTMLElement, width: number) {
  vi.spyOn(element, "getBoundingClientRect").mockReturnValue({
    left: 0,
    right: width,
    top: 0,
    width,
    height: 1440,
    bottom: 1440,
    x: 0,
    y: 0,
    toJSON: () => ({}),
  });
}

// Release before cleanup so a failing assertion never leaves a window listener behind.
afterEach(() => {
  fireEvent.mouseUp(window);
  fireEvent.click(window);
  cleanup();
  vi.restoreAllMocks();
});

it("la rejilla distingue clic y arrastre, muestra el tramo y crea sólo al soltar", () => {
  const edit = edits();
  render(<Grid edit={edit} />);
  const grid = screen.getByTestId("grid");
  box(grid, 100);
  box(screen.getByTestId("column"), 100);
  fireEvent.mouseDown(grid, { clientX: 20, clientY: 540, button: 0 });
  fireEvent.mouseMove(window, { clientX: 22, clientY: 542 });
  expect(screen.getByText("Sin previsualización")).toBeTruthy();
  fireEvent.mouseUp(window, { clientX: 22, clientY: 542 });
  expect(edit.onCreate).toHaveBeenLastCalledWith({ startMs: at(9), endMs: at(9.5) });
  fireEvent.mouseDown(grid, { clientX: 20, clientY: 540, button: 0 });
  fireEvent.mouseMove(window, { clientX: 20, clientY: 660 });
  expect(JSON.parse(screen.getByRole("status").textContent ?? "null")).toEqual({
    startMs: at(9),
    endMs: at(11),
  });
  expect(edit.onCreate).toHaveBeenCalledTimes(1);
  fireEvent.mouseUp(window, { clientX: 20, clientY: 660 });
  expect(edit.onCreate).toHaveBeenLastCalledWith({ startMs: at(9), endMs: at(11) });
  expect(screen.getByText("Sin previsualización")).toBeTruthy();
  const click = new MouseEvent("click", { bubbles: true, cancelable: true });
  grid.dispatchEvent(click);
  expect(click.defaultPrevented).toBe(true);
  fireEvent.mouseMove(window, { clientX: 20, clientY: 720 });
  expect(edit.onCreate).toHaveBeenCalledTimes(2);
});

it("la rejilla mueve, estira y conserva la negativa de un evento no editable", () => {
  const edit = edits();
  render(<Grid edit={edit} events={[event]} />);
  box(screen.getByTestId("grid"), 100);
  box(screen.getByTestId("column"), 100);
  const card = screen.getByTestId("card");
  fireEvent.mouseDown(card, { clientX: 20, clientY: 540, button: 0 });
  fireEvent.mouseMove(window, { clientX: 20, clientY: 600 });
  fireEvent.mouseUp(window, { clientX: 20, clientY: 600 });
  expect(edit.onChange).toHaveBeenLastCalledWith(event, { startMs: at(10), endMs: at(11) });
  fireEvent.mouseDown(screen.getByText("Estirar"), { clientX: 20, clientY: 600, button: 0 });
  fireEvent.mouseMove(window, { clientX: 20, clientY: 660 });
  fireEvent.mouseUp(window, { clientX: 20, clientY: 660 });
  expect(edit.onChange).toHaveBeenLastCalledWith(event, { startMs: at(9), endMs: at(11) });
  vi.mocked(edit.editable).mockReturnValue(false);
  fireEvent.mouseDown(card, { clientX: 20, clientY: 540, button: 0 });
  fireEvent.mouseMove(window, { clientX: 20, clientY: 600 });
  expect(screen.getByText("Sin previsualización")).toBeTruthy();
  fireEvent.mouseUp(window, { clientX: 20, clientY: 600 });
  expect(edit.onBlocked).toHaveBeenCalledWith(event);
  expect(edit.onChange).toHaveBeenCalledTimes(2);
});

it("el carril ignora el movimiento vertical y arrastra por días completos", () => {
  const edit = edits();
  render(<Lane edit={edit} />);
  const lane = screen.getByTestId("lane");
  box(lane, 300);
  fireEvent.mouseDown(lane, { clientX: 20, clientY: 10, button: 0 });
  fireEvent.mouseMove(window, { clientX: 22, clientY: 200 });
  expect(screen.getByText("Sin previsualización")).toBeTruthy();
  fireEvent.mouseUp(window, { clientX: 22, clientY: 200 });
  expect(edit.onCreate).toHaveBeenLastCalledWith({
    startMs: startOfDay(DAY, "UTC"),
    endMs: startOfDay(DAY + 1, "UTC") - 1_000,
  });
  fireEvent.mouseDown(lane, { clientX: 20, clientY: 10, button: 0 });
  fireEvent.mouseMove(window, { clientX: 250, clientY: 10 });
  expect(edit.onCreate).toHaveBeenCalledTimes(1);
  fireEvent.mouseUp(window, { clientX: 250, clientY: 10 });
  expect(edit.onCreate).toHaveBeenLastCalledWith({
    startMs: startOfDay(DAY, "UTC"),
    endMs: startOfDay(DAY + 3, "UTC") - 1_000,
  });
});

it.each(["Inicio", "Fin", "mover"])(
  "el carril permite %s de un evento y respeta el bloqueo",
  (action) => {
    const edit = edits();
    const allDay = {
      ...event,
      all_day: true,
      start_ms: startOfDay(DAY, "UTC"),
      end_ms: startOfDay(DAY + 2, "UTC") - 1_000,
    };
    render(<Lane edit={edit} events={[allDay]} />);
    box(screen.getByTestId("lane"), 300);
    const target = action === "mover" ? screen.getByTestId("card") : screen.getByText(action);
    const toX = action === "Fin" ? 250 : 120;
    fireEvent.mouseDown(target, { clientX: 20, button: 0 });
    fireEvent.mouseMove(window, { clientX: toX });
    fireEvent.mouseUp(window, { clientX: toX });
    expect(edit.onChange).toHaveBeenCalledWith(allDay, {
      startMs: startOfDay(action === "Fin" ? DAY : DAY + 1, "UTC"),
      endMs: startOfDay(action === "Inicio" ? DAY + 2 : DAY + 3, "UTC") - 1_000,
    });
    vi.mocked(edit.editable).mockReturnValue(false);
    fireEvent.mouseDown(target, { clientX: 20, button: 0 });
    fireEvent.mouseMove(window, { clientX: toX });
    fireEvent.mouseUp(window, { clientX: toX });
    expect(edit.onBlocked).toHaveBeenCalledWith(allDay);
    expect(edit.onChange).toHaveBeenCalledTimes(1);
  },
);

it.each([
  ["la rejilla", Grid],
  ["el carril", Lane],
] as const)("un clic libre con selección sólo deselecciona en %s", (_name, Surface) => {
  const edit = edits();
  vi.mocked(edit.onDeselect).mockReturnValue(true);
  const { container } = render(<Surface edit={edit} />);
  const body = container.firstElementChild as HTMLElement;
  box(body, 300);
  if (Surface === Grid) box(screen.getByTestId("column"), 100);
  fireEvent.mouseDown(body, { clientX: 20, clientY: 540, button: 0 });
  fireEvent.mouseUp(window, { clientX: 20, clientY: 540 });
  expect(edit.onDeselect).toHaveBeenCalledOnce();
  expect(edit.onCreate).not.toHaveBeenCalled();
});
