import type { CalendarEventSnapshot } from "../../bridge/generated/CalendarEventSnapshot";
import { guests, people } from "./people";
import { at } from "./time";

type Draft = Pick<CalendarEventSnapshot, "id" | "title" | "start_ms" | "end_ms"> &
  Partial<CalendarEventSnapshot>;

export function event(draft: Draft): CalendarEventSnapshot {
  return {
    calendar_id: "work",
    calendar_item_id: `item-${draft.id}`,
    occurrence_start_ms: draft.start_ms,
    status: "confirmed",
    self_response: "accepted",
    attendees: [],
    can_edit: true,
    availability: "busy",
    ...draft,
  };
}

export const daily = (day: number) =>
  event({
    id: `daily-${day}`,
    title: "Daily del equipo de producto",
    start_ms: at(day, 9, 0),
    end_ms: at(day, 9, 15),
    attendees: guests("diego", "lucia", "marta", "javier"),
    organizer: people.diego,
    conference_url: "https://meet.example/nubaria-daily",
    recurrence: { frequency: "weekly", interval: 1, days_of_week: [{ day: "monday" }] },
    calendar_item_id: "item-daily",
  });

export const weeklyReview = {
  frequency: "weekly" as const,
  interval: 1,
  days_of_week: [{ day: "wednesday" as const }],
};

const review = (id: string, day: number) =>
  event({
    id,
    title: "Revisión del diseño de reservas",
    start_ms: at(day, 11, 0),
    end_ms: at(day, 11, 45),
    calendar_item_id: "item-design-review",
    attendees: guests("lucia", "marta", "diego", "javier"),
    organizer: { ...people.lucia, is_self: true },
    conference_url: "https://meet.example/revision-reservas",
    recurrence: weeklyReview,
    self_response: null,
  });

/** The rest of the month, for the month and the agenda. */
export const monthEvents: CalendarEventSnapshot[] = [
  event({
    id: "receptionists",
    title: "Entrevistas con recepcionistas",
    start_ms: at(-15, 10, 0),
    end_ms: at(-15, 11, 0),
  }),
  review("review-07", -14),
  event({
    id: "pilot-kickoff",
    calendar_id: "client",
    title: "Hoteles Ribera · arranque del piloto",
    start_ms: at(-12, 12, 0),
    end_ms: at(-12, 13, 0),
    can_edit: false,
  }),
  event({
    id: "national-day",
    calendar_id: "holidays",
    title: "Día de la Hispanidad",
    start_ms: at(-9, 0, 0),
    end_ms: at(-8, 0, 0),
    all_day: true,
    can_edit: false,
  }),
  review("review-14", -7),
  event({
    id: "pilot-start",
    calendar_id: "client",
    title: "Arranque del piloto",
    start_ms: at(5, 0, 0),
    end_ms: at(6, 0, 0),
    all_day: true,
    can_edit: false,
  }),
  ...[5, 6, 7, 8, 9].map(daily),
  event({
    id: "training",
    calendar_id: "client",
    title: "Formación Ribera Playa",
    start_ms: at(6, 10, 0),
    end_ms: at(6, 11, 30),
    attendees: guests("pablo", "lucia", "irene", "sara", "diego"),
    conference_url: "https://meet.example/formacion-ribera",
    can_edit: false,
  }),
  review("review-28", 7),
];
