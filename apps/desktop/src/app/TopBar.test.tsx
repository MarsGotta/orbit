import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it } from "vitest";
import type { AppView as AppSnapshot } from "../state/app-view";
import { ShellSlots, TopBarLeading } from "./shell-slots";
import { TopBar } from "./TopBar";

function snapshot(change: Record<string, unknown> = {}): AppSnapshot {
  const permissions = {
    microphone: "granted",
    screen: "granted",
    accessibility: "granted",
    calendars: "granted",
  };
  return {
    connection: { daemon: "ready", hub: "paired" },
    status_availability: { state: "current" },
    privacy_stopped: false,
    permissions,
    settings: { hub: { paired: true, connection: "online" }, permissions },
    ...change,
  } as unknown as AppSnapshot;
}

function renderBar(value = snapshot(), leading?: string) {
  let palette = 0;
  render(
    <ShellSlots>
      <TopBar
        onOpenPalette={() => (palette += 1)}
        onPair={() => undefined}
        send={async () => undefined}
        snapshot={value}
      />
      {leading && <TopBarLeading>{leading}</TopBarLeading>}
    </ShellSlots>,
  );
  return { palette: () => palette };
}

afterEach(() => {
  delete document.documentElement.dataset.theme;
  window.localStorage.clear();
});

describe("the top bar", () => {
  it("opens the palette from its search box, with the shortcut beside it", async () => {
    const { palette } = renderBar();
    const search = screen.getByRole("button", { name: /Buscar o preguntar a Deimos/ });
    expect(search.textContent).toContain("⌘⇧␣");
    await userEvent.click(search);
    expect(palette()).toBe(1);
  });

  it("changes the theme and says which one it changes to", async () => {
    window.localStorage.setItem("orbit.ui.theme.local", "dark");
    renderBar();
    await userEvent.click(screen.getByRole("button", { name: "Cambiar a tema claro" }));
    expect(document.documentElement.dataset.theme).toBe("light");
    expect(screen.getByRole("button", { name: "Cambiar a tema oscuro" })).toBeTruthy();
  });

  it("where there is depth, the view's own trail takes the place of the search box", () => {
    renderBar(snapshot(), "Ajustes / General");
    expect(screen.queryByRole("button", { name: /Buscar o preguntar/ })).toBeNull();
    expect(screen.getByRole("banner").textContent).toContain("Ajustes / General");
  });

  it("shows one notice at a time, the most important, and the rest when asked", async () => {
    renderBar(
      snapshot({
        privacy_stopped: true,
        permissions: {
          microphone: "denied",
          screen: "granted",
          accessibility: "granted",
          calendars: "granted",
        },
      }),
    );
    const notice = screen.getByRole("status");
    expect(notice.textContent).toMatch(/Todo está parado/);
    expect(notice.textContent).not.toMatch(/micrófono/);
    await userEvent.click(screen.getByRole("button", { name: "Ver 1 aviso más" }));
    const rest = screen.getByRole("dialog", { name: "Avisos" });
    expect(rest.textContent).toMatch(/Falta el permiso de micrófono/);
    expect(within(rest).getByRole("link", { name: "Arreglar" }).getAttribute("href")).toBe(
      "#/settings/privacidad",
    );
  });

  it("while Orbit gets ready after opening, it says so instead of nothing", () => {
    renderBar(snapshot({ connection: { daemon: "connecting", hub: "paired" } }));
    expect(screen.getByRole("status").textContent).toBe("Orbit se está preparando…");
  });
});
