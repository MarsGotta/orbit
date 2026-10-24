import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import { Tooltip } from "./Tooltip";

afterEach(cleanup);

function tip(text: string): HTMLElement | null {
  return screen.queryByText(text, { selector: "[data-tooltip]" });
}

it("shows its text while the mouse is over the control, and hides it when it leaves", () => {
  render(
    <Tooltip text="Volver a preguntar">
      <button aria-label="Volver a preguntar" type="button" />
    </Tooltip>,
  );
  const button = screen.getByRole("button", { name: "Volver a preguntar" });
  expect(tip("Volver a preguntar")).toBeNull();
  fireEvent.mouseEnter(button);
  expect(tip("Volver a preguntar")).toBeTruthy();
  fireEvent.mouseLeave(button);
  expect(tip("Volver a preguntar")).toBeNull();
});

it("shows it with the keyboard's focus too, and hides it when the focus leaves", () => {
  render(
    <Tooltip text="Editar y copiar">
      <button aria-label="Editar y copiar" type="button" />
    </Tooltip>,
  );
  const button = screen.getByRole("button", { name: "Editar y copiar" });
  fireEvent.keyDown(document.body, { key: "Tab" });
  fireEvent.focus(button);
  expect(tip("Editar y copiar")).toBeTruthy();
  fireEvent.blur(button);
  expect(tip("Editar y copiar")).toBeNull();
});

it("does not show it for a focus that a press of the mouse gave", () => {
  render(
    <Tooltip text="Ajustes de Phobos">
      <button aria-label="Ajustes de Phobos" type="button" />
    </Tooltip>,
  );
  const button = screen.getByRole("button", { name: "Ajustes de Phobos" });
  fireEvent.pointerDown(button);
  fireEvent.focus(button);
  expect(tip("Ajustes de Phobos")).toBeNull();
});

it("goes above by default and below where it is asked to", () => {
  render(
    <>
      <Tooltip text="Copiar">
        <button aria-label="Copiar" type="button" />
      </Tooltip>
      <Tooltip placement="below" text="Llamar a Phobos">
        <button aria-label="Llamar a Phobos" type="button" />
      </Tooltip>
    </>,
  );
  fireEvent.mouseEnter(screen.getByRole("button", { name: "Copiar" }));
  fireEvent.mouseEnter(screen.getByRole("button", { name: "Llamar a Phobos" }));
  expect(tip("Copiar")?.dataset.placement).toBe("above");
  expect(tip("Llamar a Phobos")?.dataset.placement).toBe("below");
});

it("stays hidden while the menu of its control is open", () => {
  render(
    <Tooltip placement="below" text="Editar nombre y descripción">
      <button
        aria-expanded="true"
        aria-label="Editar nombre y descripción de Phobos"
        type="button"
      />
    </Tooltip>,
  );
  fireEvent.mouseEnter(screen.getByRole("button"));
  expect(tip("Editar nombre y descripción")).toBeNull();
});

it("is not read out: the control carries its own name", () => {
  render(
    <Tooltip text="Copiar">
      <button aria-label="Copiar" type="button" />
    </Tooltip>,
  );
  fireEvent.mouseEnter(screen.getByRole("button"));
  expect(tip("Copiar")?.getAttribute("aria-hidden")).toBe("true");
});

it("stays inside the window at its edges, whatever box its control is in", () => {
  // jsdom lays nothing out: the control sits in the window's top right
  // corner, inside a box that clips, and the tip measures 120 × 24.
  const right = window.innerWidth - 8;
  const rect = (left: number, top: number, width: number, height: number) =>
    ({
      left,
      top,
      width,
      height,
      right: left + width,
      bottom: top + height,
      x: left,
      y: top,
    }) as DOMRect;
  const measured = vi
    .spyOn(HTMLElement.prototype, "getBoundingClientRect")
    .mockImplementation(function (this: HTMLElement) {
      return this.hasAttribute("data-tooltip") ? rect(0, 0, 120, 24) : rect(right - 24, 4, 24, 24);
    });
  render(
    <div data-testid="clipping" style={{ overflow: "hidden" }}>
      <Tooltip text="Añadir asistente">
        <button aria-label="Añadir asistente" type="button" />
      </Tooltip>
    </div>,
  );
  fireEvent.mouseEnter(screen.getByRole("button"));
  const shown = tip("Añadir asistente") as HTMLElement;
  expect(shown.dataset.placement).toBe("below");
  expect(parseFloat(shown.style.top)).toBeGreaterThanOrEqual(28);
  expect(parseFloat(shown.style.left) + 120).toBeLessThanOrEqual(window.innerWidth - 8);
  expect(screen.getByTestId("clipping").contains(shown)).toBe(false);
  measured.mockRestore();
});
