import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import type { UpdateSnapshot } from "../bridge/generated/UpdateSnapshot";
import { UpdateNotice } from "./UpdateNotice";

const available: UpdateSnapshot = {
  state: "available",
  version: "0.9.0",
  message: null,
};
beforeEach(() => localStorage.clear());
afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

it("avisa de la versión nueva y Actualizar usa la orden existente", () => {
  const send = vi.fn();
  render(<UpdateNotice update={available} send={send} />);
  expect(screen.getByText("Hay una versión nueva de Orbit (0.9.0)")).toBeTruthy();
  fireEvent.click(screen.getByRole("button", { name: "Actualizar" }));
  expect(send).toHaveBeenCalledWith({ command: "update_install" });
});

it("cerrar se recuerda al reabrir y sólo vuelve con la siguiente versión", () => {
  const view = render(<UpdateNotice update={available} send={vi.fn()} />);
  fireEvent.click(screen.getByRole("button", { name: "Cerrar aviso" }));
  view.unmount();
  const reopened = render(<UpdateNotice update={available} send={vi.fn()} />);
  expect(screen.queryByRole("button", { name: "Actualizar" })).toBeNull();
  reopened.rerender(<UpdateNotice update={{ ...available, version: "0.10.0" }} send={vi.fn()} />);
  expect(screen.getByText("Hay una versión nueva de Orbit (0.10.0)")).toBeTruthy();
});

it("explica el bloqueo y espera a que app-core permita instalar", () => {
  const send = vi.fn();
  const view = render(
    <UpdateNotice
      update={{ ...available, install_blocked: "Termina la grabación antes de actualizar." }}
      send={send}
    />,
  );
  expect(screen.getByText(/Termina la grabación/)).toBeTruthy();
  fireEvent.click(screen.getByRole("button", { name: "Actualizar" }));
  expect(send).not.toHaveBeenCalled();
  view.rerender(<UpdateNotice update={available} send={send} />);
  fireEvent.click(screen.getByRole("button", { name: "Actualizar" }));
  expect(send).toHaveBeenCalledOnce();
});

it.each(["idle", "checking", "up_to_date", "not_newer", "failed", "installing"] as const)(
  "%s no anuncia versión",
  (state) => {
    render(<UpdateNotice update={{ ...available, state }} send={vi.fn()} />);
    expect(screen.queryByRole("button", { name: "Actualizar" })).toBeNull();
  },
);

it("el aviso sigue disponible hasta que Alina lo cierre", () => {
  vi.useFakeTimers();
  render(<UpdateNotice update={available} send={vi.fn()} />);
  act(() => vi.advanceTimersByTime(60_000));
  expect(screen.getByRole("button", { name: "Actualizar" })).toBeTruthy();
});
