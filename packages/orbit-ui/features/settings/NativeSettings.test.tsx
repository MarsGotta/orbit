import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import type { AppView } from "../../state/app-view";
import { LoginItem, NotificationPermission } from "./NativeSettings";

afterEach(cleanup);
it("shows approval required without claiming login is enabled, and refreshes on return", async () => {
  const send = vi.fn(async () => true);
  render(
    <LoginItem send={send} settings={{ login_item: "requires_approval" } as AppView["settings"]} />,
  );
  expect(screen.getByRole("switch").getAttribute("aria-checked")).toBe("false");
  expect(screen.getByText("Requiere aprobación en Ajustes del Sistema.")).toBeTruthy();
  fireEvent.click(screen.getByRole("button", { name: "Abrir Ajustes del Sistema" }));
  expect(send).toHaveBeenCalledWith({ command: "open_system_privacy", pane: "LoginItems" });
  send.mockClear();
  fireEvent.focus(window);
  await waitFor(() => expect(send).toHaveBeenCalledWith({ command: "desktop_status" }));
});
it("shows notification refusal and its native settings remedy, with no Orbit switch", () => {
  const send = vi.fn(async () => true);
  render(
    <NotificationPermission
      send={send}
      settings={
        { notifications: { state: "denied", error: "UNErrorDomain 17" } } as AppView["settings"]
      }
    />,
  );
  expect(screen.getByText("Sin permiso")).toBeTruthy();
  expect(screen.getByText(/UNErrorDomain 17/)).toBeTruthy();
  expect(screen.queryByRole("switch")).toBeNull();
  fireEvent.click(
    screen.getByRole("button", { name: "Abrir Ajustes del Sistema: Notificaciones" }),
  );
  expect(send).toHaveBeenCalledWith({ command: "open_system_privacy", pane: "Notifications" });
});
it("requests the system notification permission only when undetermined", () => {
  const send = vi.fn(async () => true);
  render(
    <NotificationPermission
      send={send}
      settings={{ notifications: { state: "not_determined" } } as AppView["settings"]}
    />,
  );
  fireEvent.click(screen.getByRole("button", { name: "Pedir permiso: Notificaciones" }));
  expect(send).toHaveBeenCalledWith({ command: "notifications_request" });
});
