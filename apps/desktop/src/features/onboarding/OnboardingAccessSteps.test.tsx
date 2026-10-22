import { cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import { macPermissions } from "../../i18n/es/permissions";
import { es } from "../../i18n/es";
import { PermissionsStep } from "./OnboardingAccessSteps";

afterEach(cleanup);

// A permission macOS has not reported yet is awaited, never read as a
// refusal (nothing blank: it says it waits for macOS); denied and not granted
// read the same to the person, «Sin permiso». The actions stay distinct:
// nothing is invented for an unknown.
it("unknown waits for macOS, denied and not granted read «Sin permiso», without invented actions", () => {
  const send = vi.fn(async () => undefined);
  render(
    <PermissionsStep
      permissions={{
        microphone: "unknown",
        screen: "denied",
        accessibility: "not_granted",
        calendars: "granted",
      }}
      send={send}
    />,
  );
  const row = (label: string) => screen.getByText(label).closest("li") as HTMLElement;
  const microphone = row(macPermissions.microphone.label);
  expect(within(microphone).getByText(es.controls.waitingForMacos)).toBeTruthy();
  expect(within(microphone).queryByRole("button")).toBeNull();
  expect(within(microphone).queryByText(es.permissions.notGranted)).toBeNull();
  expect(screen.getAllByText(es.permissions.notGranted)).toHaveLength(2);
  expect(screen.queryByText(/Desconocido|Denegado|No concedido/)).toBeNull();
  expect(within(row(macPermissions.calendar.label)).getByText("✓ Permitido")).toBeTruthy();
  fireEvent.click(
    within(row(es.onboarding.accessibility)).getByRole("button", {
      name: es.onboarding.openSettings,
    }),
  );
  expect(send).toHaveBeenCalledWith({ command: "open_system_privacy", pane: "Accessibility" });
});
