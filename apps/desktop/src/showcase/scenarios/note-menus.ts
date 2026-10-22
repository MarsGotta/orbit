import { withOpenNote } from "../base";
import { REVIEW_ID } from "../data/notes-list";
import {
  failedNote,
  recordingNote,
  speakerNote,
  unwrittenNote,
  writingNote,
} from "../data/note-variants";
import { recordingMeeting } from "../data/recording";
import type { Scenario, Views } from "../scenario";
import { reuniones, tab, transcriptSteps, withToday } from "./notes";
import { withMeeting } from "./recording";

// Reuniones' menus, windows, failures and notices.

const opened = (views: Views) => withOpenNote(views);
const menu = "role=button[name^='Plantilla:']";
const persona2 = "role=button[name='Persona 2 · ponle nombre']";

export const noteMenuScenarios: Scenario[] = [
  reuniones({
    id: "nota-plantilla",
    state: "Nota · Cómo se escribe",
    description:
      "Plantilla, idioma, diccionarios y «Usar lo mismo en toda la serie», sin reescribir.",
    route: `#/notes/${REVIEW_ID}`,
    views: opened,
    ready: menu,
    steps: [{ click: menu }],
  }),
  reuniones({
    id: "nota-plantilla-reescribir",
    state: "Nota · Reescribir la nota",
    description: "Otra plantilla elegida: lo que pasará y «Reescribir la nota» antes de nada.",
    route: `#/notes/${REVIEW_ID}`,
    views: opened,
    ready: menu,
    steps: [{ click: menu }, { press: "ArrowUp" }, { wait: "text=Reescribir la nota" }],
  }),
  reuniones({
    id: "nota-hablante",
    state: "Nota · ¿Quién es?",
    description: "El menú de una voz sin nombre: invitados, voces de la grabación y añadir.",
    route: `#/notes/${speakerNote.recording_id}`,
    views: (base) => withOpenNote(withToday(base, "ready"), speakerNote),
    ready: tab("Transcripción"),
    steps: [...transcriptSteps, { click: persona2 }],
  }),
  reuniones({
    id: "nota-hablante-aplicar",
    state: "Nota · ¿A qué turnos?",
    description: "El último paso del menú: solo este turno o todos los de esa voz.",
    route: `#/notes/${speakerNote.recording_id}`,
    views: (base) => withOpenNote(withToday(base, "ready"), speakerNote),
    ready: tab("Transcripción"),
    steps: [
      ...transcriptSteps,
      { click: persona2 },
      { click: "role=dialog >> role=button[name='Diego Salinas']" },
      { click: "role=radio[name^='En los 3 turnos']" },
    ],
  }),
  reuniones({
    id: "nota-corregir",
    state: "Nota · Corregir un texto seleccionado",
    description: "Con el teclado: el texto seleccionado y «Corregir «…»» abren el mismo menú.",
    route: `#/notes/${speakerNote.recording_id}`,
    views: (base) => withOpenNote(withToday(base, "ready"), speakerNote),
    ready: tab("Transcripción"),
    steps: [
      ...transcriptSteps,
      { fill: "role=searchbox[name='Buscar en la transcripción']", text: "Orvit" },
      { select: "mark" },
      { click: "role=button[name='Corregir «Orvit»']" },
    ],
  }),
  reuniones({
    id: "nota-no-escrita",
    state: "Nota · No se pudo escribir",
    description: "Sin diseño propio: el motivo y «Reintentar» en la misma nota.",
    route: `#/notes/${unwrittenNote.recording_id}`,
    views: (base) => withOpenNote(base, unwrittenNote),
    ready: "text=No se pudo escribir la nota",
  }),
  reuniones({
    id: "nota-transcripcion-fallo",
    state: "Nota · Transcripción con fallo",
    description: "Parte del audio no se pudo transcribir: lo dice y ofrece reintentar.",
    route: `#/notes/${failedNote.recording_id}`,
    views: (base) => withOpenNote(base, failedNote),
    ready: "text=No se pudo transcribir toda la reunión",
  }),
  reuniones({
    id: "nota-no-abre",
    state: "Nota · No se pudo abrir",
    description: "Una nota que el hub no pudo leer: el motivo y «Reintentar».",
    route: `#/notes/${REVIEW_ID}`,
    views: (base) => {
      const views = withOpenNote(base);
      const notes = views.notes ?? { list: [], open: null };
      const read = notes.open_read && {
        ...notes.open_read,
        state: "failed" as const,
        detail: "hub_unreachable",
      };
      return { ...views, notes: { ...notes, open_read: read } };
    },
    ready: "text=No se pudo abrir la nota",
  }),
  reuniones({
    id: "nota-grabando",
    state: "Nota · Reunión en curso",
    description: "La nota de la reunión que se graba: se abre en las notas privadas.",
    route: `#/notes/${recordingNote.recording_id}`,
    views: (base) => withOpenNote(withMeeting(base, recordingMeeting), recordingNote),
    ready: "[contenteditable=true]",
  }),
  reuniones({
    id: "nota-lista-aviso",
    state: "Aviso · Tu nota está lista",
    description: "El aviso cuando el resumen termina, con «Abrir».",
    route: `#/notes/${writingNote.recording_id}`,
    views: (base) => withOpenNote(withToday(base, "transcribing"), writingNote),
    later: (views) => ({
      notes: views.notes && {
        ...views.notes,
        ready: { recording_id: writingNote.recording_id, title: writingNote.title, sequence: 1 },
      },
    }),
    ready: "text=Tu nota está lista",
  }),
];
