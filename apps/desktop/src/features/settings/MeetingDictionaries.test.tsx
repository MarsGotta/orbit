import { cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import type { AppView } from "../../state/app-view";
import SettingsView from "./SettingsView";

function snapshot(state = "available"): AppView {
  // Only the view fields Settings consumes; all content is synthetic.
  return {
    connection: { daemon: "ready" },
    meeting: { call_preference: null },
    settings: {
      permissions: {},
      hub: { paired: false, url: null },
      helper_connected: false,
      versions: { app: "test" },
      update: { state: "idle" },
      privacy_stopped: false,
    },
    notes: {
      library: {
        read: "available",
        read_detail: null,
        bounds: {
          max_sections: 12,
          max_name_chars: 80,
          max_context_chars: 4000,
          max_instructions_chars: 2000,
          max_custom_templates: 32,
        },
        templates: { state: "available" },
        items: [],
        links: { state: "available" },
        assignments: [],
        refusals: [],
        restores: [],
        dictionaries: {
          state,
          detail: state === "needs_hub" ? "No se pudo conectar con el hub" : null,
        },
        dictionary_pending: [],
        dictionary_failures: [],
        dictionary_items:
          state === "available"
            ? [
                {
                  pending: false,
                  dictionary: {
                    id: "vocabulary",
                    name: "Proyecto",
                    series: [
                      { calendar_item_id: "daily", title: "Daily" },
                      { calendar_item_id: "refine", title: "Refinamiento" },
                    ],
                    terms: [{ id: "entry", correct: "NAV", wrong: ["nab"] }],
                  },
                },
              ]
            : [],
      },
    },
  } as unknown as AppView;
}

afterEach(cleanup);

it("Settings lists the dictionaries, terms and series and sends rename and remove orders", () => {
  const send = vi.fn();
  render(
    <SettingsView
      snapshot={snapshot()}
      send={send}
      error={null}
      onReopenOnboarding={() => undefined}
    />,
  );
  const name = screen.getByRole("textbox", { name: "Nombre del diccionario" });
  expect(screen.getByText("Series: Daily, Refinamiento")).toBeTruthy();
  expect(screen.getByText("NAV")).toBeTruthy();
  fireEvent.change(name, { target: { value: "Nuevo nombre" } });
  const form = name.closest("form");
  if (!form) throw new Error("dictionary form absent");
  fireEvent.click(within(form).getByRole("button", { name: "Guardar" }));
  expect(send).toHaveBeenLastCalledWith({
    command: "note_library",
    request: {
      action: "dictionary",
      request: { action: "save", dictionary_id: "vocabulary", name: "Nuevo nombre" },
    },
  });
  fireEvent.click(screen.getByRole("button", { name: "Quitar NAV" }));
  expect(send).toHaveBeenLastCalledWith({
    command: "note_library",
    request: {
      action: "dictionary",
      request: { action: "remove", dictionary_id: "vocabulary", entry_id: "entry" },
    },
  });
});

it("Settings shows an unreachable dictionary list as unavailable, never as empty", () => {
  render(
    <SettingsView
      snapshot={snapshot("needs_hub")}
      send={vi.fn()}
      error={null}
      onReopenOnboarding={() => undefined}
    />,
  );
  expect(screen.getByText("No se pudo conectar con el hub")).toBeTruthy();
  expect(screen.queryByText(/Todavía no hay diccionarios/)).toBeNull();
});
