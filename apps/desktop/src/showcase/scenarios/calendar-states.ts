import type { Scenario, Views } from "../scenario";

export const calendar = (scenario: Omit<Scenario, "view" | "route">): Scenario => ({
  view: "Calendario",
  route: "#/calendar",
  ...scenario,
});

export const withAgenda = (change: Partial<Views["agenda"]>) => (base: Views) => ({
  ...base,
  agenda: { ...base.agenda, ...change },
});

/** The calendar before it can show a period: loading, and without the permission. */
export const calendarStates: Scenario[] = [
  calendar({
    id: "calendario-actualizando",
    state: "Leyendo el periodo",
    description: "Un periodo que aún se lee: «Actualizando calendario…» y la forma de los eventos.",
    views: withAgenda({ stale: true, events: [] }),
    ready: "text=Actualizando calendario…",
  }),
  calendar({
    id: "calendario-cargando",
    state: "Esperando al Mac",
    description: "Antes de saber el permiso: el calendario cargando, nunca en blanco.",
    views: withAgenda({ authorization: "unknown", events: [], reading: { state: "unknown" } }),
    ready: "text=Leyendo tu calendario…",
  }),
  calendar({
    id: "calendario-sin-permiso",
    state: "Sin permiso todavía",
    description:
      "Antes de que macOS dé acceso: «Conecta tu calendario» y «Permitir acceso al calendario».",
    views: withAgenda({ authorization: "not_determined", events: [], calendars: [] }),
    ready: "text=Conecta tu calendario",
  }),
  calendar({
    id: "calendario-sin-permiso-esperando",
    state: "Esperando a macOS",
    description: "Tras pulsar «Permitir acceso al calendario», hasta que macOS contesta.",
    views: withAgenda({ authorization: "not_determined", events: [], calendars: [] }),
    ready: "text=Conecta tu calendario",
    steps: [{ click: "role=button[name='Permitir acceso al calendario']" }],
  }),
  calendar({
    id: "calendario-denegado",
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
