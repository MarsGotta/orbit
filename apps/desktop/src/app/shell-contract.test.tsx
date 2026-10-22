import { render, screen, within } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import type { AppView as AppSnapshot } from "../state/app-view";
import * as ui from "../state/ui";
import { Rail } from "./Rail";

describe("the views that stay", () => {
  afterEach(() => {
    window.location.hash = "";
  });

  it("the rail offers Hoy, Calendario, Reuniones, Asistente and Ajustes, nothing else", () => {
    const snapshot = {
      chat: { pending: null },
      notes: { list: [] },
      settings: { profile: null },
    } as unknown as AppSnapshot;
    render(<Rail activeView="briefing" onNavigate={() => undefined} snapshot={snapshot} />);
    const rail = screen.getByRole("navigation");
    const labels = within(rail)
      .getAllByRole("button")
      .map((button) => button.getAttribute("aria-label"));
    expect(labels).toEqual(["Hoy", "Calendario", "Reuniones", "Asistente", "Ajustes"]);
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
    for (const route of ["#/vault", "#/search", "#/people", "#/dev/estado", "#/design"]) {
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
