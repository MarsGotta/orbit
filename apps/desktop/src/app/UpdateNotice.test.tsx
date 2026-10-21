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

it("announces the new version and Actualizar uses the existing order", () => {
  const send = vi.fn();
  render(<UpdateNotice update={available} send={send} />);
  expect(screen.getByText("Hay una versión nueva de Orbit (0.9.0)")).toBeTruthy();
  fireEvent.click(screen.getByRole("button", { name: "Actualizar" }));
  expect(send).toHaveBeenCalledWith({ command: "update_install" });
});

it("closing it is remembered on reopening, and it only comes back with the next version", () => {
  const view = render(<UpdateNotice update={available} send={vi.fn()} />);
  fireEvent.click(screen.getByRole("button", { name: "Cerrar aviso" }));
  view.unmount();
  const reopened = render(<UpdateNotice update={available} send={vi.fn()} />);
  expect(screen.queryByRole("button", { name: "Actualizar" })).toBeNull();
  reopened.rerender(<UpdateNotice update={{ ...available, version: "0.10.0" }} send={vi.fn()} />);
  expect(screen.getByText("Hay una versión nueva de Orbit (0.10.0)")).toBeTruthy();
});

it("explains the block and waits for app-core to allow installing", () => {
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
  "%s announces no version",
  (state) => {
    render(<UpdateNotice update={{ ...available, state }} send={vi.fn()} />);
    expect(screen.queryByRole("button", { name: "Actualizar" })).toBeNull();
  },
);

it("the notice stays available until Alina closes it", () => {
  vi.useFakeTimers();
  render(<UpdateNotice update={available} send={vi.fn()} />);
  act(() => vi.advanceTimersByTime(60_000));
  expect(screen.getByRole("button", { name: "Actualizar" })).toBeTruthy();
});
