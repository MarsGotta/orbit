import { withOpenNote } from "../base";
import { reviewNote } from "../data/note-open";
import { REVIEW_ID } from "../data/notes-list";
import type { Scenario, Step, Views } from "../scenario";
import { reuniones, tab } from "./notes";

/** The private notes the design writes in, in its order (captures 25–30). */
const LINES = [
  "## Para la llamada con Hoteles Ribera",
  "Confirmar con **Pablo Medina** la fecha de arranque: el *lunes 26*.",
  "- [x] Repasar la demo de [[Revisión del diseño de reservas]]",
  "- [ ] Pedir a @Javier Ortega la caché de tarifas",
  "- [ ] Preparar la formación de ==Ribera Playa==",
  "> Marta: «la versión de una pantalla fue casi el doble de rápida».",
  "### Dudas",
  "1. ¿Bloqueamos habitaciones para grupos?",
  "2. ¿Quién firma el contrato del piloto?",
  "```bash",
  "python3 exportar_tarifas.py --semana 43",
  "```",
];

/** The design's note with `line` written at `at`, as it is being typed. */
function written(line?: string, at = 0): string {
  const lines = [...LINES];
  if (line !== undefined) lines.splice(at, 0, line);
  return lines.join("\n");
}

const withPrivateNotes = (markdown: string) => (base: Views) =>
  withOpenNote(base, { ...reviewNote, private_notes: markdown });

const TEXT = "role=textbox[name='Notas privadas']";
const line = (index: number) => `${TEXT} >> .cm-line >> nth=${index}`;
const openPrivate: Step[] = [{ click: tab("Notas privadas") }, { wait: TEXT }];
/** The cursor at the end of a line, as when it is being written. */
const writing = (index: number): Step[] => [
  ...openPrivate,
  { click: line(index) },
  { press: "End" },
];

const editor = (scenario: Omit<Scenario, "view" | "route" | "ready">): Scenario =>
  reuniones({ route: `#/notes/${REVIEW_ID}`, ready: tab("Notas privadas"), ...scenario });

export const editorScenarios: Scenario[] = [
  editor({
    id: "editor-escribiendo",
    state: "Editor · escribiendo",
    description:
      "Títulos, negrita, cursiva, tareas, una mención, un enlace a otra nota, resaltado, cita, lista numerada y código; la línea que se escribe enseña su Markdown en gris.",
    views: withPrivateNotes(written("- [ ] Llevar la tabla de **precios por cadena", 5)),
    steps: writing(5),
  }),
  editor({
    id: "editor-insertar",
    state: "Editor · insertar con «/»",
    description:
      "El menú «Insertar» bajo la línea, cada opción con su Markdown a la derecha; se filtra al escribir y se maneja con ↑ ↓ ↵ y Esc.",
    views: withPrivateNotes(written("/", 6)),
    steps: [...writing(6), { wait: "role=listbox[name='Insertar']" }],
  }),
  editor({
    id: "editor-formato",
    state: "Editor · formato de una selección",
    description:
      "La barra sobre el texto elegido: negrita, cursiva, código, enlazar a una nota, resaltar y «Preguntar a Deimos». El texto elegido, con --o-sel.",
    views: withPrivateNotes(written()),
    steps: [
      ...openPrivate,
      { hold: line(1), at: [208, 12], by: [146, 0] },
      { wait: "role=toolbar[name='Dar formato al texto elegido']" },
    ],
  }),
  editor({
    id: "editor-enlazar",
    state: "Editor · enlazar una nota con «[[»",
    description:
      "Las notas de las reuniones, filtradas al escribir; al elegir una queda «[[Título]]» y se ve como enlace.",
    views: withPrivateNotes(written("- [ ] Revisar las tarifas de [[Hot", 5)),
    steps: [...writing(5), { wait: "role=listbox[name='Enlazar una nota']" }],
  }),
  editor({
    id: "editor-markdown",
    state: "Editor · ver el Markdown",
    description:
      "El texto en bruto, en JetBrains Mono, con «Volver al texto»; lo que se escribe en una vista está en la otra (⌘/).",
    views: withPrivateNotes(written()),
    steps: [...openPrivate, { click: "role=button[name=/Markdown/]" }, { hover: ".cm-content" }],
  }),
  editor({
    id: "editor-vacio",
    state: "Editor · nota vacía",
    description: "«Escribe o pulsa / para insertar…» en la primera línea, y la ayuda debajo.",
    views: withPrivateNotes(""),
    steps: openPrivate,
  }),
  editor({
    id: "editor-mencionar",
    state: "Editor · mencionar con «@»",
    description:
      "Sin dibujar: los invitados de la reunión, quién habló en ella y las personas del calendario.",
    views: withPrivateNotes(written("- [ ] Pedir a @Ja", 5)),
    steps: [...writing(5), { wait: "role=listbox[name='Mencionar']" }],
  }),
  editor({
    id: "editor-resumen",
    state: "Editor · una sección del Resumen",
    description:
      "Sin dibujar: una sección del Resumen se edita con el mismo editor; «✓ Guardado» cuando la nota trae el cambio.",
    views: (base) => withOpenNote(base),
    steps: [
      { click: tab("Resumen") },
      { click: "role=button[name^='Editar «'] >> nth=0" },
      { wait: "role=textbox[name^='Texto de «']" },
    ],
  }),
];
