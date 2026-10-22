import { act, cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import { Toast, ToastStack } from "./Toast";
import toastCss from "./Toast.module.css?raw";
import styles from "./Toast.module.css";

afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

it("the real undo is offered for ten seconds and runs once", () => {
  vi.useFakeTimers();
  const onUndo = vi.fn();
  const onClose = vi.fn();
  render(<Toast message="Evento borrado" onClose={onClose} onUndo={onUndo} />);
  act(() => vi.advanceTimersByTime(9_000));
  fireEvent.click(screen.getByRole("button", { name: "Deshacer 1s" }));
  expect(onUndo).toHaveBeenCalledOnce();
  expect(onClose).toHaveBeenCalledOnce();
  expect(screen.queryByText("Evento borrado")).toBeNull();
  act(() => vi.advanceTimersByTime(1_000));
  expect(onUndo).toHaveBeenCalledOnce();
  expect(onClose).toHaveBeenCalledOnce();
});

it("a notice with a link lasts ten seconds and one without a link five (reported on 2026-10-20)", () => {
  vi.useFakeTimers();
  const withLink = vi.fn();
  const plain = vi.fn();
  render(
    <>
      <Toast
        action={<a href="#/notes/uno">Abrir nota</a>}
        message="Reunión guardada"
        onClose={withLink}
      />
      <Toast message="Llamada terminada" onClose={plain} />
    </>,
  );
  act(() => vi.advanceTimersByTime(4_999));
  expect(plain).not.toHaveBeenCalled();
  act(() => vi.advanceTimersByTime(1));
  expect(plain).toHaveBeenCalledOnce();
  expect(withLink).not.toHaveBeenCalled();
  expect(screen.getByRole("link", { name: "Abrir nota" })).toBeTruthy();
  act(() => vi.advanceTimersByTime(4_999));
  expect(withLink).not.toHaveBeenCalled();
  expect(screen.getByRole("link", { name: "Abrir nota" })).toBeTruthy();
  act(() => vi.advanceTimersByTime(1));
  expect(withLink).toHaveBeenCalledOnce();
  expect(screen.queryByRole("link", { name: "Abrir nota" })).toBeNull();
});

it("on expiry the notice closes without running the undo", () => {
  vi.useFakeTimers();
  const onUndo = vi.fn();
  const onClose = vi.fn();
  render(<Toast message="Plantilla borrada" onClose={onClose} onUndo={onUndo} />);
  act(() => vi.advanceTimersByTime(10_000));
  expect(screen.queryByRole("status")).toBeNull();
  expect(onUndo).not.toHaveBeenCalled();
  expect(onClose).toHaveBeenCalledOnce();
});

it("a notice without undo can be closed before it expires", () => {
  const onClose = vi.fn();
  render(<Toast message="Llamada terminada" onClose={onClose} />);
  fireEvent.click(screen.getByRole("button", { name: "Cerrar aviso" }));
  expect(onClose).toHaveBeenCalledOnce();
  expect(screen.queryByText("Llamada terminada")).toBeNull();
});

it("every emitter shares one floating corner with three visible notices", () => {
  const onClose = vi.fn();
  // jsdom does not install CSS modules: apply the real rules with Vite's names.
  const css = Object.entries(styles).reduce(
    (sheet, [name, scoped]) => sheet.replaceAll(`.${name}`, `.${scoped}`),
    toastCss,
  );
  render(
    <>
      <style>{css}</style>
      <ToastStack />
      {["uno", "dos", "tres", "cuatro"].map((message) => (
        <Toast key={message} kind="failed" message={message} onClose={onClose} />
      ))}
    </>,
  );
  expect(screen.getAllByRole("alert")).toHaveLength(3);
  const first = screen.getByText("uno").closest("[role=alert]");
  if (!first?.parentElement) throw new Error("The notice must be in the stack");
  const stack = first.parentElement;
  expect(getComputedStyle(stack).position).toBe("fixed");
  fireEvent.click(within(first as HTMLElement).getByRole("button", { name: "Cerrar aviso" }));
  expect(Array.from(stack.children, (card) => card.textContent)).toEqual([
    "dos×",
    "tres×",
    "cuatro×",
  ]);
});

it("renders with another callback do not postpone the expiry and call the current one", () => {
  vi.useFakeTimers();
  const oldClose = vi.fn();
  const currentClose = vi.fn();
  const view = render(<Toast message="Hecho" onClose={oldClose} />);
  act(() => vi.advanceTimersByTime(4_000));
  view.rerender(<Toast message="Hecho" onClose={currentClose} />);
  act(() => vi.advanceTimersByTime(1_000));
  expect(currentClose).toHaveBeenCalledOnce();
  expect(oldClose).not.toHaveBeenCalled();
  expect(screen.queryByRole("status")).toBeNull();
});

it("problems and work in progress do not expire", () => {
  vi.useFakeTimers();
  const onClose = vi.fn();
  render(
    <>
      {(["fixable", "failed", "working"] as const).map((kind) => (
        <Toast key={kind} kind={kind} message={kind} onClose={onClose} />
      ))}
    </>,
  );
  act(() => vi.advanceTimersByTime(60_000));
  expect(onClose).not.toHaveBeenCalled();
  for (const kind of ["fixable", "failed", "working"]) {
    expect(screen.getByText(kind)).toBeTruthy();
  }
});

it("a notice waiting for a place in the corner starts its time only once it shows", () => {
  vi.useFakeTimers();
  const closed: string[] = [];
  render(
    <>
      <ToastStack />
      {["uno", "dos", "tres", "cuatro"].map((message) => (
        <Toast key={message} message={message} onClose={() => closed.push(message)} />
      ))}
    </>,
  );
  expect(screen.getAllByRole("status")).toHaveLength(3);
  expect(screen.queryByText("cuatro")).toBeNull();
  act(() => vi.advanceTimersByTime(5_000));
  expect(closed).toEqual(["uno", "dos", "tres"]);
  expect(screen.getByText("cuatro")).toBeTruthy();
  act(() => vi.advanceTimersByTime(4_999));
  expect(screen.getByText("cuatro")).toBeTruthy();
  act(() => vi.advanceTimersByTime(1));
  expect(closed).toEqual(["uno", "dos", "tres", "cuatro"]);
});

it("each class carries its signal, and a failure is an alert", () => {
  render(
    <>
      <Toast kind="working" message="Transcribiendo tu nota de voz…" onClose={() => undefined} />
      <Toast
        detail="El hub no contestó. Tu texto se queda aquí."
        kind="failed"
        message="No se pudo guardar la nota"
        onClose={() => undefined}
      />
    </>,
  );
  const working = screen.getByRole("status");
  expect(working.querySelector("[data-signal=working]")).not.toBeNull();
  const failed = screen.getByRole("alert");
  expect(failed.querySelector("[data-signal=failed]")).not.toBeNull();
  expect(within(failed).getByText("El hub no contestó. Tu texto se queda aquí.")).toBeTruthy();
});
