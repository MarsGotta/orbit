import { act, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import { NotResponding, Starting, SLOW_START_MS } from "./AppStates";

afterEach(() => {
  vi.useRealTimers();
});

describe("before Orbit answers", () => {
  it("says it connects, and after the promised wait, what happens and what to do", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    let closed = 0;
    render(<Starting onClose={() => (closed += 1)} />);
    const waiting = screen.getByRole("status");
    expect(waiting.textContent).toContain("Conectando con Orbit…");
    expect(waiting.textContent).toContain("Si tarda más de 15 segundos, te diremos qué pasa.");
    expect(screen.queryByRole("button", { name: "Cerrar Orbit" })).toBeNull();
    act(() => vi.advanceTimersByTime(SLOW_START_MS));
    expect(screen.getByRole("alert").textContent).toContain("Orbit está tardando más de lo normal");
    await userEvent.click(screen.getByRole("button", { name: "Cerrar Orbit" }));
    expect(closed).toBe(1);
  });

  it("15 s is what the window promises", () => {
    expect(SLOW_START_MS).toBe(15_000);
  });
});

describe("Orbit no responde", () => {
  it("says what to do, that the saved recordings are safe, and closes Orbit", async () => {
    let closed = 0;
    render(<NotResponding onClose={() => (closed += 1)} />);
    const alert = screen.getByRole("alert");
    expect(alert.textContent).toContain("Orbit no responde");
    expect(alert.textContent).toContain(
      "Ciérralo y vuelve a abrirlo. Tus grabaciones guardadas están a salvo.",
    );
    await userEvent.click(screen.getByRole("button", { name: "Cerrar Orbit" }));
    expect(closed).toBe(1);
  });
});
