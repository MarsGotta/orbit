import type { AppCommand } from "../../bridge/generated";
import type { MeetingSnapshot } from "../../bridge/generated/MeetingSnapshot";
import { recordingMeeting } from "../data/recording";
import { at, MINUTE, NOW } from "../data/time";
import type { Scenario, Step, Views } from "../scenario";
import { withMeeting } from "./recording";
import { windowNoticeScenarios } from "./window-notices";

// The window: its sidebar with the cards «Grabando» and «En llamada», the
// «Preguntar a Deimos» menu of the card, and the top bar. Its notices and the
// states before Orbit answers are in `window-notices.ts`.

const card = "role=region[name='Grabación en curso']";
const ask = "role=dialog[name='Pregunta sobre esta reunión']";
const QUESTION = "¿Qué se decidió sobre el paso de pago?";

/** The design review, 18 minutes in, with the system's audio until noon. */
const reviewing: MeetingSnapshot = {
  ...recordingMeeting,
  audio: "system",
  ends_at_ms: at(0, 12, 0),
};

const recordingViews = (meeting: MeetingSnapshot) => (base: Views) => withMeeting(base, meeting);

const scene = (scenario: Omit<Scenario, "view" | "route"> & { route?: string }): Scenario => ({
  view: "Ventana",
  route: "#/briefing",
  ...scenario,
});

const asking: Step[] = [{ click: "role=button[name='Preguntar a Deimos']" }, { wait: ask }];
const asked: Step[] = [
  ...asking,
  { fill: "role=textbox[name='Tu pregunta']", text: QUESTION },
  { click: `${ask} >> role=button[name='Preguntar']` },
];

/** Deimos answers the typed question, as the daemon publishes it. */
function answer(views: Views, command: AppCommand): Partial<Views> | null {
  if (command.command !== "meeting_ask") return null;
  const meeting = views.recording.meeting;
  const question = {
    id: "turn-ask-1",
    question: command.question,
    minutes: command.minutes,
    origin: "typed" as const,
    stage: "answered" as const,
    answer:
      "Marta propone dejar el pago en una sola pantalla: en las pruebas con recepcionistas fue casi el doble de rápido. Javier medirá cuánto tarda en cargar.",
    shown_until_ms: NOW + 45_000,
    sent: true,
  };
  return { recording: { ...views.recording, meeting: { ...meeting, question } } };
}

const call = {
  started_at_ms: NOW - 252_000,
  seeing_screen: true,
  last_line: null,
  speaking: false,
  expires_at_ms: NOW + 55 * MINUTE,
  origin: "hotkey" as const,
  idle_hangup_ms: 60_000,
};

export const windowScenarios: Scenario[] = [
  scene({
    id: "ventana",
    state: "Sin nada en marcha",
    description:
      "La barra lateral con nombres y «Grabar reunión», y la barra superior con el buscador y el tema.",
    ready: "role=navigation[name='Principal']",
  }),
  scene({
    id: "ventana-grabando",
    state: "Grabando",
    description:
      "La tarjeta «Grabando» en lugar de «Grabar reunión»: cronómetro, qué graba y hasta cuándo, «Oye Deimos» y los controles siempre a la vista.",
    views: recordingViews(reviewing),
    ready: card,
  }),
  scene({
    id: "ventana-grabando-pausa",
    state: "En pausa",
    description: "La grabación en pausa: el punto deja de latir, el cronómetro se para y «Seguir».",
    views: recordingViews({
      ...reviewing,
      phase: "paused",
      voice_trigger: { state: "not_listening", why: { cause: "paused" } },
    }),
    ready: card,
  }),
  scene({
    id: "ventana-grabando-sin-micro",
    state: "Micrófono en silencio",
    description: "Una pista llega en silencio: la tarjeta lo dice y lleva a Privacidad y permisos.",
    views: recordingViews({
      ...reviewing,
      tracks: reviewing.tracks.map((track) =>
        track.name === "mic_local" ? { ...track, level: "silent" as const } : track,
      ),
    }),
    ready: "text=Micrófono en silencio",
  }),
  scene({
    id: "ventana-preguntar",
    state: "Preguntar a Deimos",
    description:
      "La ventana anclada a la tarjeta: la pregunta, cuántos minutos escucha y «Preguntar».",
    views: recordingViews(reviewing),
    ready: card,
    steps: asking,
  }),
  scene({
    id: "ventana-preguntar-pensando",
    state: "Deimos está pensando",
    description: "Al pulsar «Preguntar», al momento: la pregunta y «Deimos está pensando…».",
    views: recordingViews(reviewing),
    ready: card,
    steps: [...asked, { wait: "text=Deimos está pensando…" }],
  }),
  scene({
    id: "ventana-preguntar-respuesta",
    state: "La respuesta",
    description: "La respuesta de Deimos, que queda también en la conversación, y «Otra pregunta».",
    views: recordingViews(reviewing),
    react: answer,
    ready: card,
    steps: [...asked, { wait: "role=button[name='Otra pregunta']" }],
  }),
  scene({
    id: "ventana-llamada",
    route: "#/calendar",
    state: "En llamada",
    description:
      "Una llamada con ⌃⌥D se ve en cualquier vista: «En llamada», cuánto dura, si ve la pantalla, cuánto queda y «Colgar».",
    views: (base) => ({ ...base, recording: { ...base.recording, call } }),
    ready: "role=region[name='Llamada en curso']",
  }),
  scene({
    id: "ventana-ajustes-seccion",
    route: "#/settings/privacidad",
    state: "Ajustes por sección",
    description:
      "Una sección de Ajustes abierta por su ruta: las migas «Ajustes / Privacidad y permisos» en lugar del buscador.",
    ready: "text=Privacidad y permisos",
  }),
  ...windowNoticeScenarios,
];
