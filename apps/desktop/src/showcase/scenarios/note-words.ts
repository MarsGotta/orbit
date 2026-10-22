import type { AppCommand } from "../../bridge/generated";
import type { DictionaryRequest } from "../../bridge/generated/DictionaryRequest";
import type { NoteLibrarySnapshot } from "../../bridge/generated/NoteLibrarySnapshot";
import type { OpenNoteSnapshot } from "../../bridge/generated/OpenNoteSnapshot";
import { withOpenNote } from "../base";
import { wordsLibrary, wordsNote, withTerm } from "../data/note-words";
import type { Scenario, Step, Views } from "../scenario";
import { reuniones, tab, transcriptSteps, withToday } from "./notes";

// The annex's transcript (changes 5 and 6): a word under the pointer, its
// menu, the correction and its undo, and several dictionaries in the note's
// menu. Transcribing again and rewriting the summary are in note-redo.ts.

export const route = `#/notes/${wordsNote.recording_id}`;
const turn = "p:has-text('Orvit ya lo tiene')";
/** «Orvit», the first word of the turn at 00:21. */
const onOrvit: [number, number] = [14, 12];
const how = "role=button[name^='Plantilla:']";

export function opened(
  base: Views,
  note: OpenNoteSnapshot = wordsNote,
  library = wordsLibrary,
): Views {
  const views = withOpenNote(withToday(base, "ready"), note);
  return { ...views, notes: views.notes && { ...views.notes, library } };
}

export function withPass(final_pass: NonNullable<OpenNoteSnapshot["transcript"]["final_pass"]>) {
  return { ...wordsNote, transcript: { ...wordsNote.transcript, final_pass } };
}

/** The daemon's answer to a dictionary order, as the library read brings it. */
export function answered(views: Views, library: NoteLibrarySnapshot, note?: OpenNoteSnapshot) {
  const shown = note ?? views.notes?.open ?? wordsNote;
  const next = withOpenNote(views, shown);
  return { note: next.note, notes: next.notes && { ...next.notes, library } };
}

function dictionaryOrder(command: AppCommand): DictionaryRequest | null {
  return command.command === "note_library" && command.request.action === "dictionary"
    ? command.request.request
    : null;
}

/** The hub took the correction: every dictionary has it, the transcript reads it. */
function corrects(views: Views, command: AppCommand, extra: Partial<NoteLibrarySnapshot> = {}) {
  const order = dictionaryOrder(command);
  if (order?.action === "remove_correction") return answered(views, wordsLibrary, wordsNote);
  if (order?.action !== "correct") return null;
  const ids = order.dictionary_ids ?? [];
  const note = {
    ...wordsNote,
    transcript: {
      ...wordsNote.transcript,
      turns: wordsNote.transcript.turns.map((one) => ({
        ...one,
        text: one.text.replaceAll(order.wrong, order.correct),
      })),
    },
  };
  return answered(views, { ...withTerm(ids, order.wrong, order.correct), ...extra }, note);
}

const pressOrvit: Step[] = [
  ...transcriptSteps,
  { mouse: turn, at: onOrvit },
  { fill: "role=textbox[name='Debe decir']", text: "Orbit" },
];
const correctIt: Step[] = [
  ...pressOrvit,
  { click: "role=button[name='Corregir y añadir']" },
  { wait: "text=ahora es «Orbit»" },
];

export const noteWordScenarios: Scenario[] = [
  reuniones({
    id: "nota-palabra-pasar",
    state: "Transcripción · Pasar el ratón",
    description: "Cada palabra responde al ratón con «Pulsa para corregir».",
    route,
    views: (base) => opened(base),
    ready: tab("Transcripción"),
    steps: [...transcriptSteps, { hover: turn, at: onOrvit }],
  }),
  reuniones({
    id: "nota-palabra-corregir",
    state: "Transcripción · Corregir una palabra",
    description: "El menú junto a la palabra: cuántas veces sale y en qué diccionarios entra.",
    route,
    views: (base) => opened(base),
    ready: tab("Transcripción"),
    steps: [...pressOrvit, { click: "role=button[name^='Se añade a']" }],
  }),
  reuniones({
    id: "nota-palabra-corregida",
    state: "Transcripción · Corregida",
    description: "Las tres apariciones cambian y la franja ofrece «Deshacer» y «Rehacer resumen».",
    route,
    views: (base) => opened(base),
    react: (views, command) => corrects(views, command),
    ready: tab("Transcripción"),
    steps: correctIt,
  }),
  reuniones({
    id: "nota-palabra-deshecha",
    state: "Transcripción · Corrección deshecha",
    description: "«Deshacer» la quita de todos los diccionarios, sin preguntar.",
    route,
    views: (base) => opened(base),
    react: (views, command) => corrects(views, command),
    ready: tab("Transcripción"),
    steps: [
      ...correctIt,
      { click: "role=button[name='Deshacer']" },
      { wait: "text=Corrección deshecha" },
    ],
  }),
  reuniones({
    id: "nota-palabra-conflicto",
    state: "Transcripción · Dos diccionarios no coinciden",
    description:
      "Sin diseño propio: otro diccionario oye la palabra de otro modo y gana el primero.",
    route,
    views: (base) => opened(base),
    react: (views, command) =>
      corrects(views, command, {
        dictionary_conflicts: [
          {
            target: "dictionaries:dict-nubaria,dict-ribera",
            conflicts: [
              {
                wrong: "Orvit",
                correct: "Orbit",
                dictionary_id: "dict-ribera",
                other_dictionary_id: "dict-product",
                other_name: "Producto",
                other_correct: "Órbita",
                applied_dictionary_id: "dict-nubaria",
              },
            ],
          },
        ],
      }),
    ready: tab("Transcripción"),
    steps: correctIt,
  }),
  reuniones({
    id: "nota-palabra-lleno",
    state: "Transcripción · Diccionario lleno",
    description:
      "Sin diseño propio: el hub no la guardó y lo dice con sus números; lo escrito se queda.",
    route,
    views: (base) => opened(base),
    react: (views, command) =>
      dictionaryOrder(command)?.action === "correct"
        ? answered(views, {
            ...wordsLibrary,
            dictionary_failures: [
              {
                target: "dictionaries:dict-nubaria,dict-ribera",
                code: "dictionary_full",
                detail: "",
                numbers: { terms: 128, max_terms: 128 },
              },
            ],
          })
        : null,
    ready: tab("Transcripción"),
    steps: [
      ...pressOrvit,
      { click: "role=button[name='Corregir y añadir']" },
      { wait: "text=El diccionario está lleno" },
    ],
  }),
  reuniones({
    id: "nota-diccionarios",
    state: "Nota · Varios diccionarios",
    description: "«Diccionarios de palabras» elige varios; la cabecera dice cuántos.",
    route,
    views: (base) => opened(base),
    ready: how,
    steps: [{ click: how }, { click: "role=button[name^='Diccionarios de palabras']" }],
  }),
];
