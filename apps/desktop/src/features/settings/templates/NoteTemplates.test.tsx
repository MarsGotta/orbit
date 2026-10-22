import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import type { NoteLibrarySnapshot } from "../../../bridge/generated/NoteLibrarySnapshot";
import { NoteTemplates } from "./NoteTemplates";

// Ajustes › Reuniones › Plantillas. Invented templates.

const SECTIONS = [
  { name: "Perfil", instructions: "Experiencia que ha contado" },
  { name: "Cómo trabaja", instructions: "Ejemplos de problemas resueltos" },
  { name: "Impresión", instructions: "Puntos fuertes y dudas" },
];

function library(state: "available" | "unknown" = "available"): NoteLibrarySnapshot {
  return {
    read: "available",
    read_detail: null,
    bounds: {
      max_sections: 12,
      max_name_chars: 80,
      max_context_chars: 4000,
      max_instructions_chars: 2000,
      max_custom_templates: 32,
    },
    templates: { state, detail: null },
    items:
      state === "available"
        ? [
            {
              id: "general",
              name: "Reunión general",
              context: "",
              sections: [{ name: "Resumen", instructions: "" }],
              builtin: true,
              forked_from: null,
              pending: false,
            },
            {
              id: "entrevista",
              name: "Entrevista",
              context: "Una persona candidata.",
              sections: SECTIONS,
              builtin: false,
              forked_from: null,
              pending: false,
            },
          ]
        : [],
    links: { state, detail: null },
    assignments: [],
    refusals: [],
    restores: [],
    dictionary_items: [],
    dictionary_pending: [],
    dictionary_failures: [],
  } as NoteLibrarySnapshot;
}

afterEach(cleanup);

it("lists Orbit's templates marked «de Orbit» and edits one that saves itself", () => {
  render(<NoteTemplates library={library()} send={vi.fn()} />);
  const list = screen.getByRole("list", { name: "Plantillas" });
  expect(within(list).getByRole("button", { name: "Reunión general de Orbit" })).toBeTruthy();
  fireEvent.click(within(list).getByRole("button", { name: "Entrevista" }));
  const editor = screen.getByRole("region", { name: "Entrevista" });
  expect(within(editor).getByText("Se guarda sola")).toBeTruthy();
  expect(within(editor).getByText("Secciones de la nota")).toBeTruthy();
  expect(within(editor).getByText("3 de 12")).toBeTruthy();
  expect(within(editor).getByRole("button", { name: "Borrar plantilla" })).toBeTruthy();
});

it("moves a section with the keyboard from its handle, and saves the new order", async () => {
  const send = vi.fn();
  render(<NoteTemplates library={library()} send={send} />);
  fireEvent.click(screen.getByRole("button", { name: "Entrevista" }));
  fireEvent.keyDown(screen.getByRole("button", { name: /^Mover «Perfil»/ }), {
    key: "ArrowDown",
  });
  expect(
    screen.getAllByRole("textbox", { name: /^Nombre de la sección/ }).map((input) => {
      return (input as HTMLInputElement).value;
    }),
  ).toEqual(["Cómo trabaja", "Perfil", "Impresión"]);
  await waitFor(() =>
    expect(send).toHaveBeenCalledWith({
      command: "note_library",
      request: {
        action: "save_template",
        template_id: "entrevista",
        draft: {
          name: "Entrevista",
          context: "Una persona candidata.",
          sections: [SECTIONS[1], SECTIONS[0], SECTIONS[2]],
          forked_from: null,
        },
      },
    }),
  );
});

it("says why no section can be added once there are twelve", () => {
  const full = library();
  const own = full.items[1];
  if (!own) throw new Error("fixture template missing");
  own.sections = Array.from({ length: 12 }, (_, index) => ({
    name: `Parte ${index + 1}`,
    instructions: "",
  }));
  render(<NoteTemplates library={full} send={vi.fn()} />);
  fireEvent.click(screen.getByRole("button", { name: "Entrevista" }));
  expect(screen.getByText("12 de 12")).toBeTruthy();
  expect(screen.getByRole<HTMLButtonElement>("button", { name: "Añadir sección" }).disabled).toBe(
    true,
  );
  expect(screen.getByText("12 secciones como máximo. Quita una para añadir otra.")).toBeTruthy();
});

it("shows the templates' shape while they have not been read", () => {
  render(<NoteTemplates library={library("unknown")} send={vi.fn()} />);
  const templates = screen.getByRole("region", { name: "Plantillas" });
  expect(templates.querySelector("[aria-busy='true']")).toBeTruthy();
});
