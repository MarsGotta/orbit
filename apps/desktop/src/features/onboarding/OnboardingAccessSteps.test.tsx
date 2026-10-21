import { cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import { macPermissions } from "../../i18n/es/permissions";
import { es } from "../../i18n/es";
import { PermissionsStep } from "./OnboardingAccessSteps";

afterEach(cleanup);

// An unknown permission says nothing (it is not a refusal); denied
// and not granted read the same to the person, «Sin conceder». The actions
// stay distinct: nothing is invented for an unknown.
it("unknown says nothing, denied and not granted read «Sin conceder», without invented actions", () => {
  const send = vi.fn(async () => undefined);
  render(
    <PermissionsStep
      onContinue={() => undefined}
      permissions={{
        microphone: "unknown",
        screen: "denied",
        accessibility: "not_granted",
        calendars: "granted",
      }}
      send={send}
    />,
  );
  const microphoneRow = screen.getByText(macPermissions.microphone.label).closest("li");
  expect(microphoneRow?.textContent).toBe(macPermissions.microphone.label);
  expect(screen.getAllByText(es.permissions.notGranted)).toHaveLength(2);
  expect(screen.queryByText(/Desconocido|Denegado|No concedido/)).toBeNull();
  const microphone = screen.getByText(macPermissions.microphone.label).closest("li");
  expect(within(microphone as HTMLElement).queryByRole("button")).toBeNull();
  const accessibility = screen.getByText(es.onboarding.accessibility).closest("li");
  fireEvent.click(
    within(accessibility as HTMLElement).getByRole("button", {
      name: es.settings.openSystemSettings,
    }),
  );
  expect(send).toHaveBeenCalledWith({ command: "open_system_privacy", pane: "Accessibility" });
});
