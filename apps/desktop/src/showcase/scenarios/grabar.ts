import type { MeetingSnapshot } from "../../bridge/generated/MeetingSnapshot";
import { idleMeeting, recordingMeeting } from "../data/recording";
import { at, NOW } from "../data/time";
import type { Scenario, Views } from "../scenario";
import { privacyStopped } from "./edit";
import { withMeeting } from "./recording";

/** «Grabar reunión», wherever the window puts it (the sidebar, or the bar before it). */
const OPEN = "role=button[name=/^Grabar reunión$/]";
const DIALOG = "role=dialog[name='Nueva grabación']";
const APPLICATION = "role=radio[name=/Una app concreta/]";

const dialog = (
  scenario: Omit<Scenario, "view" | "route" | "ready"> & { choose?: string },
): Scenario => {
  const { choose, steps, ...rest } = scenario;
  return {
    view: "Nueva grabación",
    route: "#/briefing",
    ready: OPEN,
    steps: [
      { click: OPEN },
      { wait: DIALOG },
      ...(choose === undefined ? [] : [{ click: choose }]),
      ...(steps ?? []),
    ],
    ...rest,
  };
};

const withApplications = (views: Views, change: Partial<MeetingSnapshot>): Views =>
  withMeeting(views, { ...views.recording.meeting, ...change });

export const grabarScenarios: Scenario[] = [
  dialog({
    id: "grabar-nueva",
    state: "Empezar a grabar",
    description:
      "«Grabar reunión» abre la ventana: el título que sugiere la reunión que empieza, qué audio grabar y hasta cuándo graba.",
  }),
  dialog({
    id: "grabar-una-app",
    state: "Una app concreta",
    description: "Al elegir «Una app concreta», las apps que suenan ahora, como chips.",
    choose: APPLICATION,
  }),
  dialog({
    id: "grabar-buscando-apps",
    state: "Buscando apps con sonido",
    description:
      "Mientras el Mac dice qué apps suenan, «Una app concreta» lo dice en vez de quedarse muda.",
    views: (base) =>
      withApplications(base, { applications: [], applications_state: "unavailable" }),
    choose: APPLICATION,
  }),
  dialog({
    id: "grabar-sin-apps",
    state: "Ninguna app suena",
    description: "Ninguna app suena ahora: lo dice y deja volver a buscar.",
    views: (base) => withApplications(base, { applications: [], applications_state: "empty" }),
    choose: APPLICATION,
  }),
  dialog({
    id: "grabar-sin-permiso",
    state: "Falta un permiso",
    description:
      "Sin permiso de micrófono: una sola fila dentro de la ventana, con lo que lo arregla, y «Empezar a grabar» esperando.",
    views: (base) => ({ ...base, permissions: { ...base.permissions, microphone: "denied" } }),
  }),
  dialog({
    id: "grabar-todo-parado",
    state: "Todo está parado",
    description: "Con todo parado, la ventana lo dice y reactiva Orbit desde ella.",
    views: privacyStopped,
  }),
  {
    id: "reunion-guardada",
    view: "Nueva grabación",
    state: "Reunión guardada",
    description:
      "Al terminar, el aviso de que la reunión se guardó; «Tu nota está lista» llegará cuando lo esté.",
    route: "#/briefing",
    views: (base) => withMeeting(base, { ...recordingMeeting, phase: "finishing" }),
    later: (views) => {
      const saved = withMeeting(views, {
        ...idleMeeting,
        recent: [
          {
            recording_id: recordingMeeting.recording_id ?? "",
            title: recordingMeeting.title ?? "",
            started_at_ms: at(0, 10, 22),
            duration_ms: NOW - at(0, 10, 22),
            calendar_event_id: "design-review",
            transcription: { pending: 2, running: 1, completed: 15, failed: 0 },
            summary: "pending",
          },
          ...idleMeeting.recent,
        ],
      });
      return { recording: saved.recording, briefing: saved.briefing };
    },
    ready: "text=Reunión guardada",
  },
];
