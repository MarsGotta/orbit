import type { MeetingSnapshot } from "../../bridge/generated/MeetingSnapshot";
import { idleMeeting, recordingMeeting } from "../data/recording";
import { at, NOW } from "../data/time";
import type { Scenario, Views } from "../scenario";

/** `views` with the meeting changed, and the briefing saying it records. */
export function withMeeting(views: Views, meeting: MeetingSnapshot): Views {
  const recording = meeting.phase !== "idle";
  const next = views.briefing?.next;
  return {
    ...views,
    recording: { ...views.recording, meeting },
    briefing: views.briefing && {
      ...views.briefing,
      next: next ? { ...next, recording: recording ? "recording" : "none" } : null,
      upcoming: views.briefing.upcoming?.map((item, index) =>
        index === 0 ? { ...item, recording: recording ? "recording" : "none" } : item,
      ),
    },
  };
}

const record = (scenario: Omit<Scenario, "view" | "route"> & { route?: string }): Scenario => ({
  view: "Grabación",
  route: "#/briefing",
  ...scenario,
});

const pill = "role=status[name=/Grabando ·/]";

export const recordingScenarios: Scenario[] = [
  record({
    id: "grabar-lanzador",
    state: "Empezar a grabar",
    description: "«Meet» abre el lanzador: qué reunión, qué audio y cuánto dura.",
    ready: "text=A tratar",
    steps: [{ click: "role=button[name='Meet']" }, { wait: "role=dialog" }],
  }),
  record({
    id: "grabando",
    state: "Grabación en curso",
    description:
      "Una reunión grabándose: la píldora de la barra con el tiempo y la tarjeta de Hoy que lo dice.",
    views: (base) => withMeeting(base, recordingMeeting),
    ready: pill,
  }),
  record({
    id: "grabando-controles",
    state: "Controles de la grabación",
    description: "Al pasar por la píldora: preguntar a Deimos, pausar y terminar.",
    views: (base) => withMeeting(base, recordingMeeting),
    ready: pill,
    steps: [{ hover: pill }, { wait: "role=button[name='Terminar']" }],
  }),
  record({
    id: "grabando-preguntar",
    state: "Preguntar durante la reunión",
    description:
      "La pregunta a Deimos sobre lo último que se dijo, con la ventana de minutos que mira.",
    views: (base) => withMeeting(base, recordingMeeting),
    ready: pill,
    steps: [
      { hover: pill },
      { click: "role=button[name=/Preguntar a/]" },
      { wait: "role=textbox" },
    ],
  }),
  record({
    id: "grabando-pausa",
    state: "Grabación en pausa",
    description: "La grabación en pausa, con el botón para seguir.",
    views: (base) => withMeeting(base, { ...recordingMeeting, phase: "paused" }),
    ready: "role=status[name=/En pausa|Pausada/]",
  }),
  record({
    id: "grabando-sin-micro",
    state: "El micrófono no se oye",
    description: "La píldora avisa cuando una pista llega en silencio y lleva a Ajustes.",
    views: (base) =>
      withMeeting(base, {
        ...recordingMeeting,
        tracks: recordingMeeting.tracks.map((track) =>
          track.name === "mic_local" ? { ...track, level: "silent" as const } : track,
        ),
      }),
    ready: "role=status[name=/Micrófono en silencio/]",
    steps: [{ hover: "role=status[name=/Micrófono en silencio/]" }],
  }),
  record({
    id: "reunion-guardada",
    state: "Reunión guardada",
    description: "Al terminar, el aviso de que la reunión se guardó, con el enlace a su nota.",
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
  }),
];
