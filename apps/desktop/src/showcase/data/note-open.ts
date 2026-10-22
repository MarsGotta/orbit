import type { NoteProgressSnapshot } from "../../bridge/generated/NoteProgressSnapshot";
import type { OpenNoteSnapshot } from "../../bridge/generated/OpenNoteSnapshot";
import type { NoteReadSnapshot } from "../../bridge/generated/NoteReadSnapshot";
import { REVIEW_ID } from "./notes-list";
import { speakers, turns } from "./transcript";
import { at, MINUTE } from "./time";

export const progressDone: NoteProgressSnapshot = {
  upload: { state: "done", cause: null },
  transcript: { state: "done", cause: null },
  summary: { state: "done", cause: null },
};

const sections = [
  {
    name: "Contexto",
    markdown:
      "El piloto con **Hoteles Ribera** arranca el lunes con cinco hoteles. Había que elegir entre el pago en tres pasos y el pago en una sola pantalla.",
    edited: false,
  },
  {
    name: "Lo que se habló",
    markdown: [
      "- En las pruebas con recepcionistas, la versión de **una pantalla** fue casi el doble de rápida.",
      "- La disponibilidad tarda unos **2 s** con cinco hoteles; con cincuenta no aguantaría.",
      "- El calendario semanal necesita un estado vacío que proponga la siguiente semana con hueco.",
      "- El cliente pidió bloquear habitaciones para grupos.",
    ].join("\n"),
    edited: false,
  },
  {
    name: "Próximos pasos",
    markdown:
      "1. Caché de tarifas por día antes del jueves.\n2. Estado vacío del calendario en el prototipo.\n3. Confirmar con Pablo la fecha de arranque.",
    edited: true,
  },
];

export const reviewNote: OpenNoteSnapshot = {
  capture: "closed",
  recording_id: REVIEW_ID,
  title: "Revisión del diseño de reservas",
  owner_name: "Lucía Ferrer",
  started_at_ms: at(-7, 11, 0),
  ended_at_ms: at(-7, 11, 47),
  event: {
    attendees: [
      { name: "Marta Ibáñez", email: "marta@nubaria.example", is_self: false },
      { name: "Lucía Ferrer", email: "lucia@nubaria.example", is_self: true },
      { name: "Diego Salinas", email: "diego@nubaria.example", is_self: false },
      { name: "Javier Ortega", email: "javier@nubaria.example", is_self: false },
    ],
    title: "Revisión del diseño de reservas",
    start_ms: at(-7, 11, 0),
    end_ms: at(-7, 11, 45),
  },
  summary: {
    short_summary:
      "Se elige el pago en una sola pantalla para el piloto y se reparte el trabajo pendiente antes del lunes.",
    sections,
    template_id: "design-review",
    template_name: "Revisión de diseño",
    generated_at_ms: at(-7, 11, 52),
    model: "hub",
    superseded_available: false,
  },
  note_revision: 3,
  actions: [
    {
      id: "a1",
      owner: "Javier Ortega",
      text: "Cachear las tarifas por día",
      due: "jueves",
      done: false,
    },
    {
      id: "a2",
      owner: "Marta Ibáñez",
      text: "Añadir el estado vacío al calendario semanal",
      due: null,
      done: true,
    },
    {
      id: "a3",
      owner: "Lucía Ferrer",
      text: "Confirmar con Pablo la fecha de arranque del piloto",
      due: "viernes",
      done: false,
    },
    {
      id: "a4",
      owner: "Lucía Ferrer",
      text: "Enviar el resumen al equipo de Ribera",
      due: null,
      done: true,
    },
    { id: "a5", owner: null, text: "Revisar los textos del paso de pago", due: null, done: false },
  ],
  decisions: [
    "El piloto usa el pago en una sola pantalla.",
    "No se toca el modelo de habitaciones hasta después del piloto.",
  ],
  open_topics: [
    "Bloqueo de habitaciones para grupos",
    "Precio del plan para cadenas de más de 20 hoteles",
  ],
  private_notes:
    "## Para mí\n\n- Preguntar a Pablo si **Sara** puede venir a la demo.\n- [ ] Preparar la tabla de precios para cadenas\n- [x] Reservar sala para el lunes",
  private_notes_revision: 2,
  progress: progressDone,
  transcript: {
    final_pass: { state: "completed", error: null },
    language: "es",
    turns,
    pending_chunks: 0,
    failed_groups: 0,
  },
  speakers,
  series: {
    calendar_item_id: "item-design-review",
    assigned: { state: "known", template_id: "design-review" },
  },
  dictionary: {
    dictionary_id: "dict-nubaria",
    source: "series",
    series: { calendar_item_id: "item-design-review", title: "Revisión del diseño de reservas" },
  },
};

export function readOf(note: OpenNoteSnapshot): NoteReadSnapshot {
  return {
    recording_id: note.recording_id,
    revision: note.note_revision ?? 0,
    state: "available",
    detail: null,
    read_at_ms: note.ended_at_ms + 10 * MINUTE,
  };
}
