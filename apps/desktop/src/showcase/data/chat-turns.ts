import type { AgentWork } from "../../bridge/generated/AgentWork";
import type { ChatMessage } from "../../bridge/generated/ChatMessage";
import type { ChatPending } from "../../bridge/generated/ChatPending";
import { message } from "./chat";
import { NOW } from "./time";

/** The question of the turn in progress, asked 42 seconds ago. */
export const question = message({
  id: "m7",
  role: "user",
  text: "Compara la ocupación del piloto con la de septiembre y dime qué hotel va peor.",
  at_ms: NOW - 42_000,
});

export const workInFlight: AgentWork = {
  turn_id: "m7",
  started_at_ms: NOW - 42_000,
  tools: 2,
  subjects: ["la base de reservas"],
  items: [
    {
      id: "r1",
      kind: "reasoning",
      phase: "completed",
      label: "Necesito la ocupación de las dos semanas; luego comparo hotel por hotel.",
      steps: [],
    },
    {
      id: "t1",
      kind: "tool",
      phase: "completed",
      label: "Leer la nota «Hoteles Ribera · arranque del piloto»",
      steps: [],
    },
    {
      id: "t2",
      kind: "tool",
      phase: "progress",
      label: "Consultar la base de reservas: ocupación de septiembre",
      steps: [],
    },
    {
      id: "t3",
      kind: "subagent",
      phase: "started",
      label: "Analista de datos: comparar las dos series",
      steps: [],
    },
    {
      id: "p1",
      kind: "plan",
      phase: "progress",
      label: "Plan",
      steps: [
        { text: "Leer la ocupación de la semana del piloto", status: "completed" },
        { text: "Leer la de septiembre", status: "in_progress" },
        { text: "Comparar hotel por hotel", status: "pending" },
        { text: "Explicar la diferencia del peor", status: "pending" },
      ],
    },
  ],
};

/** The answer as it arrives, cut where the stream is. */
export const streamingAnswer = message({
  id: "m8",
  role: "assistant",
  text: "Comparando con septiembre, la ocupación media del piloto sube **6 puntos** (del 71 % al 77 %).\n\nEl que va peor es **Ribera Aeropuerto**:\n\n- 58 % de ocupación frente al 74 % de septiembre\n- tres cancelaciones de grupos el martes",
  at_ms: NOW - 3_000,
  streaming: true,
});

export const approval: ChatPending = {
  kind: "approval",
  id: "ap-1",
  text: "Deimos quiere enviar desde Mail el correo «Arranque del piloto el lunes 26» a pablo@hotelesribera.example.",
  choices: ["once", "session", "always", "deny"],
};

export const approvalQuestion = message({
  id: "m7",
  role: "user",
  text: "Está bien así. Envíaselo a Pablo.",
  at_ms: NOW - 20_000,
});

export const clarify: ChatPending = {
  kind: "clarify",
  id: "cl-1",
  text: "Antes de preparar el informe necesito dos datos.",
  choices: [],
  questions: [
    {
      qid: "q1",
      question: "¿Qué hoteles incluyo en el informe de ocupación?",
      choices: ["Ribera Centro", "Ribera Playa", "Ribera Golf", "Casa Ribera", "Ribera Aeropuerto"],
      multi_select: true,
    },
    {
      qid: "q2",
      question: "¿En qué formato lo quieres?",
      choices: ["PDF", "Hoja de cálculo"],
      multi_select: false,
    },
  ],
};

export const clarifyQuestion = message({
  id: "m7",
  role: "user",
  text: "Prepárame el informe de ocupación del piloto para el viernes.",
  at_ms: NOW - 15_000,
});

export const failedQuestion = message({
  id: "m7",
  role: "user",
  text: "Resúmeme todas las conversaciones de este mes con Hoteles Ribera, una por una.",
  at_ms: NOW - 60_000,
});

export const attachmentsMessages: ChatMessage[] = [
  message({
    id: "a1",
    role: "user",
    text: "Mira el boceto de la pizarra y la propuesta. ¿Encaja con lo que acordamos?",
    at_ms: NOW - 8 * 60_000,
    attachments: [
      {
        id: "photo-1",
        name: "Pizarra reservas.png",
        kind: "image",
        mime: "image/png",
        bytes: 1_840_000,
      },
      {
        id: "chart-1",
        name: "ocupacion-semana-44.png",
        kind: "image",
        mime: "image/png",
        bytes: 412_000,
      },
      {
        id: "pdf-1",
        name: "Propuesta piloto Ribera.pdf",
        kind: "pdf",
        mime: "application/pdf",
        bytes: 2_310_000,
        pages: 4,
      },
    ],
  }),
  message({
    id: "a2",
    role: "assistant",
    text: "Sí, encaja. El boceto ya junta **fechas, habitación y pago** en una sola pantalla, como decidisteis.\n\nDos cosas que no veo en la propuesta:\n\n1. El estado vacío cuando no hay habitaciones esa semana.\n2. Qué pasa con Ribera Aeropuerto si la caché no llega el jueves.",
    at_ms: NOW - 7 * 60_000,
  }),
];

export const voiceMessages: ChatMessage[] = [
  message({
    id: "v1",
    role: "user",
    text: "",
    at_ms: NOW - 12 * 60_000,
    voice: {
      duration_ms: 9_000,
      transcript: "Apúntame que mañana llame a Sara para lo de la formación de Ribera Playa.",
      audio_available: true,
    },
  }),
  message({
    id: "v2",
    role: "assistant",
    text: "Hecho: mañana a las **10:00** tienes «Llamar a Sara: formación de Ribera Playa» en Recordatorios.",
    at_ms: NOW - 12 * 60_000 + 6_000,
    receipts: [
      {
        kind: "reminder",
        label: "Creé un recordatorio para mañana a las 10:00",
        path: "reminders/sara",
      },
    ],
  }),
  message({
    id: "v3",
    role: "user",
    text: "",
    at_ms: NOW - 2 * 60_000,
    voice: {
      duration_ms: 14_000,
      failure: {
        code: "voice_transcription_failed",
        provider_status: 503,
        hub_status: 502,
        timeout_ms: null,
        bytes: null,
        max_bytes: null,
        attempts: 3,
      },
      audio_available: true,
    },
  }),
];
