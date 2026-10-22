import type { MeetingSnapshot } from "../../bridge/generated/MeetingSnapshot";
import { approval } from "../data/chat-turns";
import { recordingMeeting } from "../data/recording";
import { NOW } from "../data/time";
import type { Scenario } from "../scenario";
import { withMeeting } from "./recording";

const CALL = { application: "Google Meet", event_title: "Hoteles Ribera · seguimiento del piloto" };
/** The panel's height with a question to Deimos (`PANEL_ANSWER_HEIGHT`). */
const ANSWER = { width: 360, height: 300 };

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
    state: "Llamada detectada",
    description: "El panel que aparece al empezar una videollamada: grabarla o dejarla.",
    meeting: { call_panel: { state: "offer", call_id: "call-7", ...CALL } },
    ready: "role=button[name='Grabar']",
  }),
  panel({
    id: "panel-grabando",
    state: "Grabando la llamada",
    description:
      "El panel mientras graba una llamada: parar, preguntar a Deimos y si escucha «Oye Deimos».",
    meeting: {
      ...recordingMeeting,
      call_panel: { state: "recording", recording_id: "rec-review-21", ...CALL },
    },
    ready: "role=button[name='Parar']",
  }),
  panel({
    id: "panel-terminando",
    state: "La llamada terminó",
    description:
      "La llamada terminó: el panel cuenta los segundos que faltan para cerrar la grabación.",
    meeting: {
      ...recordingMeeting,
      call_panel: {
        state: "ending",
        recording_id: "rec-review-21",
        ends_at_ms: NOW + 25_000,
        application: "Google Meet",
      },
    },
    ready: "role=button[name='Seguir grabando']",
  }),
  panel({
    id: "panel-preguntar",
    state: "Escribiendo una pregunta",
    description: "La pregunta a Deimos desde el panel, con los minutos de reunión que mira.",
    size: ANSWER,
    meeting: {
      ...recordingMeeting,
      call_panel: { state: "recording", recording_id: "rec-review-21", ...CALL },
    },
    ready: "role=button[name=/Preguntar a/]",
    steps: [
      { click: "role=button[name=/Preguntar a/]" },
      { fill: "role=textbox", text: "¿Qué dijo Pablo de Ribera Aeropuerto?" },
    ],
  }),
  panel({
    id: "panel-pensando",
    state: "Deimos piensa y pide permiso",
    description:
      "Una pregunta en curso; mientras, Deimos pide una aprobación que se responde en el panel.",
    size: ANSWER,
    asks: true,
    meeting: {
      ...recordingMeeting,
      question: {
        id: "q-2",
        question: "Mándale a Sara la hora de la formación",
        minutes: 5,
        origin: "voice",
        stage: "thinking",
        sent: true,
      },
    },
    ready: "role=button[name='Permitir una vez']",
  }),
  panel({
    id: "panel-respuesta",
    state: "Respuesta de Deimos",
    description:
      "La respuesta a una pregunta hecha durante la reunión, con un enlace y «Preguntar otra cosa».",
    size: ANSWER,
    meeting: {
      ...recordingMeeting,
      question: {
        id: "q-1",
        question: "¿Qué dijo Pablo de Ribera Aeropuerto?",
        minutes: 5,
        origin: "typed",
        stage: "answered",
        answer,
        sent: true,
      },
    },
    ready: "text=Ribera Aeropuerto",
  }),
];
