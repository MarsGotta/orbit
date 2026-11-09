import { act, render, screen, within } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import { followTheme, toggleTheme } from "../../design/theme";
import { CopyProvider } from "../../i18n/CopyProvider";
import type { AppView } from "../../state/app-view";
import { GeneralSection } from "./GeneralSection";

let stop = () => undefined as void;
afterEach(() => {
  stop();
  window.localStorage.clear();
  vi.unstubAllGlobals();
});

it.each([
  ["en", "Theme", "System", "Light", "Dark"],
  ["es", "Tema", "Sistema", "Claro", "Oscuro"],
] as const)(
  "%s: System is the default and can follow the Mac again after a fixed theme",
  (language, label, system, light, dark) => {
    window.localStorage.clear();
    const media = new EventTarget();
    let systemLight = false;
    vi.stubGlobal("matchMedia", () => Object.assign(media, { matches: systemLight }));
    stop = followTheme();
    render(
      <CopyProvider choice={language}>
        <GeneralSection
          snapshot={
            {
              settings: {
                hub: { paired: false },
                agents: [],
                permissions: {},
                versions: { app: "0.13.0" },
                update: { state: "idle" },
                profile: { name: "Robin" },
              },
            } as unknown as AppView
          }
          onReopenOnboarding={vi.fn()}
          send={vi.fn()}
        />
      </CopyProvider>,
    );
    const choices = within(screen.getByRole("radiogroup", { name: label }));
    const selected = (name: string) =>
      choices.getByRole("radio", { name }).getAttribute("aria-checked");
    expect(selected(system)).toBe("true");
    act(() => choices.getByRole("radio", { name: light }).click());
    expect(selected(light)).toBe("true");
    expect(document.documentElement.dataset.theme).toBe("light");
    act(() => toggleTheme());
    expect(selected(dark)).toBe("true");
    act(() => choices.getByRole("radio", { name: system }).click());
    expect(window.localStorage.getItem("orbit.ui.theme.local")).toBeNull();
    expect(document.documentElement.dataset.theme).toBeUndefined();
    act(() => {
      systemLight = true;
      media.dispatchEvent(new Event("change"));
    });
    expect(selected(system)).toBe("true");
    // The top bar keeps toggling the resolved theme, even from System.
    act(() => toggleTheme());
    expect(document.documentElement.dataset.theme).toBe("dark");
  },
);
