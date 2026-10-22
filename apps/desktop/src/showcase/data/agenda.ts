import type { CalendarEventSnapshot } from "../../bridge/generated/CalendarEventSnapshot";
import type { CalendarItemSnapshot } from "../../bridge/generated/CalendarItemSnapshot";
import type { Views } from "../scenario";
import { guests, people } from "./people";
import { at, ZONE } from "./time";

export const calendars: CalendarItemSnapshot[] = [
  {
    id: "work",
    title: "Nubaria",
    source_id: "google",
    source_title: "lucia@nubaria.example",
    color: "accent",
    can_edit: true,
    is_default: true,
  },
  {
    id: "client",
    title: "Hoteles Ribera",
    source_id: "google",
    source_title: "lucia@nubaria.example",
    color: "blue",
    can_edit: false,
  },
  {
    id: "home",
    title: "Personal",
    source_id: "icloud",
    source_title: "iCloud",
    color: "green",
    can_edit: true,
  },
  {
    id: "holidays",
    title: "Festivos en España",
    source_id: "subscribed",
    source_title: "Suscritos",
    color: "gray",
    can_edit: false,
  },
];

type Draft = Pick<CalendarEventSnapshot, "id" | "title" | "start_ms" | "end_ms"> &
  Partial<CalendarEventSnapshot>;

function event(draft: Draft): CalendarEventSnapshot {
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

const daily = (day: number) =>
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

export const designReview = event({
  id: "design-review",
  title: "Revisión del diseño de reservas",
  start_ms: at(0, 11, 0),
  end_ms: at(0, 11, 45),
  attendees: guests("marta", "lucia", "diego", "javier"),
  organizer: people.marta,
  conference_url: "https://meet.example/revision-reservas",
  notes: "Repasamos el flujo nuevo de reserva en tres pasos y los estados vacíos del calendario.",
});

export const riberaCall = event({
  id: "ribera-pilot",
  calendar_id: "client",
  calendar_item_id: "item-ribera",
  title: "Hoteles Ribera · seguimiento del piloto",
  start_ms: at(0, 16, 0),
  end_ms: at(0, 17, 0),
  attendees: guests("pablo", "lucia", "irene", "sara"),
  organizer: people.pablo,
  conference_url: "https://meet.example/ribera-piloto",
  can_edit: false,
});

export const events: CalendarEventSnapshot[] = [
  ...[-2, -1, 0, 1, 2].map(daily),
  event({
    id: "one-to-one",
    title: "1:1 Lucía y Diego",
    start_ms: at(-2, 12, 0),
    end_ms: at(-2, 12, 30),
    attendees: guests("diego", "lucia"),
  }),
  event({
    id: "roadmap",
    title: "Planificación del trimestre",
    start_ms: at(-1, 10, 0),
    end_ms: at(-1, 12, 0),
    attendees: guests("lucia", "diego", "irene"),
    location: "Sala Tramontana",
  }),
  designReview,
  event({
    id: "lunch",
    calendar_id: "home",
    title: "Comida con Irene",
    start_ms: at(0, 13, 30),
    end_ms: at(0, 14, 30),
    location: "Mercado de San Ildefonso",
    availability: "free",
  }),
  riberaCall,
  event({
    id: "yoga",
    calendar_id: "home",
    title: "Yoga",
    start_ms: at(0, 19, 0),
    end_ms: at(0, 20, 0),
  }),
  event({
    id: "hiring",
    title: "Entrevista: diseñadora de producto",
    start_ms: at(1, 12, 0),
    end_ms: at(1, 13, 0),
    attendees: guests("lucia", "marta"),
    self_response: "needs_action",
  }),
  event({
    id: "retro",
    title: "Retrospectiva del sprint 42",
    start_ms: at(2, 15, 0),
    end_ms: at(2, 16, 0),
    attendees: guests("diego", "lucia", "marta", "javier"),
    self_response: "tentative",
  }),
  event({
    id: "dentist",
    calendar_id: "home",
    title: "Dentista",
    start_ms: at(2, 18, 30),
    end_ms: at(2, 19, 15),
    location: "Clínica Arganzuela",
  }),
  event({
    id: "trip",
    calendar_id: "home",
    title: "Escapada a Cuenca",
    start_ms: at(3, 0, 0),
    end_ms: at(5, 0, 0),
    all_day: true,
  }),
];

const weekStart = at(-2, 0, 0);
const weekEnd = at(5, 0, 0);

export const agenda: Views["agenda"] = {
  authorization: "granted",
  calendars,
  window: { start_ms: weekStart, end_ms: weekEnd, timezone: ZONE },
  range: {
    view: "week",
    days: 7,
    week_start: "monday",
    anchor_ms: at(0, 10, 40),
    start_ms: weekStart,
    end_ms: weekEnd,
  },
  events,
  reading: { state: "available" },
  stale: false,
  writes: [],
};
