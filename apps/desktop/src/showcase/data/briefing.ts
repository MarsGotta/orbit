import type { BriefingAgendaSnapshot } from "../../bridge/generated/BriefingAgendaSnapshot";
import type { BriefingNextSnapshot } from "../../bridge/generated/BriefingNextSnapshot";
import type { Views } from "../scenario";
import { designReview, events, riberaCall } from "./agenda";
import { OWNER_FIRST } from "./status";
import { at } from "./time";

const colors: Record<string, string> = { work: "accent", client: "blue", home: "green" };

const today = events.filter(
  (event) => event.start_ms >= at(0, 0) && event.start_ms < at(1, 0) && event.all_day !== true,
);

const agendaRows: BriefingAgendaSnapshot[] = today.map((event) => ({
  event_id: event.id,
  start_ms: event.start_ms,
  title: event.title,
  color: colors[event.calendar_id ?? "work"] ?? "accent",
  people_label: "",
  minutes: (event.end_ms - event.start_ms) / 60_000,
  people_count: event.attendees?.length ?? 0,
}));

export const nextReview: BriefingNextSnapshot = {
  event_id: designReview.id,
  event: { calendar_item_id: "item-design-review", occurrence_start_ms: designReview.start_ms },
  title: designReview.title,
  start_ms: designReview.start_ms,
  end_ms: designReview.end_ms,
  attendees: [
    { name: "Marta Ibáñez", initial: "M", color: "orange" },
    { name: "Diego Salinas", initial: "D", color: "blue" },
    { name: "Javier Ortega", initial: "J", color: "green" },
  ],
  url: "https://meet.example/revision-reservas",
  series_key: "item-design-review",
  previous: {
    recording_id: "rec-review-14",
    title: "Revisión del diseño de reservas",
    summary:
      "Se acordó simplificar el paso de pago y probar el calendario semanal con cinco hoteles del piloto.",
    open_items: [
      "Marta: prototipo del paso de pago en una sola pantalla",
      "Javier: medir cuánto tarda la disponibilidad en cargar",
    ],
  },
  recording: "none",
};

const nextRibera: BriefingNextSnapshot = {
  event_id: riberaCall.id,
  title: riberaCall.title,
  start_ms: riberaCall.start_ms,
  end_ms: riberaCall.end_ms,
  attendees: [
    { name: "Pablo Medina", initial: "P", color: "blue" },
    { name: "Irene Castaño", initial: "I", color: "accent" },
    { name: "Sara Llorente", initial: "S", color: "gray" },
  ],
  url: "https://meet.example/ribera-piloto",
  series_key: "item-ribera",
  previous: null,
  recording: "none",
};

export const briefing: Views["briefing"] = {
  greeting: { name: OWNER_FIRST },
  summary: { meetings_today: 4, focus_minutes: 165 },
  next: nextReview,
  upcoming: [nextReview, nextRibera],
  agenda: agendaRows,
  sources: {
    agenda: { state: "present" },
    notes: { state: "present" },
    recording: { state: "present" },
  },
};
