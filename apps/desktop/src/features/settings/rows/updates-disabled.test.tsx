import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import type { SettingsSnapshot } from "../../../bridge/generated";
import { es } from "../../../i18n/es";
import { UpdatesSection } from "./GeneralRows";

afterEach(cleanup);

it("explains why this build cannot check updates and disables the search", () => {
  const settings = {
    update: { state: "disabled" },
    versions: { app: "1.0.0" },
  } as unknown as SettingsSnapshot;
  render(<UpdatesSection send={vi.fn()} settings={settings} />);
  expect(screen.getByText(/ORBIT_UPDATE_BASE_URL/)).toBeTruthy();
  expect(
    screen.getByRole<HTMLButtonElement>("button", { name: es.settings.searchUpdate }).disabled,
  ).toBe(true);
});
