import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import type { AppView } from "orbit-ui/state/app-view";
import { OnboardingView } from "./OnboardingView";
import styles from "./OnboardingView.module.css";

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

const snapshot = {
  settings: { hub: { url: null, paired: false, device_name: null }, permissions: {} },
} as unknown as AppView;

const MAC =
  "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko)";
const IPHONE =
  "Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Mobile/15E148";

function headerFor(userAgent: string): HTMLElement {
  vi.spyOn(navigator, "userAgent", "get").mockReturnValue(userAgent);
  render(
    <OnboardingView
      error={null}
      onComplete={vi.fn()}
      send={vi.fn(async () => undefined)}
      snapshot={snapshot}
    />,
  );
  const header = screen.getByRole("button", { name: "Configurarlo más tarde" }).closest("header");
  if (!header) throw new Error("the welcome has no header");
  return header;
}

function leavesMacRoom(header: HTMLElement): boolean {
  const mac = styles.mac;
  if (mac === undefined) throw new Error("the welcome has no Mac class");
  return header.classList.contains(mac);
}

// The header leaves room for the window buttons only on a Mac; an iPhone's
// agent says «like Mac OS X» but has no window buttons.
it("leaves room for the window buttons on a Mac", () => {
  expect(leavesMacRoom(headerFor(MAC))).toBe(true);
});

it("leaves no room for window buttons on an iPhone", () => {
  expect(leavesMacRoom(headerFor(IPHONE))).toBe(false);
});
