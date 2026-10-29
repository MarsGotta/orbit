import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import type { AppView } from "../../state/app-view";
import { SystemPermissions } from "./Permissions";
import { PermissionsStep } from "../onboarding/OnboardingAccessSteps";

afterEach(cleanup);
const permissions = {
  microphone: "granted",
  screen: "not_granted",
  accessibility: "not_granted",
  calendars: "granted",
} as const;

it.each(["screen", "accessibility"] as const)(
  "Settings requests %s before offering its system pane",
  async (key) => {
    const send = vi.fn(async () => true);
    render(
      <SystemPermissions
        send={send}
        settings={{ permissions, helper_connected: true } as AppView["settings"]}
      />,
    );
    const title = key === "screen" ? "Pantalla y audio del sistema" : "Accesibilidad";
    fireEvent.click(screen.getByRole("button", { name: `Pedir permiso: ${title}` }));
    expect(send).toHaveBeenLastCalledWith({ command: "permissions", request: [key] });
    fireEvent.click(
      await screen.findByRole("button", { name: `Abrir Ajustes del Sistema: ${title}` }),
    );
    expect(send).toHaveBeenLastCalledWith({
      command: "open_system_privacy",
      pane: key === "screen" ? "ScreenCapture" : "Accessibility",
    });
  },
);

it.each(["screen", "accessibility"] as const)(
  "onboarding requests %s before offering its system pane",
  async (key) => {
    const send = vi.fn(async () => true);
    render(<PermissionsStep permissions={permissions} send={send} />);
    const title = key === "screen" ? "Pantalla y audio del sistema" : "Accesibilidad";
    const row = screen.getByText(title).closest("li")!;
    fireEvent.click(within(row).getByRole("button", { name: "Pedir" }));
    expect(send).toHaveBeenLastCalledWith({ command: "permissions", request: [key] });
    await waitFor(() =>
      expect(within(row).getByRole("button", { name: "Abrir Ajustes del Sistema" })).toBeTruthy(),
    );
    fireEvent.click(within(row).getByRole("button", { name: "Abrir Ajustes del Sistema" }));
    expect(send).toHaveBeenLastCalledWith({
      command: "open_system_privacy",
      pane: key === "screen" ? "ScreenCapture" : "Accessibility",
    });
  },
);
