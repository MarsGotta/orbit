import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import type { SettingsSnapshot } from "../../../bridge/generated";
import { es } from "../../../i18n/es";
import { PrivacySection } from "./GeneralRows";

afterEach(cleanup);

function settings(errorReports?: boolean): SettingsSnapshot {
  return {
    privacy_stopped: false,
    ...(errorReports === undefined ? {} : { error_reports: errorReports }),
  } as SettingsSnapshot;
}

it("the switch is offered only when the build can send, off until the user turns it on", () => {
  const send = vi.fn();
  const { rerender } = render(<PrivacySection send={send} settings={settings()} />);
  expect(screen.queryByRole("switch", { name: es.settings.errorReports })).toBeNull();

  rerender(<PrivacySection send={send} settings={settings(false)} />);
  const toggle = screen.getByRole("switch", { name: es.settings.errorReports });
  expect(toggle.getAttribute("aria-checked")).toBe("false");
  expect(screen.getByText(es.settings.errorReportsHelp)).toBeTruthy();
  fireEvent.click(toggle);
  expect(send).toHaveBeenCalledWith({ command: "error_reports", enabled: true });

  rerender(<PrivacySection send={send} settings={settings(true)} />);
  fireEvent.click(screen.getByRole("switch", { name: es.settings.errorReports }));
  expect(send).toHaveBeenLastCalledWith({ command: "error_reports", enabled: false });
});
