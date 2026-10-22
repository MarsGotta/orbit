import { withOpenNote } from "../base";
import { REVIEW_ID } from "../data/notes-list";
import { writingNote } from "../data/note-variants";
import { at, MINUTE } from "../data/time";
import type { Scenario, Step, Views } from "../scenario";

export const reuniones = (scenario: Omit<Scenario, "view">): Scenario => ({
  view: "Reuniones",
  ...scenario,
});

export const tab = (name: string) => `role=tab[name^='${name}']`;
const opened = (views: Views) => withOpenNote(views);

export const listed = (base: Views, change: Partial<NonNullable<Views["notes"]>>): Views => ({
  ...base,
  notes: { ...(base.notes ?? { list: [], open: null }), ...change },
});

/** Today's review at the top of the list, being written. */
export const withToday = (base: Views, phase: "transcribing" | "ready"): Views => {
  const list = base.notes?.list ?? [];
  const row = {
    recording_id: writingNote.recording_id,
    title: writingNote.title,
    started_at_ms: at(0, 11, 0),
    duration_ms: 47 * MINUTE,
    state: phase === "ready" ? ("ready" as const) : ("transcribing" as const),
    group: "today" as const,
    headline:
      phase === "ready"
        ? { phase: "ready" as const, preview: "Se elige el pago en una sola pantalla" }
        : { phase: "transcribing" as const },
  };
  return listed(base, { list: [row, ...list] });
};

export const transcriptSteps: Step[] = [{ click: tab("Transcripción") }];

export const notesScenarios: Scenario[] = [
  reuniones({
    id: "reuniones",
    state: "Lista de reuniones",
    description:
      "Las reuniones por días, con su línea: el resumen, «Transcribiendo…» o el fallo; ninguna abierta.",
    route: "#/notes",
    ready: "text=Planificación del trimestre",
  }),
  reuniones({
    id: "reuniones-vacia",
    state: "Sin reuniones",
    description: "Antes de la primera grabación: qué hacer para empezar.",
    route: "#/notes",
    views: (base) =>
      listed(base, { list: [], index: { state: "available", more: false, detail: null } }),
    ready: "text=Aún no hay reuniones",
  }),
  reuniones({
    id: "reuniones-cargando",
    state: "Leyendo del hub",
    description: "La lista mientras se lee del hub: indicador con texto y filas en esqueleto.",
    route: "#/notes",
    views: (base) =>
      listed(base, { list: [], index: { state: "loading", more: false, detail: null } }),
    ready: "text=Leyendo tus reuniones del hub…",
  }),
  reuniones({
    id: "nota-cargando",
    state: "Nota · Leyendo del hub",
    description: "Una reunión abierta cuya nota aún se lee: su forma en esqueleto.",
    route: `#/notes/${REVIEW_ID}`,
    views: (base) => ({
      ...base,
      notes: base.notes && {
        ...base.notes,
        open: null,
        open_read: {
          recording_id: REVIEW_ID,
          revision: 0,
          state: "loading",
          detail: null,
          read_at_ms: null,
        },
      },
    }),
    ready: "text=Leyendo la nota del hub…",
  }),
  reuniones({
    id: "nota-escribiendose",
    state: "Nota escribiéndose",
    description:
      "Audio guardado, transcribiendo y el resumen por escribir; la transcripción se va rellenando.",
    route: `#/notes/${writingNote.recording_id}`,
    views: (base) => withOpenNote(withToday(base, "transcribing"), writingNote),
    ready: "text=Orbit está preparando tu nota",
  }),
  reuniones({
    id: "nota-resumen",
    state: "Nota · Resumen",
    description: "Una nota escrita: cabecera, plantilla, secciones, decisiones y quién habló.",
    route: `#/notes/${REVIEW_ID}`,
    views: opened,
    ready: tab("Resumen"),
  }),
  reuniones({
    id: "nota-acciones",
    state: "Nota · Acciones",
    description: "Las tareas por persona, las tuyas primero, con su casilla y su fecha.",
    route: `#/notes/${REVIEW_ID}`,
    views: opened,
    ready: tab("Acciones"),
    steps: [{ click: tab("Acciones") }],
  }),
  reuniones({
    id: "nota-transcripcion",
    state: "Nota · Transcripción",
    description: "La transcripción por turnos, desde el principio, con su buscador.",
    route: `#/notes/${REVIEW_ID}`,
    views: opened,
    ready: tab("Transcripción"),
    steps: transcriptSteps,
  }),
  reuniones({
    id: "nota-buscar",
    state: "Nota · Buscar en la transcripción",
    description: "La búsqueda con el número de resultados y las coincidencias resaltadas.",
    route: `#/notes/${REVIEW_ID}`,
    views: opened,
    ready: tab("Transcripción"),
    steps: [
      ...transcriptSteps,
      { fill: "role=searchbox[name='Buscar en la transcripción']", text: "caché" },
    ],
  }),
  reuniones({
    id: "nota-privadas",
    state: "Nota · Notas privadas",
    description: "Lo apuntado durante la reunión, que solo ve quien la grabó.",
    route: `#/notes/${REVIEW_ID}`,
    views: opened,
    ready: tab("Notas privadas"),
    steps: [{ click: tab("Notas privadas") }, { wait: "[contenteditable=true]" }],
  }),
];
