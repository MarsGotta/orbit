import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import type { AppView } from "../../state/app-view";
import { es } from "../../i18n/es";
import { OnboardingView } from "./OnboardingView";

afterEach(() => {
  cleanup();
  vi.unstubAllEnvs();
});

function show(url: string | null) {
  render(
    <OnboardingView
      error={null}
      onComplete={vi.fn()}
      send={vi.fn()}
      snapshot={{ settings: { hub: { url, paired: false, device_name: null } } } as AppView}
      startAtPair
    />,
  );
  return screen.getByLabelText<HTMLInputElement>(es.onboarding.hubUrl).value;
}

it("leaves the hub empty when the build has no proposed URL", () => {
  vi.stubEnv("VITE_ORBIT_HUB_URL", "");
  expect(show(null)).toBe("");
});

it("proposes the URL configured for the build", () => {
  vi.stubEnv("VITE_ORBIT_HUB_URL", "https://hub.example");
  expect(show(null)).toBe("https://hub.example");
});

it("keeps the saved hub ahead of the build's proposal", () => {
  vi.stubEnv("VITE_ORBIT_HUB_URL", "https://hub.example");
  expect(show("https://saved.example.test")).toBe("https://saved.example.test");
});
