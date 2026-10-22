import { emptyChat } from "../data/chat";
import {
  approval,
  approvalQuestion,
  clarify,
  clarifyQuestion,
  failedQuestion,
  question,
  streamingAnswer,
  workInFlight,
} from "../data/chat-turns";
import { approvalWork } from "../data/chat-approval";
import type { Scenario, Views } from "../scenario";
import { notices } from "./edit";

type Chat = Views["assistant"];

export const withChat = (change: Partial<Chat>) => (base: Views) => ({
  ...base,
  assistant: { ...base.assistant, ...change },
});

export const chat = (scenario: Omit<Scenario, "view" | "route">): Scenario => ({
  view: "Asistente",
  route: "#/assistant",
  ...scenario,
});

/** Deimos waits for permission to send the mail, with something written and attached. */
const waiting = withChat({
  turn: [approvalQuestion],
  pending: approval,
  activity: "thinking",
  work: approvalWork,
});

export const chatScenarios: Scenario[] = [
  chat({
    id: "chat-conversacion",
    state: "Conversación",
    description:
      "Respuestas sin burbuja: títulos, listas, una tabla con estados en color y lo que hizo Deimos, plegado.",
    ready: "text=Estado de los cinco hoteles",
    steps: [{ scroll: "text=Estado de los cinco hoteles" }],
  }),
  chat({
    id: "chat-trabajo-abierto",
    state: "«Usó … · 3 pasos» abierto",
    description: "La línea de lo que hizo el agente, abierta: sus pasos con ✓ y el razonamiento.",
    ready: "text=/Usó tus notas y Mail/",
    steps: [{ click: "text=/Usó tus notas y Mail/" }, { scroll: "text=Escríbele a Pablo" }],
  }),
  chat({
    id: "chat-permiso",
    state: "Espera tu permiso",
    description:
      "Deimos quiere enviar un correo: los pasos con ✓, el actual con un punto, la tarjeta con las cuatro respuestas y el compositor que no envía y dice por qué.",
    views: waiting,
    ready: "role=button[name='Permitir una vez']",
    steps: [
      { click: "role=button[name='Adjuntar']" },
      { wait: "role=group[name='Propuesta piloto Ribera.pdf']" },
      {
        fill: "role=textbox[name='Mensaje para Deimos']",
        text: "Y compara la ocupación del piloto con la de septiembre",
      },
    ],
  }),
  chat({
    id: "chat-permiso-dado",
    state: "Permiso dado",
    description:
      "Tras «Permitir una vez»: el resultado, «Usó Mail · 3 pasos · 46 s» y lo que permitiste.",
    views: waiting,
    ready: "role=button[name='Permitir una vez']",
    steps: [{ click: "role=button[name='Permitir una vez']" }, { wait: "text=/ya está enviado/" }],
  }),
  chat({
    id: "chat-permiso-negado",
    state: "No lo permitiste",
    description:
      "Tras «No permitir»: el paso tachado con «no lo permitiste» y la respuesta de Deimos.",
    views: waiting,
    ready: "role=button[name='No permitir']",
    steps: [{ click: "role=button[name='No permitir']" }, { wait: "text=No lo permitiste." }],
  }),
  chat({
    id: "chat-trabajando",
    state: "El agente trabaja",
    description: "Un turno en curso: el tiempo, los pasos con ✓ y el que corre, y el plan.",
    views: withChat({ turn: [question], activity: "thinking", work: workInFlight }),
    ready: "text=/Trabajando · /",
    steps: [{ wait: "text=Estado de los cinco hoteles" }],
  }),
  chat({
    id: "chat-respondiendo",
    state: "Respondiendo",
    description: "La respuesta llega por partes, con «Interrumpir» en la cabecera.",
    views: withChat({
      turn: [question, streamingAnswer],
      activity: "responding",
      work: {
        ...workInFlight,
        items: workInFlight.items.map((item) => ({ ...item, phase: "completed" as const })),
      },
    }),
    ready: "role=button[name='Interrumpir']",
    steps: [{ wait: "text=Estado de los cinco hoteles" }],
  }),
  chat({
    id: "chat-aclaracion",
    state: "Aclaración",
    description:
      "Deimos pregunta antes de seguir: una pregunta de dos, sugerencias que se eligen y texto libre.",
    views: withChat({
      turn: [clarifyQuestion],
      pending: clarify,
      activity: "thinking",
      work: undefined,
    }),
    ready: "text=Pregunta 1 de 2",
    steps: [
      { wait: "text=Estado de los cinco hoteles" },
      { click: "role=button[name='Ribera Centro']" },
      { click: "role=button[name='Casa Ribera']" },
    ],
  }),
  chat({
    id: "chat-fallo",
    state: "Fallo de un turno",
    description:
      "Un turno sin respuesta: su causa con sus números bajo la pregunta y «Volver a preguntar».",
    views: withChat({ turn: [failedQuestion], work: undefined }),
    later: (views) =>
      notices(views, {
        code: "assistant_failed",
        data: { code: "assistant_context_exceeded", tokens: 141_203, max_tokens: 131_072 },
      }),
    ready: "role=alert >> text=/no cabe en el modelo/",
  }),
  chat({
    id: "chat-escuchar",
    state: "Escuchar una respuesta",
    description: "Una respuesta que suena: «Escuchar» pasa a barras que se mueven y «Parar».",
    views: withChat({ speaking: "m6", work: undefined }),
    ready: "role=button[name='Parar']",
  }),
  chat({
    id: "chat-copiado",
    state: "«✓ Copiado»",
    description: "Cada «Copiar» confirma: una respuesta copiada y la conversación entera.",
    views: withChat({ work: undefined }),
    ready: "role=button[name='Copiar respuesta']",
    steps: [
      { click: "role=button[name='Copiar respuesta'] >> nth=-1" },
      { click: "role=button[name='Copiar conversación']" },
      { wait: "text=✓ Copiada" },
    ],
  }),
  chat({
    id: "chat-vacio",
    state: "Conversación nueva",
    description: "Antes del primer mensaje: qué hacer y con qué agente hablar.",
    views: withChat(emptyChat),
    ready: "role=radiogroup[name='Hablar con']",
  }),
  chat({
    id: "chat-cargando",
    state: "Leyendo del hub",
    description: "La lista y la conversación mientras llegan del hub: esqueletos, nunca «vacío».",
    stall: true,
    ready: "[aria-busy='true']",
  }),
  chat({
    id: "chat-sin-conocer",
    state: "Asistente sin conocer",
    description: "Antes de que el Mac diga nada del asistente: lo que lee, con su forma.",
    views: withChat({ availability: { state: "unknown", cause: "pending:not_observed_yet" } }),
    ready: "text=Leyendo tus conversaciones del hub…",
  }),
  chat({
    id: "chat-sin-emparejar",
    state: "Sin emparejar",
    description: "El asistente de un Mac sin emparejar: qué hacer y «Emparejar este Mac».",
    views: (base) => ({
      ...withChat(emptyChat)(base),
      status: {
        ...base.status,
        connection: { daemon: "ready", hub: "unpaired" },
        settings: {
          ...base.status.settings,
          hub: { ...base.status.settings.hub, paired: false, connection: "offline" },
        },
      },
    }),
    hub: (base) => ({ ...base, sessions: [], messages: {} }),
    ready: "role=link[name='Emparejar este Mac']",
  }),
];
