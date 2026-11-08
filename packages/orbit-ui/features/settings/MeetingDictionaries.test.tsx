import { cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import type { AppView } from "../../state/app-view";
import SettingsView from "./SettingsView";

function snapshot(state = "available", items?: unknown[]): AppView {
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
          items ??
          (state === "available"
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
            : []),
      },
    },
  } as unknown as AppView;
}

afterEach(cleanup);

it("Settings lists the dictionaries, each term with what it replaces, and sends rename and remove orders", () => {
  const send = vi.fn();
  render(
    <SettingsView
      section="reuniones"
      snapshot={snapshot()}
      send={send}
      error={null}
      onReopenOnboarding={() => undefined}
    />,
  );
  expect(screen.getByText("Lo usan siempre: Daily, Refinamiento")).toBeTruthy();
  expect(screen.getByText("NAV")).toBeTruthy();
  expect(screen.getByText("en lugar de nab")).toBeTruthy();
  expect(
    screen.getByText(
      "Una reunión o una serie puede usar varios diccionarios a la vez. Las palabras se añaden al corregirlas en una transcripción: pulsa una palabra y elige «Corregir».",
    ),
  ).toBeTruthy();
  fireEvent.click(screen.getByRole("button", { name: "Cambiar el nombre de Proyecto" }));
  const name = screen.getByRole("textbox", { name: "Nombre del diccionario" });
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
      section="reuniones"
      snapshot={snapshot("needs_hub")}
      send={vi.fn()}
      error={null}
      onReopenOnboarding={() => undefined}
    />,
  );
  expect(screen.getByText("No se pudo conectar con el hub")).toBeTruthy();
  expect(screen.queryByText(/Todavía no hay diccionarios/)).toBeNull();
});

it("Settings shows the dictionaries' shape before they are read", () => {
  render(
    <SettingsView
      section="reuniones"
      snapshot={snapshot("unknown")}
      send={vi.fn()}
      error={null}
      onReopenOnboarding={() => undefined}
    />,
  );
  const dictionaries = screen.getByRole("region", { name: "Diccionarios" });
  expect(dictionaries.querySelector("[aria-busy='true']")).toBeTruthy();
  expect(screen.queryByText(/Todavía no hay diccionarios/)).toBeNull();
});

const term = (id: string, correct: string, wrong: string[]) => ({ id, correct, wrong });

it("Settings shows several dictionaries, each in its own card with its words and its series", () => {
  const items = [
    {
      pending: false,
      dictionary: {
        id: "nubaria",
        name: "Nubaria",
        series: [
          { calendar_item_id: "weekly", title: "Revisión semanal" },
          { calendar_item_id: "board", title: "Comité" },
        ],
        terms: [
          term("t1", "Orbit", ["orvit", "órbit"]),
          term("t2", "Deimos", ["demos"]),
          term("t3", "Nubaria", ["nubaría"]),
          term("t4", "Piloto", ["pilotó"]),
          term("t5", "Ribera", ["rivera"]),
          term("t6", "Comité", ["comite"]),
          term("t7", "Kickoff", ["kikof"]),
        ],
      },
    },
    {
      pending: false,
      dictionary: {
        id: "product",
        name: "Producto",
        series: [],
        terms: [term("p1", "NAV", ["nab"])],
      },
    },
  ];
  render(
    <SettingsView
      section="reuniones"
      snapshot={snapshot("available", items)}
      send={vi.fn()}
      error={null}
      onReopenOnboarding={() => undefined}
    />,
  );
  const dictionaries = screen.getByRole("region", { name: "Diccionarios" });
  expect(within(dictionaries).getByText("2")).toBeTruthy();
  const nubaria = within(dictionaries).getByRole("group", { name: "Nubaria" });
  expect(within(nubaria).getByText("7 palabras")).toBeTruthy();
  expect(within(nubaria).getByText("Lo usan siempre: Revisión semanal, Comité")).toBeTruthy();
  expect(within(nubaria).getByText("en lugar de orvit, órbit")).toBeTruthy();
  // The first words, then the rest on asking.
  expect(within(nubaria).queryByText("Kickoff")).toBeNull();
  fireEvent.click(within(nubaria).getByRole("button", { name: "Ver las 7 palabras" }));
  expect(within(nubaria).getByText("Kickoff")).toBeTruthy();
  expect(within(nubaria).queryByRole("button", { name: /Ver las/ })).toBeNull();
  const product = within(dictionaries).getByRole("group", { name: "Producto" });
  expect(within(product).getByText("1 palabra")).toBeTruthy();
  expect(within(product).getByText("Ninguna serie lo usa siempre")).toBeTruthy();
  expect(within(product).queryByRole("button", { name: /Ver las/ })).toBeNull();
});
