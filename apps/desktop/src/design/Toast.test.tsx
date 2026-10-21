import { act, cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import { Toast, ToastStack } from "./Toast";
import toastCss from "./Toast.module.css?raw";
import styles from "./Toast.module.css";

afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

it("el deshacer real se ofrece durante diez segundos y se ejecuta una vez", () => {
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

it("un aviso con enlace dura diez segundos y uno sin enlace, cinco (Alina, 2026-10-20)", () => {
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

it("al caducar el aviso cierra sin ejecutar el deshacer", () => {
  vi.useFakeTimers();
  const onUndo = vi.fn();
  const onClose = vi.fn();
  render(<Toast message="Plantilla borrada" onClose={onClose} onUndo={onUndo} />);
  act(() => vi.advanceTimersByTime(10_000));
  expect(screen.queryByRole("status")).toBeNull();
  expect(onUndo).not.toHaveBeenCalled();
  expect(onClose).toHaveBeenCalledOnce();
});

it("un aviso sin deshacer se puede cerrar antes de caducar", () => {
  const onClose = vi.fn();
  render(<Toast message="Llamada terminada" onClose={onClose} />);
  fireEvent.click(screen.getByRole("button", { name: "Cerrar aviso" }));
  expect(onClose).toHaveBeenCalledOnce();
  expect(screen.queryByText("Llamada terminada")).toBeNull();
});

it("todos los emisores comparten una esquina flotante con tres avisos visibles", () => {
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
  if (!first?.parentElement) throw new Error("El aviso debe estar en la pila");
  const stack = first.parentElement;
  expect(getComputedStyle(stack).position).toBe("fixed");
  fireEvent.click(within(first as HTMLElement).getByRole("button", { name: "Cerrar aviso" }));
  expect(Array.from(stack.children, (card) => card.textContent)).toEqual([
    "dos×",
    "tres×",
    "cuatro×",
  ]);
});

it("los renders con otro callback no aplazan la caducidad y llaman al actual", () => {
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

it("los problemas y el trabajo en curso no caducan", () => {
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
