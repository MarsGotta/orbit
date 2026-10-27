import { render, screen, within } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import type { AppView as AppSnapshot } from "../state/app-view";
import * as ui from "../state/ui";
import { Sidebar } from "./Sidebar";

describe("the views that stay", () => {
  afterEach(() => {
    window.location.hash = "";
  });

  it("the sidebar offers Hoy, Calendario, Reuniones, Notas, Asistente and Ajustes", () => {
    const snapshot = {
      chat: { pending: null },
      notes: { list: [] },
      settings: { profile: null },
      meeting: { phase: "idle", tracks: [] },
      call: null,
    } as unknown as AppSnapshot;
    render(
      <Sidebar
        onRecord={() => undefined}
        send={() => undefined}
        snapshot={snapshot}
        view="briefing"
      />,
    );
    const sidebar = screen.getByRole("navigation");
    const labels = within(sidebar)
      .getAllByRole("link")
      .map((link) => link.textContent);
    expect(labels).toEqual(["Hoy", "Calendario", "Reuniones", "Notas", "Asistente", "Ajustes"]);
  });

  it("opening the app without a route shows Hoy", () => {
    window.location.hash = "";
    const stop = ui.initializeUiStore();
    try {
      expect(ui.useUiStore.getState().view).toBe("briefing");
    } finally {
      stop();
    }
  });

  it("a route of a view that left shows Hoy", () => {
    for (const route of ["#/search", "#/people", "#/dev/estado", "#/design"]) {
      window.location.hash = route;
      const stop = ui.initializeUiStore();
      try {
        expect(ui.useUiStore.getState().view).toBe("briefing");
      } finally {
        stop();
      }
    }
  });
});

it("the vault route opens Notas independently of meeting notes", () => {
  window.location.hash = "#/vault/n/personas%2Faster.md";
  const stop = ui.initializeUiStore();
  try {
    expect(ui.useUiStore.getState().view).toBe("vault");
  } finally {
    stop();
    window.location.hash = "";
  }
});
