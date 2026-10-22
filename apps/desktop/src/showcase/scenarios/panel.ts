import type { MeetingSnapshot } from "../../bridge/generated/MeetingSnapshot";
import { approval } from "../data/chat-turns";
import { recordingMeeting } from "../data/recording";
import { NOW } from "../data/time";
import type { Scenario } from "../scenario";
import { withMeeting } from "./recording";

const CALL = { application: "Google Meet", event_title: "Hoteles Ribera · seguimiento del piloto" };
/** The window is the 340 point card, as tall as what it shows (`src-tauri/src/panel.rs`). */
const OFFER = { width: 340, height: 120 };
const ENDING = { width: 340, height: 132 };
const RECORDING = { width: 340, height: 164 };
const ASK = { width: 340, height: 200 };
const ANSWER = { width: 340, height: 300 };
const BY_CALL = { state: "recording", recording_id: "rec-review-21", ...CALL } as const;
const question = {
  id: "q-1",
  question: "¿Qué dijo Pablo de Ribera Aeropuerto?",
  minutes: 5,
  origin: "typed",
  sent: true,
} as const;

const panel = (
  scenario: Omit<Scenario, "view" | "route" | "panel" | "views"> & {
    meeting: Partial<MeetingSnapshot>;
    /** Deimos waits for an approval while it answers. */
    asks?: boolean;
  },
): Scenario => {
  const { meeting, asks, ...rest } = scenario;
  return {
    view: "Panel flotante",
    route: "",
    panel: true,
    views: (base) => {
      const views = withMeeting(base, { ...base.recording.meeting, ...meeting });
      return asks === true
        ? { ...views, assistant: { ...views.assistant, pending: approval } }
        : views;
    },
    ...rest,
  };
};

const answer = `Lo último fue sobre **Ribera Aeropuerto**: Pablo pidió esperar a la caché de tarifas antes de incluirlo. Javier dijo que la tendrá el jueves.

Más detalle en [la nota del arranque](https://hub.example/notas/rec-ribera-09).`;

export const panelScenarios: Scenario[] = [
  panel({
    id: "panel-llamada",
    state: "Empieza una videollamada",
    description: "El panel que aparece al empezar una videollamada: grabarla o dejarla.",
    size: OFFER,
    meeting: { call_panel: { state: "offer", call_id: "call-7", ...CALL } },
    ready: "role=button[name='Grabar']",
  }),
  panel({
    id: "panel-grabando",
    state: "Grabando la llamada",
    description:
      "Mientras graba la llamada: el tiempo, preguntar a Deimos, parar y cómo preguntar con la voz.",
    size: RECORDING,
    meeting: { ...recordingMeeting, call_panel: BY_CALL },
    ready: "role=button[name='Parar']",
  }),
  panel({
    id: "panel-grabando-a-mano",
    state: "Grabando sin llamada",
    description:
      "Una grabación empezada a mano también tiene el panel, para preguntar a Deimos sin volver a Orbit.",
    size: RECORDING,
    meeting: { ...recordingMeeting, call_panel: null },
    ready: "role=button[name='Parar']",
  }),
  panel({
    id: "panel-terminando",
    state: "Se cuelga la llamada",
    description: "La llamada terminó: cuenta los segundos que faltan para dejar de grabar.",
    size: ENDING,
    meeting: {
      ...recordingMeeting,
      call_panel: {
        state: "ending",
        recording_id: "rec-review-21",
        ends_at_ms: NOW + 13_000,
        application: "Google Meet",
      },
    },
    ready: "role=button[name='Seguir grabando']",
  }),
  panel({
    id: "panel-preguntar",
    state: "Preguntar sin hablar",
    description:
      "La pregunta a Deimos escrita en el panel, con los minutos de la llamada que escucha.",
    size: ASK,
    meeting: { ...recordingMeeting, call_panel: BY_CALL },
    ready: "role=button[name=/Preguntar a/]",
    steps: [
      { click: "role=button[name=/Preguntar a/]" },
      { fill: "role=textbox", text: "¿Qué dijo Pablo de Ribera Aeropuerto?" },
    ],
  }),
  panel({
    id: "panel-pensando",
    state: "Deimos piensa",
    description: "La pregunta, mientras Deimos piensa la respuesta.",
    size: ANSWER,
    meeting: { ...recordingMeeting, question: { ...question, stage: "thinking" } },
    ready: "text=está pensando",
  }),
  panel({
    id: "panel-permiso",
    state: "Deimos pide permiso",
    description: "Mientras responde, Deimos pide permiso para algo; se responde en el panel.",
    size: ANSWER,
    asks: true,
    meeting: {
      ...recordingMeeting,
      question: {
        ...question,
        question: "Mándale a Pablo la fecha del arranque",
        origin: "voice",
        stage: "thinking",
      },
    },
    ready: "role=button[name='Permitir una vez']",
  }),
  panel({
    id: "panel-respuesta",
    state: "La respuesta",
    description:
      "La respuesta, con su enlace, «Otra pregunta» y la barra de los 45 s que tarda en irse.",
    size: ANSWER,
    meeting: {
      ...recordingMeeting,
      question: { ...question, stage: "answered", answer, shown_until_ms: NOW + 45_000 },
    },
    ready: "text=Ribera Aeropuerto",
  }),
  panel({
    id: "panel-sin-respuesta",
    state: "Sin respuesta",
    description: "Una pregunta que no tuvo respuesta dice por qué.",
    size: ANSWER,
    meeting: {
      ...recordingMeeting,
      question: {
        ...question,
        stage: "failed",
        cause: { cause: "hub_unreachable", connection: "offline" },
      },
    },
    ready: "text=no respondió",
  }),
];
