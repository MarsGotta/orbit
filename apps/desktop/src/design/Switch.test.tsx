import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import { Switch } from "./Switch";

afterEach(cleanup);

it("is a switch whose visible label also presses it", () => {
  const onCheckedChange = vi.fn();
  render(
    <Switch
      checked={false}
      label="Abrir Orbit al encender el Mac"
      onCheckedChange={onCheckedChange}
    >
      Abrir Orbit al encender el Mac
    </Switch>,
  );
  fireEvent.click(screen.getByText("Abrir Orbit al encender el Mac"));
  expect(onCheckedChange).toHaveBeenCalledWith(true);
  expect(screen.getByRole("switch", { name: "Abrir Orbit al encender el Mac" })).toBeTruthy();
});

it("when it waits for the system it stays in place, says so, and moves once the state changes", () => {
  const onCheckedChange = vi.fn();
  const view = render(
    <Switch
      checked={false}
      label="Abrir Orbit al encender el Mac"
      onCheckedChange={onCheckedChange}
      waitsForSystem
    />,
  );
  const control = screen.getByRole("switch", { name: "Abrir Orbit al encender el Mac" });
  fireEvent.click(control);
  expect(control.getAttribute("aria-checked")).toBe("false");
  expect(screen.getByText("Esperando a macOS…")).toBeTruthy();
  fireEvent.click(control);
  expect(onCheckedChange).toHaveBeenCalledOnce();
  view.rerender(
    <Switch
      checked
      label="Abrir Orbit al encender el Mac"
      onCheckedChange={onCheckedChange}
      waitsForSystem
    />,
  );
  expect(control.getAttribute("aria-checked")).toBe("true");
  expect(screen.queryByText("Esperando a macOS…")).toBeNull();
});

it("stops waiting when the request is refused", async () => {
  let refuse: (error: Error) => void = () => undefined;
  const request = new Promise<void>((_, fail) => (refuse = fail));
  render(
    <Switch
      checked={false}
      label="Micrófono siempre listo"
      onCheckedChange={() => request}
      waitsForSystem
    />,
  );
  fireEvent.click(screen.getByRole("switch"));
  expect(screen.getByText("Esperando a macOS…")).toBeTruthy();
  await act(async () => refuse(new Error("rechazado")));
  expect(screen.queryByText("Esperando a macOS…")).toBeNull();
  expect(screen.getByRole("switch").getAttribute("aria-checked")).toBe("false");
});
