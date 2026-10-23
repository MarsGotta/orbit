import { cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import { macPermissions } from "../../i18n/es/permissions";
import { es } from "../../i18n/es";
import { PermissionsStep } from "./OnboardingAccessSteps";

afterEach(cleanup);

// A permission macOS has not reported yet is awaited, never read as a
// refusal (nothing blank: it says it waits for macOS); one refused reads
// «Sin conceder». Accessibility, which macOS never asks for, only opens
// System Settings. The actions stay distinct: nothing is invented for an
// unknown.
it("unknown waits for macOS, a refusal reads «Sin conceder», without invented actions", () => {
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
  expect(within(microphone).queryByText(es.onboarding.notGranted)).toBeNull();
  expect(screen.getAllByText(es.onboarding.notGranted)).toHaveLength(1);
  expect(
    within(row(macPermissions.screen_recording.label)).getByText(es.onboarding.notGranted),
  ).toBeTruthy();
  expect(screen.queryByText(/Desconocido|Denegado|No concedido/)).toBeNull();
  expect(within(row(macPermissions.calendar.label)).getByText("✓ Concedido")).toBeTruthy();
  fireEvent.click(
    within(row(es.onboarding.accessibility)).getByRole("button", {
      name: es.onboarding.openSettings,
    }),
  );
  expect(send).toHaveBeenCalledWith({ command: "open_system_privacy", pane: "Accessibility" });
});
