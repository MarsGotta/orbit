import { afterEach, describe, expect, it } from "vitest";
import * as ui from "./ui";

afterEach(() => {
  window.location.hash = "";
});

function openedAt(hash: string) {
  window.location.hash = hash;
  const stop = ui.initializeUiStore();
  try {
    const { view, settingsSection } = ui.useUiStore.getState();
    return { view, section: settingsSection };
  } finally {
    stop();
  }
}

describe("the routes of Ajustes", () => {
  it("has exactly the five sections, each with its own route", () => {
    expect(ui.SETTINGS_SECTIONS).toEqual([
      "general",
      "privacidad",
      "grabacion",
      "asistentes",
      "reuniones",
    ]);
    for (const section of ui.SETTINGS_SECTIONS) {
      expect(ui.settingsRoute(section)).toBe(`#/settings/${section}`);
      expect(openedAt(`#/settings/${section}`)).toEqual({ view: "settings", section });
    }
  });

  it("the old route of «Asistente» opens «Asistentes» and the address says so", () => {
    expect(openedAt("#/settings/asistente")).toEqual({ view: "settings", section: "asistentes" });
    expect(window.location.hash).toBe("#/settings/asistentes");
  });

  it("opens General without a section, and with one it does not know", () => {
    expect(openedAt("#/settings")).toEqual({ view: "settings", section: "general" });
    expect(openedAt("#/settings/permisos")).toEqual({ view: "settings", section: "general" });
  });

  it("a notice that fixes something opens its exact section", () => {
    ui.useUiStore.getState().openSettings("privacidad");
    expect(window.location.hash).toBe("#/settings/privacidad");
    expect(ui.useUiStore.getState()).toMatchObject({
      view: "settings",
      settingsSection: "privacidad",
    });
  });

  it("a note's own route is Reuniones", () => {
    expect(openedAt("#/notes/rec-1").view).toBe("notes");
  });
});
