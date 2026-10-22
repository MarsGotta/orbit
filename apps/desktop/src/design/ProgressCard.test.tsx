import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import { BeingRedone, ProgressCard } from "./ProgressCard";

afterEach(cleanup);

it("says what it is doing and how long is left, with a bar of how far it has gone", () => {
  render(
    <ProgressCard percent={38} remaining="unos 3 min">
      Volviendo a transcribir la reunión…
    </ProgressCard>,
  );
  const card = screen.getByRole("status");
  expect(card.textContent).toContain("Volviendo a transcribir la reunión…");
  expect(card.textContent).toContain("unos 3 min");
  const bar = card.querySelector<HTMLElement>("[data-progress]");
  expect(bar?.style.width).toBe("38%");
});

it("keeps the bar within 0 and 100 whatever it is told", () => {
  render(<ProgressCard percent={140}>Volviendo a transcribir la reunión…</ProgressCard>);
  expect(
    screen.getByRole("status").querySelector<HTMLElement>("[data-progress]")?.style.width,
  ).toBe("100%");
});

it("in the queue, before any progress, it has no bar", () => {
  render(<ProgressCard>En cola para volver a transcribir.</ProgressCard>);
  expect(screen.getByRole("status").querySelector("[data-progress]")).toBeNull();
});

it("when it fails it turns to a warning that stays until it is closed", () => {
  const onClose = vi.fn();
  render(
    <ProgressCard failed onClose={onClose}>
      No se pudo completar la nueva transcripción; se conserva la anterior.
    </ProgressCard>,
  );
  const card = screen.getByRole("status");
  expect(card.dataset.tone).toBe("warn");
  expect(card.querySelector("[data-progress]")).toBeNull();
  fireEvent.click(screen.getByRole("button", { name: "Cerrar" }));
  expect(onClose).toHaveBeenCalledOnce();
});

it("what is being redone stays in sight, dimmed and out of reach", () => {
  const { rerender } = render(
    <BeingRedone active>
      <button type="button">Marta Ibáñez</button>
    </BeingRedone>,
  );
  const box = screen.getByRole("button", { name: "Marta Ibáñez" }).parentElement;
  expect(box?.hasAttribute("inert")).toBe(true);
  expect(box?.getAttribute("aria-busy")).toBe("true");
  expect(box?.dataset.dimmed).toBe("true");
  rerender(
    <BeingRedone active={false}>
      <button type="button">Marta Ibáñez</button>
    </BeingRedone>,
  );
  expect(box?.hasAttribute("inert")).toBe(false);
  expect(box?.dataset.dimmed).toBe("false");
});
