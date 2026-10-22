import type { CalendarViewKind } from "../../bridge/generated/CalendarViewKind";
import { at } from "../data/time";
import type { Scenario } from "../scenario";
import { calendar, calendarStates, withAgenda } from "./calendar-states";
import { withMeeting } from "./recording";
import { recordingMeeting } from "../data/recording";

/** The range app-core publishes for `view`, from `first` days after today, `days` long. */
function shown(view: CalendarViewKind, first: number, days: number) {
  const start = at(first, 0, 0);
  const end = at(first + days, 0, 0);
  return withAgenda({
    range: {
      view,
      days,
      week_start: "monday",
      anchor_ms: at(0, 10, 40),
      start_ms: start,
      end_ms: end,
    },
    window: { start_ms: Math.min(start, at(0, 0, 0)), end_ms: end, timezone: "Europe/Madrid" },
  });
}

/** The view the user chose on this Mac, so the calendar keeps the range shown. */
const chosen = (view: CalendarViewKind, days: number) => ({
  "orbit.calendar.view.local": JSON.stringify({ byTier: { wide: { view, days } } }),
});

const ready = "text=Revisión del diseño de reservas";
const panel = "role=complementary[name='Detalles del evento']";
const review = "[data-event-id='design-review'] button";
// Thursday 22 October; the n-th line of its column is the hour n.
const thursdayAt = (hour: number) => `[data-day='20748'] > div:nth-child(${hour})`;

export const calendarScenarios: Scenario[] = [
  calendar({
    id: "calendario-semana",
    state: "Semana",
    description:
      "La semana: tinte y punto de cada calendario, «Quizá» y «Sin responder» con borde discontinuo, lo pasado atenuado y la línea de ahora.",
    ready,
  }),
  calendar({
    id: "calendario-dia",
    state: "Día",
    description: "Un día: «Miércoles, 21 de octubre», con la hora actual.",
    storage: chosen("day", 1),
    views: shown("day", 0, 1),
    ready,
  }),
  calendar({
    id: "calendario-3-dias",
    state: "3 días",
    description: "Tres días: «21 – 23 de octubre».",
    storage: chosen("days", 3),
    views: shown("days", 0, 3),
    ready,
  }),
  calendar({
    id: "calendario-mes",
    state: "Mes",
    description: "El mes, con hoy elegido y su agenda a la derecha.",
    storage: chosen("month", 7),
    views: shown("month", -23, 35),
    ready,
  }),
  calendar({
    id: "calendario-mes-otro-dia",
    state: "Mes, otro día elegido",
    description: "Pulsar un día lo elige y su agenda sale a la derecha con «Abrir este día».",
    storage: chosen("month", 7),
    views: shown("month", -23, 35),
    ready,
    steps: [{ click: "role=button[name=/^jueves 22 de octubre/]" }],
  }),
  calendar({
    id: "calendario-agenda",
    state: "Agenda",
    description: "Los próximos días en lista, agrupados por día, con «HOY» y lo pasado atenuado.",
    storage: chosen("agenda", 7),
    views: shown("agenda", -2, 10),
    ready,
  }),
  calendar({
    id: "calendario-evento",
    state: "Panel del evento",
    description:
      "Una reunión tuya que se repite: «Unirse y grabar», «Solo grabar», «Avisarme», «Mostrarme como», tu respuesta e invitados. «Sin cambios».",
    ready,
    steps: [{ click: review }, { wait: panel }],
  }),
  calendar({
    id: "calendario-evento-cambios",
    state: "Panel con cambios en una serie",
    description:
      "Al cambiar algo de un evento que se repite: «Solo este» y «Este y los siguientes».",
    ready,
    steps: [
      { click: review },
      { wait: panel },
      { fill: "role=textbox[name='Ubicación']", text: "Sala Atlántico" },
    ],
  }),
  calendar({
    id: "calendario-evento-guardando",
    state: "Guardando",
    description: "Tras «Solo este»: el panel dice «Guardando…» y la tarjeta también.",
    ready,
    steps: [
      { click: review },
      { wait: panel },
      { fill: "role=textbox[name='Ubicación']", text: "Sala Atlántico" },
      { click: "role=button[name='Solo este']" },
    ],
  }),
  calendar({
    id: "calendario-evento-solo-lectura",
    state: "Evento de solo lectura",
    description: "Un evento de un calendario ajeno: el candado, por qué, y ningún campo.",
    ready: "text=Hoteles Ribera · seguimiento del piloto",
    steps: [{ click: "[data-event-id='ribera-pilot'] button" }, { wait: panel }],
  }),
  calendar({
    id: "calendario-evento-nuevo",
    state: "Evento nuevo",
    description:
      "Un clic en un hueco empieza un borrador: título, fecha y hora, calendario y avisos.",
    ready,
    steps: [
      { mouse: thursdayAt(17), at: [60, 30] },
      { wait: panel },
      { fill: "role=textbox[name='Título']", text: "Preparar la demo para Ribera" },
    ],
  }),
  calendar({
    id: "calendario-arrastrando",
    state: "Arrastrando un evento nuevo",
    description: "Arrastrar en un hueco dibuja el borrador con sus horas, antes de soltar.",
    ready,
    steps: [{ hold: thursdayAt(14), at: [60, 4], by: [0, 90] }],
  }),
  calendar({
    id: "calendario-no-se-mueve",
    state: "Un evento que no se puede mover",
    description:
      "Arrastrar un evento de un calendario de solo lectura: no se mueve y dice por qué.",
    ready: "text=Hoteles Ribera · seguimiento del piloto",
    steps: [{ drag: "[data-event-id='ribera-pilot']", by: [0, 90] }, { wait: "role=alert" }],
  }),
  calendar({
    id: "calendario-grabando",
    state: "Reunión grabándose",
    description: "La semana mientras se graba la revisión de diseño: su tarjeta lo dice.",
    views: (base) => {
      const views = withMeeting(base, recordingMeeting);
      const events = views.agenda.events.map((event) =>
        event.id === "design-review" ? { ...event, recording: "recording" as const } : event,
      );
      return { ...views, agenda: { ...views.agenda, events } };
    },
    ready: "[data-recording='true']",
  }),
  calendar({
    id: "calendario-opciones",
    state: "Opciones de vista",
    description: "Fines de semana, rechazados, el día que empieza la semana, el zoom y 14 días.",
    ready,
    steps: [{ click: "role=button[name='Opciones de vista']" }],
  }),
  calendar({
    id: "calendario-calendarios",
    state: "Calendarios visibles",
    description: "Ocultar o mostrar cada calendario del Mac, solo en esta vista.",
    ready,
    steps: [
      { click: "role=button[name=/calendarios/]" },
      { wait: "role=dialog[name='Calendarios visibles']" },
    ],
  }),
  ...calendarStates,
];
