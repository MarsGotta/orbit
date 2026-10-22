import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import { Button } from "./Button";
import { useFeedback } from "./feedback";

afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

function SaveButton({ save }: { save: () => Promise<void> }) {
  const { feedback, track } = useFeedback();
  return (
    <Button feedback={feedback} onClick={() => track(save())}>
      Guardar
    </Button>
  );
}

function deferred() {
  let resolve: () => void = () => undefined;
  let reject: (error: Error) => void = () => undefined;
  const promise = new Promise<void>((done, fail) => {
    resolve = done;
    reject = fail;
  });
  return { promise, resolve, reject };
}

it("says «Guardando…» at once, «✓ Guardado» when it is confirmed, and goes back after 1.8 s", async () => {
  vi.useFakeTimers();
  const pending = deferred();
  render(<SaveButton save={() => pending.promise} />);
  fireEvent.click(screen.getByRole("button", { name: "Guardar" }));
  const button = screen.getByRole("button", { name: "Guardando…" });
  expect(button.getAttribute("aria-busy")).toBe("true");
  await act(async () => pending.resolve());
  expect(screen.getByRole("button", { name: "✓ Guardado" })).toBe(button);
  act(() => vi.advanceTimersByTime(1_799));
  expect(screen.getByRole("button", { name: "✓ Guardado" })).toBeTruthy();
  act(() => vi.advanceTimersByTime(1));
  expect(screen.getByRole("button", { name: "Guardar" })).toBe(button);
});

it("a second press while it saves does not run the action again", () => {
  const save = vi.fn(() => new Promise<void>(() => undefined));
  render(<SaveButton save={save} />);
  fireEvent.click(screen.getByRole("button", { name: "Guardar" }));
  fireEvent.click(screen.getByRole("button", { name: "Guardando…" }));
  expect(save).toHaveBeenCalledOnce();
});

it("when the action fails it goes back to its label instead of confirming", async () => {
  const pending = deferred();
  render(<SaveButton save={() => pending.promise} />);
  fireEvent.click(screen.getByRole("button", { name: "Guardar" }));
  await act(async () => pending.reject(new Error("rechazado")));
  expect(screen.getByRole("button", { name: "Guardar" })).toBeTruthy();
  expect(screen.queryByRole("button", { name: "✓ Guardado" })).toBeNull();
});

it("all three labels share one place, so the button keeps its width", () => {
  render(<Button feedback="pending">Guardar</Button>);
  const button = screen.getByRole("button", { name: "Guardando…" });
  expect(button.textContent).toContain("Guardar");
  expect(button.textContent).toContain("✓ Guardado");
});

it("a disabled button says why beside it", () => {
  const onClick = vi.fn();
  render(
    <Button disabledReason="Deimos está respondiendo · tu texto se queda" onClick={onClick}>
      Enviar
    </Button>,
  );
  const button = screen.getByRole("button", { name: "Enviar" }) as HTMLButtonElement;
  expect(button.disabled).toBe(true);
  expect(screen.getByText("Deimos está respondiendo · tu texto se queda")).toBeTruthy();
  const reason = document.getElementById(button.getAttribute("aria-describedby") ?? "");
  expect(reason?.textContent).toBe("Deimos está respondiendo · tu texto se queda");
  fireEvent.click(button);
  expect(onClick).not.toHaveBeenCalled();
});
