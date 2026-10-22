import { at } from "../data/time";
import type { Scenario, Views } from "../scenario";
import { withMeeting } from "./recording";
import { recordingMeeting } from "../data/recording";

const agenda = (scenario: Omit<Scenario, "view" | "route">): Scenario => ({
  view: "Agenda",
  route: "#/calendar",
  ...scenario,
});

const withAgenda = (change: Partial<Views["agenda"]>) => (base: Views) => ({
  ...base,
  agenda: { ...base.agenda, ...change },
});

const monthStart = at(-23, 0, 0);
const monthEnd = at(12, 0, 0);

export const agendaScenarios: Scenario[] = [
  agenda({
    id: "agenda-semana",
    state: "Semana",
    description:
      "La semana con los calendarios de este Mac: reuniones, un evento sin responder y la hora actual.",
    ready: "text=Revisión del diseño de reservas",
  }),
  agenda({
    id: "agenda-mes",
    state: "Mes",
    description: "El mes entero, con los eventos de cada día.",
    storage: {
      "orbit.calendar.view.local": JSON.stringify({ byTier: { wide: { view: "month", days: 7 } } }),
    },
    views: withAgenda({
      range: {
        view: "month",
        days: 35,
        week_start: "monday",
        anchor_ms: at(0, 10, 40),
        start_ms: monthStart,
        end_ms: monthEnd,
      },
      window: { start_ms: monthStart, end_ms: monthEnd, timezone: "Europe/Madrid" },
    }),
    ready: "text=Revisión del diseño de reservas",
  }),
  agenda({
    id: "agenda-vistas",
    state: "Menú de vistas",
    description:
      "Elegir día, varios días, semana, mes o agenda, el zoom y las preferencias de la semana.",
    ready: "text=Revisión del diseño de reservas",
    steps: [{ click: "role=button[name=/Vista del calendario/]" }],
  }),
  agenda({
    id: "agenda-calendarios",
    state: "Calendarios visibles",
    description: "Ocultar o mostrar cada calendario del Mac en esta vista.",
    ready: "text=Revisión del diseño de reservas",
    steps: [
      { click: "role=button[name=/calendarios/]" },
      { wait: "role=group[name='Calendarios visibles']" },
    ],
  }),
  agenda({
    id: "agenda-evento",
    state: "Detalle de un evento",
    description:
      "Una reunión abierta: invitados con su respuesta, videollamada, grabar y la plantilla de su serie.",
    ready: "text=Revisión del diseño de reservas",
    steps: [
      { click: "[data-event-id='design-review']" },
      { wait: "role=complementary[name='Detalles del evento']" },
    ],
  }),
  agenda({
    id: "agenda-evento-solo-lectura",
    state: "Evento de un calendario ajeno",
    description:
      "Un evento de un calendario que no se puede cambiar desde aquí: lo dice y lo muestra sin editar.",
    ready: "text=Hoteles Ribera · seguimiento del piloto",
    steps: [
      { click: "[data-event-id='ribera-pilot']" },
      { wait: "role=complementary[name='Detalles del evento']" },
    ],
  }),
  agenda({
    id: "agenda-nuevo-evento",
    state: "Evento nuevo",
    description:
      "Un clic en un hueco de la semana empieza un evento: título, hora, calendario y avisos.",
    ready: "text=Revisión del diseño de reservas",
    // Thursday 22 October; the n-th line of its column is the hour n.
    steps: [
      { mouse: "[data-day='20748'] > div:nth-child(17)", at: [60, 30] },
      { wait: "role=complementary[name='Detalles del evento']" },
      { fill: "role=textbox[name='Título']", text: "Preparar la demo para Ribera" },
    ],
  }),
  agenda({
    id: "agenda-no-se-mueve",
    state: "Un evento que no se puede mover",
    description:
      "Arrastrar un evento de un calendario de solo lectura: no se mueve y dice por qué.",
    ready: "text=Hoteles Ribera · seguimiento del piloto",
    steps: [{ drag: "[data-event-id='ribera-pilot']", by: [0, 90] }, { wait: "role=alert" }],
  }),
  agenda({
    id: "agenda-grabando",
    state: "Reunión grabándose",
    description: "La semana mientras se graba la revisión de diseño: su tarjeta lo dice.",
    views: (base) => {
      const views = withMeeting(base, recordingMeeting);
      const events = views.agenda.events.map((event) =>
        event.id === "design-review" ? { ...event, recording: "recording" as const } : event,
      );
      return { ...views, agenda: { ...views.agenda, events } };
    },
    ready: "text=Grabando…",
  }),
  agenda({
    id: "agenda-conectar",
    state: "Sin permiso todavía",
    description: "La agenda antes de que macOS dé permiso a los calendarios: qué gana y el botón.",
    views: withAgenda({ authorization: "not_determined", events: [], calendars: [] }),
    ready: "text=Permitir acceso al calendario",
  }),
  agenda({
    id: "agenda-denegada",
    state: "Permiso denegado",
    description: "Sin permiso de calendarios: cómo abrir Ajustes del Sistema para darlo.",
    views: withAgenda({
      authorization: "denied",
      events: [],
      reading: { state: "unavailable", blocker: "calendar_permission" },
    }),
    ready: "role=button[name=/Ajustes del Sistema/]",
  }),
];
