import type { BriefingPreviousActions } from "../../bridge/generated/BriefingPreviousActions";
import type { BriefingSnapshot } from "../../bridge/generated/BriefingSnapshot";
import { recordingMeeting } from "../data/recording";
import type { Scenario, Views } from "../scenario";
import { withMeeting } from "./recording";

const hoy = (scenario: Omit<Scenario, "view" | "route">): Scenario => ({
  view: "Hoy",
  route: "#/briefing",
  ...scenario,
});

const withBriefing =
  (change: (briefing: BriefingSnapshot) => Partial<BriefingSnapshot>) =>
  (base: Views): Views => ({
    ...base,
    briefing: base.briefing && { ...base.briefing, ...change(base.briefing) },
  });

/** The next meeting with «Pendiente de la última vez» in another state. */
const withPending = (actions: BriefingPreviousActions) =>
  withBriefing((briefing) => {
    const next = briefing.next && { ...briefing.next, previous_actions: actions };
    return {
      next,
      upcoming: briefing.upcoming?.map((item, index) => (index === 0 && next) || item),
    };
  });

/** A day the agenda cannot be read: no meetings to show, and why. */
const withoutCalendar = (authorization: "not_determined" | "denied") => (base: Views) => ({
  ...withBriefing(() => ({
    next: null,
    upcoming: [],
    agenda: [],
    summary: { meetings_today: 0, focus_minutes: 0 },
    sources: {
      agenda: { state: "absent", cause: "calendar_permission_denied" },
      notes: { state: "present" },
      recording: { state: "present" },
    },
  }))(base),
  agenda: {
    ...base.agenda,
    authorization,
    events: [],
    reading: { state: "unavailable" as const, blocker: "calendar_permission" as const },
  },
});

const noMeetings = withBriefing(() => ({
  summary: { meetings_today: 0, focus_minutes: 600 },
  next: null,
  upcoming: [],
  agenda: [],
}));

export const hoyScenarios: Scenario[] = [
  hoy({
    id: "hoy",
    state: "Un día normal",
    description:
      "La fecha y la semana, el saludo, la siguiente reunión con lo pendiente de la última vez y el recordatorio personal, y la agenda del día con la marca de ahora.",
    ready: "text=Pendiente de la última vez",
  }),
  hoy({
    id: "hoy-grabando",
    state: "Grabando",
    description:
      "Con la reunión grabándose: el tiempo, «Apuntar en mis notas privadas», «Terminar» y los atajos para preguntar a Deimos.",
    views: (base) => withMeeting(base, recordingMeeting),
    ready: "text=Durante la reunión",
  }),
  hoy({
    id: "hoy-sin-reuniones",
    state: "Sin reuniones",
    description: "Un día sin reuniones en la agenda: la frase del día lo dice y ofrece grabar.",
    views: noMeetings,
    ready: "text=Nada en tu agenda hoy",
  }),
  hoy({
    id: "hoy-cargando",
    state: "Leyendo la agenda",
    description:
      "Antes de que llegue el resumen del día: la forma de lo que viene, nunca en blanco.",
    views: (base) => ({ ...base, briefing: null }),
    ready: "text=Leyendo tu agenda…",
  }),
  hoy({
    id: "hoy-sin-permiso",
    state: "Sin permiso de calendario",
    description: "macOS aún no ha dado el permiso: «Permitir acceso al calendario» se lo pide.",
    views: withoutCalendar("not_determined"),
    ready: "text=Conecta tu calendario",
  }),
  hoy({
    id: "hoy-permiso-denegado",
    state: "Permiso de calendario denegado",
    description: "El permiso se denegó antes: el botón lleva a Ajustes del Sistema.",
    views: withoutCalendar("denied"),
    ready: "text=Abrir Ajustes del Sistema",
  }),
  hoy({
    id: "hoy-pendiente-leyendo",
    state: "Pendiente · leyendo",
    description: "Lo pendiente de la última vez aún se está leyendo del hub.",
    views: withPending({ state: "unknown", cause: "series_not_read" }),
    ready: "text=Leyendo lo pendiente de la última vez…",
  }),
  hoy({
    id: "hoy-pendiente-fallo",
    state: "Pendiente · no se pudo leer",
    description: "La lectura de la serie falló: lo dice, con el motivo.",
    views: withPending({ state: "unknown", cause: "El hub no respondió a tiempo (30 s)" }),
    ready: "text=No se pudo leer lo pendiente de la última vez.",
  }),
  hoy({
    id: "hoy-pendiente-primera",
    state: "Pendiente · primera de la serie",
    description: "No hubo reunión anterior en la serie: no hay nada pendiente.",
    views: withPending({ state: "no_previous_meeting" }),
    ready: "text=Es la primera de esta serie: no hay nada pendiente.",
  }),
  hoy({
    id: "hoy-pendiente-sin-nota",
    state: "Pendiente · nota escribiéndose",
    description:
      "La nota de la última vez aún se está escribiendo: sus tareas no se saben todavía.",
    views: withPending({
      state: "note_not_written",
      recording_id: "rec-review-14",
      note_state: "generating",
    }),
    ready: "text=La nota de la última vez aún se está escribiendo.",
  }),
  hoy({
    id: "hoy-pendiente-guardando",
    state: "Pendiente · marcando una tarea",
    description:
      "Una tarea recién marcada: tachada, con «Guardando…» hasta que el hub lo confirma.",
    ready: "text=Pendiente de la última vez",
    steps: [{ click: "role=checkbox[name=/Marta/]" }, { wait: "text=Guardando…" }],
  }),
];
