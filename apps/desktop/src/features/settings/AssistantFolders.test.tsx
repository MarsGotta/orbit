import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import type { AppView } from "../../state/app-view";
import SettingsView from "./SettingsView";

// Invented folders; only the fields Settings reads.
function snapshot(folders: unknown): AppView {
  return {
    connection: { daemon: "ready" },
    meeting: { call_preference: null },
    settings: {
      permissions: {},
      hub: { paired: false, url: null },
      helper_connected: true,
      versions: { app: "test" },
      update: { state: "idle" },
      privacy_stopped: false,
    },
    computer_control: {
      operation_id: "",
      stage: "",
      detail: "",
      remaining_steps: 0,
      expires_in_ms: 0,
      engine: { state: "unknown" },
      engine_tools: [],
      history: [],
      folders,
    },
  } as unknown as AppView;
}

afterEach(cleanup);

function renderSettings(folders: unknown, send = vi.fn()) {
  render(
    <SettingsView
      snapshot={snapshot(folders)}
      send={send}
      error={null}
      onReopenOnboarding={() => undefined}
    />,
  );
  return send;
}

it("lists each folder with whether it opens and sends add and remove orders", () => {
  const send = renderSettings({
    state: "known",
    max: 8,
    folders: [
      { id: "a", path: "/Users/ejemplo/Proyectos", access: { state: "accessible" } },
      {
        id: "b",
        path: "/Volumes/Disco/Notas",
        access: {
          state: "unavailable",
          cause: { kind: "bookmark_unresolvable", domain: "NSCocoaErrorDomain", code: 4 },
        },
      },
      { id: "c", path: "/Users/ejemplo/Fotos", access: { state: "unknown" } },
    ],
  });
  const section = screen.getByRole("region", { name: "Carpetas que puede leer" });
  expect(section.textContent).toContain("/Users/ejemplo/Proyectos");
  expect(section.textContent).toContain("Se puede leer");
  expect(section.textContent).toContain("No se encuentra");
  expect(section.textContent).toContain("Sin comprobar todavía");
  fireEvent.click(screen.getByRole("button", { name: "Añadir carpeta" }));
  expect(send).toHaveBeenLastCalledWith({ command: "assistant_folder_add" });
  fireEvent.click(screen.getByRole("button", { name: "Quitar /Volumes/Disco/Notas" }));
  expect(send).toHaveBeenLastCalledWith({ command: "assistant_folder_remove", id: "b" });
});

it("says how many folders fit when the list is full and does not offer another", () => {
  const folders = Array.from({ length: 8 }, (_, index) => ({
    id: `f${index}`,
    path: `/Users/ejemplo/C${index}`,
    access: { state: "accessible" },
  }));
  renderSettings({ state: "known", max: 8, folders });
  const add = screen.getByRole("button", { name: "Añadir carpeta" }) as HTMLButtonElement;
  expect(add.disabled).toBe(true);
  expect(screen.getByText(/8 carpetas como máximo/)).toBeTruthy();
});

it("shows a list it could not read as unreadable, never as empty", () => {
  renderSettings({ state: "unreadable", max: 8, folders: [], unreadable: "fila dañada" });
  expect(screen.getByText(/No se pudieron leer las carpetas guardadas/)).toBeTruthy();
  expect(screen.queryByText(/Todavía no hay carpetas/)).toBeNull();
});
