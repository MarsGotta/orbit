import type { DictionaryEntry } from "../../bridge/generated/DictionaryEntry";
import type { NoteLibrarySnapshot } from "../../bridge/generated/NoteLibrarySnapshot";
import type { OpenNoteSnapshot } from "../../bridge/generated/OpenNoteSnapshot";
import { library } from "./library";
import { reviewNote } from "./note-open";
import { at } from "./time";

// The annex's transcript (changes 5 to 7): today's design review says
// «Orvit» three times, and the meeting uses two of the three dictionaries.

const SECOND = 1_000;
const said: Array<[number, string, string]> = [
  [8, "mic_local", "Empezamos. Marta, ¿cómo va el prototipo del pago en una sola pantalla?"],
  [
    21,
    "remote_1",
    "Orvit ya lo tiene. Lo probamos ayer con tres recepcionistas y lo terminaron todas sin ayuda.",
  ],
  [47, "remote_2", "¿Y la disponibilidad? La semana pasada tardaba unos dos segundos."],
  [
    65,
    "remote_3",
    "Con la caché por día baja a unos trescientos milisegundos con los cinco hoteles.",
  ],
  [92, "mic_local", "Perfecto. Lo dejo apuntado en Orvit para el lunes."],
  [110, "remote_2", "Yo preparo la demo con Ribera Aeropuerto si llega la caché."],
  [130, "remote_1", "¿Las notas de Orvit ya salen con el diseño nuevo?"],
  [144, "mic_local", "Sí, desde esta semana. Escribe el resumen solo."],
];

export const wordsNote: OpenNoteSnapshot = {
  ...reviewNote,
  recording_id: "rec-review-today",
  started_at_ms: at(0, 11, 0),
  ended_at_ms: at(0, 11, 47),
  transcript: {
    ...reviewNote.transcript,
    turns: said.map(([seconds, speaker_key, text], index) => ({
      index,
      at_ms: seconds * SECOND,
      speaker_key,
      text,
    })),
  },
  dictionary: {
    dictionary_id: "dict-nubaria",
    dictionary_ids: ["dict-nubaria", "dict-ribera"],
    source: "series",
    series: { calendar_item_id: "item-design-review", title: "Revisión del diseño de reservas" },
  },
};

/** As many invented words as the design says each dictionary has. */
function words(prefix: string, count: number): DictionaryEntry[] {
  return Array.from({ length: count }, (_, index) => ({
    id: `${prefix}-${index}`,
    correct: `${prefix} ${index + 1}`,
    wrong: [`${prefix.toLowerCase()}${index + 1}`],
  }));
}

export const wordsLibrary: NoteLibrarySnapshot = {
  ...library,
  dictionary_items: [
    {
      dictionary: {
        id: "dict-nubaria",
        name: "Nubaria",
        terms: words("Nubaria", 24),
        series: [
          { calendar_item_id: "item-weekly", title: "Revisión semanal" },
          { calendar_item_id: "item-board", title: "Comité" },
        ],
      },
      pending: false,
    },
    {
      dictionary: {
        id: "dict-ribera",
        name: "Hoteles Ribera",
        terms: words("Ribera", 11),
        series: [{ calendar_item_id: "item-pilot", title: "Seguimiento del piloto" }],
      },
      pending: false,
    },
    {
      dictionary: { id: "dict-product", name: "Producto", terms: words("Producto", 8), series: [] },
      pending: false,
    },
  ],
};

/** The library once a correction is in each of these dictionaries. */
export function withTerm(ids: string[], wrong: string, correct: string): NoteLibrarySnapshot {
  return {
    ...wordsLibrary,
    dictionary_items: wordsLibrary.dictionary_items.map((item) =>
      ids.includes(item.dictionary.id)
        ? {
            ...item,
            dictionary: {
              ...item.dictionary,
              terms: [...item.dictionary.terms, { id: `fix-${wrong}`, correct, wrong: [wrong] }],
            },
          }
        : item,
    ),
  };
}
