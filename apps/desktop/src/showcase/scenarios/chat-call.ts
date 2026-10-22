import { NOW } from "../data/time";
import type { Scenario, Views } from "../scenario";
import { chat, withChat } from "./chat";

const lastTurnDone = withChat({ work: undefined });

/** A call with Deimos, 4 min 12 s in, while Deimos speaks. */
const inCall = (base: Views): Views => ({
  ...base,
  assistant: {
    ...base.assistant,
    work: undefined,
    turn: [
      {
        id: "c1",
        role: "user",
        text: "",
        at_ms: NOW - 20_000,
        streaming: false,
        receipts: [],
        voice: {
          duration_ms: 3_000,
          transcript: "¿Qué hotel va peor comparado con septiembre?",
        },
      },
    ],
  },
  recording: {
    ...base.recording,
    call: {
      started_at_ms: NOW - 252_000,
      seeing_screen: true,
      last_line:
        "Ribera Aeropuerto: va 16 puntos por debajo de septiembre. ¿Quieres que te prepare un correo para Pablo con los datos?",
      speaking: true,
      expires_at_ms: NOW - 252_000 + 60 * 60_000,
      origin: "app",
      idle_hangup_ms: 60_000,
    },
  },
});

/** The call with the agent: its own view, while it speaks or listens, minimised, beginning. */
export const chatCallScenarios: Scenario[] = [
  chat({
    id: "chat-llamada",
    state: "Llamada con Deimos",
    description:
      "La llamada: el círculo con las barras mientras Deimos habla, el cronómetro, lo último que se dijo y los límites.",
    views: inCall,
    ready: "role=button[name='Colgar']",
  }),
  chat({
    id: "chat-llamada-escucha",
    state: "Llamada, te escucha",
    description: "La misma llamada mientras hablas tú: las barras quietas y «Te escucha».",
    views: (base) => {
      const views = inCall(base);
      const call = views.recording.call;
      return {
        ...views,
        recording: { ...views.recording, call: call && { ...call, speaking: false } },
      };
    },
    ready: "text=Te escucha",
  }),
  chat({
    id: "chat-llamada-minimizada",
    state: "Llamada minimizada",
    description:
      "La conversación con la tarjeta flotante «En llamada con Deimos», «Abrir» y «Colgar».",
    views: inCall,
    ready: "role=button[name='Minimizar']",
    steps: [
      { click: "role=button[name='Minimizar']" },
      { wait: "role=region[name='Llamada minimizada']" },
    ],
  }),
  chat({
    id: "chat-llamando",
    state: "Llamando",
    description: "Tras «Llamar a Deimos», mientras la llamada empieza.",
    views: lastTurnDone,
    ready: "role=button[name='Llamar a Deimos']",
    steps: [{ click: "role=button[name='Llamar a Deimos']" }, { wait: "text=Llamando a Deimos…" }],
  }),
];
