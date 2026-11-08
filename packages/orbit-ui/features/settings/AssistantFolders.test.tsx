import { act, cleanup, fireEvent, render, screen, within } from "@testing-library/react";
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
      section="asistentes"
      snapshot={snapshot(folders)}
      send={send}
      error={null}
      onReopenOnboarding={() => undefined}
    />,
  );
  return send;
}

it("lists each folder by name and place, with whether it opens, and sends add and remove orders", () => {
  const send = renderSettings({
    state: "known",
    max: 8,
    folders: [
      { id: "a", path: "/Users/ejemplo/Documents/Proyectos", access: { state: "accessible" } },
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
  const section = screen.getByRole("region", { name: "Carpetas que pueden leer" });
  expect(within(section).getByText("3 de 8")).toBeTruthy();
  expect(section.textContent).toContain("ProyectosDocumentos › ProyectosSe puede leer");
  // The error is said inside its row, with what fixes it.
  const lost = within(section).getByText("Disco «Disco» › Notas").closest("[data-warn]");
  expect(lost?.textContent).toContain(
    "No se encuentra: se borró o su disco no está conectado.Quítala y vuelve a añadirla.",
  );
  expect(section.textContent).toContain("Comprobando…");
  fireEvent.click(screen.getByRole("button", { name: "Añadir carpeta" }));
  expect(send).toHaveBeenLastCalledWith({ command: "assistant_folder_add" });
  fireEvent.click(screen.getByRole("button", { name: "Quitar Notas" }));
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
  expect(screen.getByText("8 de 8")).toBeTruthy();
});

it("shows a list it could not read as unreadable, never as empty", () => {
  renderSettings({ state: "unreadable", max: 8, folders: [], unreadable: "fila dañada" });
  expect(screen.getAllByText(/No se pudieron leer las carpetas guardadas/).length).toBeGreaterThan(
    0,
  );
  expect(screen.queryByText(/Todavía no hay carpetas/)).toBeNull();
});

it("shows the folders' shape while the daemon has not reported them", () => {
  renderSettings({ state: "unknown", max: 8, folders: [] });
  const section = screen.getByRole("region", { name: "Carpetas que pueden leer" });
  expect(section.querySelector("[aria-busy='true']")).toBeTruthy();
  expect(screen.queryByText(/Todavía no hay carpetas/)).toBeNull();
});

it("«Añadir carpeta» says «Abriendo…» until the daemon has the order", async () => {
  let settle: () => void = () => undefined;
  const send = vi.fn(() => new Promise<void>((resolve) => (settle = resolve)));
  renderSettings({ state: "known", max: 8, folders: [] }, send);
  const add = screen.getByRole("button", { name: /Añadir carpeta/ });
  fireEvent.click(add);
  expect(add.getAttribute("aria-busy")).toBe("true");
  expect(within(add).getByText("Abriendo…").getAttribute("aria-hidden")).toBe("false");
  await act(async () => settle());
  expect(add.getAttribute("aria-busy")).toBeNull();
});
