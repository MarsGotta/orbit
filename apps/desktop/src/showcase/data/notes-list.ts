import type { NoteListItem } from "../../bridge/generated/NoteListItem";
import { at, HOUR, MINUTE } from "./time";

type Draft = Pick<NoteListItem, "recording_id" | "title" | "started_at_ms" | "group"> &
  Partial<NoteListItem>;

function note(draft: Draft): NoteListItem {
  return { duration_ms: 45 * MINUTE, state: "ready", ...draft };
}

const ready = (preview: string) => ({ phase: "ready", preview }) as const;

export const REVIEW_ID = "rec-review-14";
export const DAILY_ID = "rec-daily-21";

export const noteList: NoteListItem[] = [
  note({
    recording_id: DAILY_ID,
    title: "Daily del equipo de producto",
    started_at_ms: at(0, 9, 0),
    duration_ms: 14 * MINUTE,
    group: "today",
    headline: ready("Javier termina hoy la caché de disponibilidad; Marta comparte el prototipo."),
  }),
  note({
    recording_id: "rec-api-20",
    title: "Llamada con Javier sobre la API de tarifas",
    started_at_ms: at(-1, 17, 10),
    duration_ms: 32 * MINUTE,
    group: "yesterday",
    state: "transcribing",
    headline: { phase: "transcribing" },
  }),
  note({
    recording_id: "rec-roadmap-20",
    title: "Planificación del trimestre",
    started_at_ms: at(-1, 10, 0),
    duration_ms: HOUR + 52 * MINUTE,
    group: "yesterday",
    headline: ready("Tres apuestas para el trimestre: pagos, multiidioma y el panel de hoteles."),
  }),
  note({
    recording_id: "rec-one-19",
    title: "1:1 Lucía y Diego",
    started_at_ms: at(-2, 12, 0),
    duration_ms: 28 * MINUTE,
    group: "this_week",
    headline: ready("Diego quiere reforzar el equipo de backend antes de enero."),
  }),
  note({
    recording_id: REVIEW_ID,
    title: "Revisión del diseño de reservas",
    started_at_ms: at(-7, 11, 0),
    duration_ms: 47 * MINUTE,
    group: "last_30_days",
    headline: ready("Pago en una sola pantalla y calendario semanal para el piloto."),
  }),
  note({
    recording_id: "rec-ribera-09",
    title: "Hoteles Ribera · arranque del piloto",
    started_at_ms: at(-12, 16, 0),
    duration_ms: 58 * MINUTE,
    group: "last_30_days",
    headline: ready("Cinco hoteles entran en el piloto; Sara será la persona de contacto."),
  }),
  note({
    recording_id: "rec-users-06",
    title: "Entrevistas con recepcionistas",
    started_at_ms: at(-15, 10, 30),
    duration_ms: 40 * MINUTE,
    group: "last_30_days",
    state: "failed",
    headline: { phase: "failed" },
  }),
  note({
    recording_id: "rec-offsite-sep",
    title: "Offsite de septiembre",
    started_at_ms: at(-34, 9, 30),
    duration_ms: 3 * HOUR,
    group: "before",
    headline: ready("Valores del equipo, ritmo de entregas y cómo medimos la satisfacción."),
  }),
];
