import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import type { DaemonPort } from "../bridge/port";
import { baseViews, bootstrapOf, withOpenNote } from "../showcase/base";
import { App } from "./App";

afterEach(() => {
  cleanup();
  window.localStorage.clear();
  window.location.hash = "";
});

function openApp(views = baseViews()) {
  window.localStorage.setItem("orbit.onboarding.done.local", "true");
  let action: (name: string) => void = () => undefined;
  const states: Record<string, unknown>[] = [];
  const send = vi.fn(async () => ({ state: "accepted" }));
  const port = {
    snapshot: async () => bootstrapOf(views),
    subscribeFrames: async () => () => undefined,
    report: async () => undefined,
    send,
    onMenuAction: async (listener: typeof action) => {
      action = listener;
      return () => undefined;
    },
    setMenuState: async (state: Record<string, unknown>) => {
      states.push(state);
    },
  } as unknown as DaemonPort;
  const view = render(<App port={port} />);
  return { ...view, send, states, menu: (name: string) => act(() => action(name)) };
}

it("opens existing Settings and new-conversation routes from native menu actions", async () => {
  const app = openApp();
  await screen.findByRole("button", { name: "Ocultar barra lateral" });
  app.menu("settings");
  expect(window.location.hash).toBe("#/settings/general");
  app.menu("new_conversation");
  expect(window.location.hash).toBe("#/assistant/new");
  app.menu("stop");
  await waitFor(() =>
    expect(app.send).toHaveBeenCalledWith({ command: "privacy_stop" }, expect.any(String)),
  );
});

it("hides the sidebar from the top area and restores that choice after remounting", async () => {
  const app = openApp();
  fireEvent.click(await screen.findByRole("button", { name: "Ocultar barra lateral" }));
  expect(screen.queryByRole("navigation", { name: "Principal" })).toBeNull();
  expect(screen.getByRole("button", { name: "Mostrar barra lateral" })).toBeTruthy();
  app.unmount();
  const again = openApp();
  await screen.findByRole("button", { name: "Mostrar barra lateral" });
  again.menu("sidebar");
  expect(screen.getByRole("navigation", { name: "Principal" })).toBeTruthy();
});

it("shares and remembers the reading scale, caps Bigger at 200%, and restores Actual Size", async () => {
  const app = openApp();
  await screen.findByRole("button", { name: "Ocultar barra lateral" });
  for (let step = 0; step < 12; step++) app.menu("bigger");
  expect(document.documentElement.style.getPropertyValue("--o-reading-scale")).toBe("2");
  expect(app.states.at(-1)).toMatchObject({ bigger: false, smaller: true });
  app.unmount();
  const again = openApp();
  await screen.findByRole("button", { name: "Ocultar barra lateral" });
  expect(document.documentElement.style.getPropertyValue("--o-reading-scale")).toBe("2");
  again.menu("smaller");
  expect(document.documentElement.style.getPropertyValue("--o-reading-scale")).toBe("1.9");
  again.menu("actual_size");
  expect(document.documentElement.style.getPropertyValue("--o-reading-scale")).toBe("1");
});

it("enables Find only for a mounted transcript and focuses its existing search", async () => {
  window.location.hash = "#/notes";
  const app = openApp(withOpenNote(baseViews()));
  fireEvent.click(await screen.findByRole("tab", { name: "Transcripción" }));
  await waitFor(() => expect(app.states.at(-1)).toMatchObject({ find: true }));
  app.menu("find");
  expect(document.activeElement?.getAttribute("type")).toBe("search");
  app.menu("settings");
  await waitFor(() => expect(app.states.at(-1)).toMatchObject({ find: false }));
});

it("opens New Recording and disables it while a recording is already running", async () => {
  const app = openApp();
  await screen.findByRole("button", { name: "Ocultar barra lateral" });
  expect(app.states.at(-1)).toMatchObject({ record: true, finish: false });
  app.menu("record");
  expect(await screen.findByRole("dialog", { name: "Nueva grabación" })).toBeTruthy();
  app.unmount();
  const views = baseViews();
  views.recording.meeting.phase = "recording";
  const live = openApp(views);
  await screen.findByRole("button", { name: "Ocultar barra lateral" });
  expect(live.states.at(-1)).toMatchObject({ record: false, finish: true });
  live.menu("sidebar");
  expect(screen.getByRole("button", { name: "Mostrar barra lateral" })).toBeTruthy();
  expect(live.states.at(-1)).toMatchObject({ finish: true });
  live.menu("finish");
  await waitFor(() =>
    expect(live.send).toHaveBeenCalledWith({ command: "meeting_finish" }, expect.any(String)),
  );
});

it("returns Undo to the template handler when navigation removes a focused text field", async () => {
  window.location.hash = "#/assistant";
  const app = openApp();
  const field = await screen.findByRole("textbox", { name: "Mensaje para Deimos" });
  act(() => field.focus());
  await waitFor(() => expect(app.states.at(-1)).toMatchObject({ editing: true }));
  app.menu("settings");
  await waitFor(() => expect(app.states.at(-1)).toMatchObject({ editing: false }));
});

it("disables Find while the transcript has no search field yet", async () => {
  window.location.hash = "#/notes";
  const views = withOpenNote(baseViews());
  views.note!.transcript.pending_chunks = 1;
  const app = openApp(views);
  fireEvent.click(await screen.findByRole("tab", { name: "Transcripción" }));
  expect(screen.queryByRole("searchbox")).toBeNull();
  await waitFor(() => expect(app.states.at(-1)).toMatchObject({ find: false }));
});

it("keeps Command-Z available after leaving the deleted template's settings", async () => {
  window.location.hash = "#/settings/reuniones";
  const app = openApp();
  fireEvent.click(await screen.findByRole("button", { name: "Revisión de diseño" }));
  fireEvent.click(screen.getByRole("button", { name: "Borrar plantilla" }));
  app.menu("settings");
  await waitFor(() => expect(screen.queryByRole("button", { name: "Deshacer" })).toBeNull());
  fireEvent.keyDown(window, { key: "z", metaKey: true });
  await waitFor(() =>
    expect(app.send).toHaveBeenCalledWith(
      expect.objectContaining({
        command: "note_library",
        request: expect.objectContaining({
          action: "restore_template",
          template_id: "design-review",
          series: ["item-design-review"],
        }),
      }),
      expect.any(String),
    ),
  );
});
