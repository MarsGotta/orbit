import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import type { AppView } from "../../state/app-view";
import { OnboardingView } from "./OnboardingView";

afterEach(cleanup);

const snapshot = {
  settings: { hub: { url: null, paired: false, device_name: null }, permissions: {} },
} as unknown as AppView;

// The welcome's skip leaves the whole first run; a step's own skip leaves
// only that step. Each says which.
it("«Configurarlo más tarde» on the welcome leaves the whole first run", () => {
  const onComplete = vi.fn();
  const send = vi.fn(async () => undefined);
  render(<OnboardingView error={null} onComplete={onComplete} send={send} snapshot={snapshot} />);
  expect(screen.getByRole("heading", { name: "Te damos la bienvenida a Orbit" })).toBeTruthy();
  fireEvent.click(screen.getByRole("button", { name: "Configurarlo más tarde" }));
  expect(send).toHaveBeenCalledWith({ command: "onboarding_done" });
  expect(onComplete).toHaveBeenCalledTimes(1);
});

it("«Emparejar después» on the hub step goes on to the next step", () => {
  const onComplete = vi.fn();
  render(
    <OnboardingView
      error={null}
      onComplete={onComplete}
      send={vi.fn(async () => undefined)}
      snapshot={snapshot}
      startAtPair
    />,
  );
  fireEvent.click(screen.getByRole("button", { name: "Emparejar después" }));
  expect(onComplete).not.toHaveBeenCalled();
  expect(screen.getByRole("region", { name: "Elige qué puede usar Orbit" })).toBeTruthy();
});
