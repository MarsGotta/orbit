import { withOpenNote } from "../base";
import { REVIEW_ID } from "../data/notes-list";
import { failedNote, recordingNote, transcribingNote } from "../data/note-variants";
import { recordingMeeting } from "../data/recording";
import type { Scenario, Views } from "../scenario";
import { withMeeting } from "./recording";

const notes = (scenario: Omit<Scenario, "view">): Scenario => ({
  view: "Reuniones y notas",
  ...scenario,
});

const tab = (name: string) => `role=tab[name='${name}']`;
const opened = (views: Views) => withOpenNote(views);

const listed = (base: Views, change: Partial<NonNullable<Views["notes"]>>): Views => ({
  ...base,
  notes: { ...(base.notes ?? { list: [], open: null }), ...change },
});

export const notesScenarios: Scenario[] = [
  notes({
    id: "notas",
    state: "Lista de notas",
    description:
      "Las reuniones grabadas por días, con su estado: lista, transcribiéndose o con fallo.",
    route: "#/notes",
    ready: "text=Planificación del trimestre",
  }),
  notes({
    id: "notas-vacia",
    state: "Sin notas",
    description: "Notas antes de la primera grabación: qué hacer para empezar.",
    route: "#/notes",
    views: (base) =>
      listed(base, { list: [], index: { state: "available", more: false, detail: null } }),
    ready: "text=Aún no hay notas",
  }),
  notes({
    id: "notas-cargando",
    state: "Cargando del hub",
    description: "La lista mientras se lee del hub por primera vez.",
    route: "#/notes",
    views: (base) =>
      listed(base, { list: [], index: { state: "loading", more: false, detail: null } }),
    ready: "text=Cargando notas…",
  }),
  notes({
    id: "nota-resumen",
    state: "Nota · Resumen",
    description:
      "Una nota escrita: título, resumen corto, plantilla, idioma, diccionario y sus secciones.",
    route: `#/notes/${REVIEW_ID}`,
    views: opened,
    ready: tab("Resumen"),
  }),
  notes({
    id: "nota-acciones",
    state: "Nota · Acciones",
    description: "Las tareas de la reunión, agrupadas por persona, las mías primero.",
    route: `#/notes/${REVIEW_ID}`,
    views: opened,
    ready: tab("Acciones"),
    steps: [{ click: tab("Acciones") }],
  }),
  notes({
    id: "nota-privadas",
    state: "Nota · Notas privadas",
    description:
      "Las notas propias de la reunión, en un editor con formato, que nunca ve nadie más.",
    route: `#/notes/${REVIEW_ID}`,
    views: opened,
    ready: tab("Notas privadas"),
    steps: [{ click: tab("Notas privadas") }, { wait: "[contenteditable=true]" }],
  }),
  notes({
    id: "nota-transcripcion",
    state: "Nota · Transcripción",
    description: "La transcripción por turnos, con quién habla, cuándo y cuánto habló cada voz.",
    route: `#/notes/${REVIEW_ID}`,
    views: opened,
    ready: tab("Transcripción"),
    steps: [
      { click: tab("Transcripción") },
      { wait: "text=/Traigo dos versiones/" },
      { scroll: "[class*=noteBody]" },
    ],
  }),
  notes({
    id: "nota-buscar",
    state: "Nota · Buscar en la transcripción",
    description:
      "La búsqueda en la transcripción (⌘F), con las coincidencias marcadas y su cuenta.",
    route: `#/notes/${REVIEW_ID}`,
    views: opened,
    ready: tab("Transcripción"),
    steps: [
      { click: tab("Transcripción") },
      { press: "Control+f" },
      { fill: "role=textbox[name='Buscar en la transcripción']", text: "caché" },
      { scroll: "[class*=noteBody]" },
    ],
  }),
  notes({
    id: "nota-hablante",
    state: "Nota · Cambiar quién habla",
    description: "El menú de una voz de la transcripción, para ponerle nombre o cambiarlo.",
    route: `#/notes/${REVIEW_ID}`,
    views: opened,
    ready: tab("Transcripción"),
    steps: [{ click: tab("Transcripción") }, { click: "role=button[name='Diego Salinas']" }],
  }),
  notes({
    id: "nota-transcribiendo",
    state: "Nota · Transcribiéndose",
    description:
      "Una reunión subida que aún se transcribe: el progreso de cada paso y la nota por escribir.",
    route: `#/notes/${transcribingNote.recording_id}`,
    views: (base) => withOpenNote(base, transcribingNote),
    ready: tab("Transcripción"),
  }),
  notes({
    id: "nota-grabando",
    state: "Nota · Reunión en curso",
    description:
      "La nota de la reunión que se graba ahora: se abre en las notas privadas para apuntar.",
    route: `#/notes/${recordingNote.recording_id}`,
    views: (base) => withOpenNote(withMeeting(base, recordingMeeting), recordingNote),
    ready: "[contenteditable=true]",
  }),
  notes({
    id: "nota-fallo",
    state: "Nota · Transcripción con fallo",
    description: "Parte del audio no se pudo transcribir: lo dice y ofrece reintentar.",
    route: `#/notes/${failedNote.recording_id}`,
    views: (base) => withOpenNote(base, failedNote),
    ready: "role=status >> text=Reintentar",
  }),
  notes({
    id: "nota-no-abre",
    state: "Nota · No se pudo abrir",
    description: "Una nota que el hub no pudo leer: el aviso y el botón para volver a intentarlo.",
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
    ready: "text=No se pudo abrir la nota.",
  }),
];
