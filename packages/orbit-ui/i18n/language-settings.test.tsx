import { fireEvent, render, screen } from "@testing-library/react";
import { expect, it, vi } from "vitest";
import type { AppView } from "../state/app-view";
import SettingsView from "../features/settings/SettingsView";
import { CopyProvider } from "./CopyProvider";

it("changes the visible Settings copy immediately and sends the Mac's choice", async () => {
  const send = vi.fn(async () => true);
  const snapshot = {
    connection: { daemon: "ready" },
    settings: {
      language: "system",
      agents: [],
      permissions: {},
      hub: { paired: false },
      versions: { app: "0.1.0" },
      update: { state: "idle" },
      profile: { name: "Robin" },
    },
  } as unknown as AppView;
  render(
    <CopyProvider choice="system" languages={["es-ES"]}>
      <SettingsView
        snapshot={snapshot}
        send={send}
        error={null}
        onReopenOnboarding={() => undefined}
      />
    </CopyProvider>,
  );
  expect(screen.getByRole("heading", { name: "Ajustes" })).toBeTruthy();
  fireEvent.change(screen.getByRole("combobox", { name: "Idioma" }), { target: { value: "en" } });
  expect(await screen.findByRole("heading", { name: "Settings" })).toBeTruthy();
  expect(screen.getByRole("combobox", { name: "Language" }).textContent).toContain(
    "Same as the system",
  );
  expect(send).toHaveBeenCalledWith({ command: "language_set", language: "en" });
});
