import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import { Checkbox } from "./Checkbox";

afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

it("shows the new state at once with «Guardando…» and never turns back by itself", () => {
  vi.useFakeTimers();
  const onCheckedChange = vi.fn();
  render(
    <Checkbox checked={false} onCheckedChange={onCheckedChange}>
      Enviar el resumen a Sara
    </Checkbox>,
  );
  const box = screen.getByRole("checkbox", { name: "Enviar el resumen a Sara" });
  fireEvent.click(box);
  expect(onCheckedChange).toHaveBeenCalledWith(true);
  expect(box.getAttribute("aria-checked")).toBe("true");
  expect(screen.getByText("Guardando…")).toBeTruthy();
  act(() => vi.advanceTimersByTime(60_000));
  expect(box.getAttribute("aria-checked")).toBe("true");
  expect(screen.getByText("Guardando…")).toBeTruthy();
});

it("stops saying «Guardando…» when the hub confirms", () => {
  const view = render(
    <Checkbox checked={false} onCheckedChange={() => undefined}>
      Enviar el resumen a Sara
    </Checkbox>,
  );
  fireEvent.click(screen.getByRole("checkbox"));
  view.rerender(
    <Checkbox checked onCheckedChange={() => undefined}>
      Enviar el resumen a Sara
    </Checkbox>,
  );
  expect(screen.getByRole("checkbox").getAttribute("aria-checked")).toBe("true");
  expect(screen.queryByText("Guardando…")).toBeNull();
});

it("goes back to what the hub has when the change is refused", async () => {
  let refuse: (error: Error) => void = () => undefined;
  const request = new Promise<void>((_, fail) => (refuse = fail));
  render(
    <Checkbox checked={false} onCheckedChange={() => request}>
      Enviar el resumen a Sara
    </Checkbox>,
  );
  fireEvent.click(screen.getByRole("checkbox"));
  await act(async () => refuse(new Error("rechazado")));
  expect(screen.getByRole("checkbox").getAttribute("aria-checked")).toBe("false");
  expect(screen.queryByText("Guardando…")).toBeNull();
});
