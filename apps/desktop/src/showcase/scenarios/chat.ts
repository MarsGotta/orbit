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
import type { Scenario, Views } from "../scenario";
import { notices } from "./edit";

type Chat = Views["assistant"];

const withChat = (change: Partial<Chat>) => (base: Views) => ({
  ...base,
  assistant: { ...base.assistant, ...change },
});

export const chat = (scenario: Omit<Scenario, "view" | "route">): Scenario => ({
  view: "Asistente",
  route: "#/assistant",
  ...scenario,
});

export const chatScenarios: Scenario[] = [
  chat({
    id: "chat-vacio",
    state: "Conversación nueva",
    description:
      "Una conversación sin mensajes, con el selector «Hablar con» y la lista de conversaciones.",
    views: withChat(emptyChat),
    ready: "text=Aún no hay mensajes",
  }),
  chat({
    id: "chat-conversacion",
    state: "Conversación larga",
    description:
      "Respuestas en Markdown: títulos, listas, una tabla y una cita, con lo que Deimos leyó.",
    ready: "text=Estado de los cinco hoteles",
    steps: [{ scroll: "text=Para la llamada de las 16:00" }],
  }),
  chat({
    id: "chat-conversacion-codigo",
    state: "Conversación con código",
    description:
      "Un bloque de código resaltado, el borrador de un correo y las acciones de cada respuesta.",
    ready: "text=Estado de los cinco hoteles",
    steps: [{ wait: "pre code" }, { scroll: "text=Genial. Y necesito la ocupación" }],
  }),
  chat({
    id: "chat-trabajo-terminado",
    state: "«Trabajó…» abierto",
    description:
      "Lo que hizo el agente en el último turno, plegado en una línea y abierto: pasos y razonamiento.",
    ready: "text=/Trabajó/",
    steps: [
      { click: "text=/Trabajó/" },
      { click: "text=Razonamiento" },
      { scroll: "text=Escríbele a Pablo" },
    ],
  }),
  chat({
    id: "chat-trabajando",
    state: "El agente trabaja",
    description:
      "Un turno en curso: el tiempo, las herramientas que corren, el plan con el estado de cada paso.",
    views: withChat({ turn: [question], activity: "thinking", work: workInFlight }),
    ready: "text=Trabajando 42 s",
    steps: [
      { wait: "text=Estado de los cinco hoteles" },
      { click: "text=Razonamiento" },
      { scroll: "text=Compara la ocupación del piloto" },
    ],
  }),
  chat({
    id: "chat-respondiendo",
    state: "Respondiendo",
    description: "La respuesta llega por partes, con el cursor y el botón para interrumpir.",
    views: withChat({
      turn: [question, streamingAnswer],
      activity: "responding",
      work: {
        ...workInFlight,
        items: workInFlight.items.map((item) => ({ ...item, phase: "completed" as const })),
      },
    }),
    ready: "role=button[name='Interrumpir']",
    steps: [
      { wait: "text=Estado de los cinco hoteles" },
      { scroll: "text=Compara la ocupación del piloto" },
    ],
  }),
  chat({
    id: "chat-aprobacion",
    state: "Aprobación pendiente",
    description: "El agente pide permiso para una acción, con las opciones que ofrece.",
    views: withChat({ turn: [approvalQuestion], pending: approval, work: undefined }),
    ready: "role=button[name='Permitir una vez']",
    steps: [{ wait: "text=Estado de los cinco hoteles" }, { scroll: "text=Está bien así" }],
  }),
  chat({
    id: "chat-aclaracion",
    state: "Aclaración",
    description:
      "El agente pregunta antes de seguir: una pregunta de varias, con sugerencias y texto libre.",
    views: withChat({ turn: [clarifyQuestion], pending: clarify, work: undefined }),
    ready: "text=Pregunta 1 de 2",
    steps: [
      { wait: "text=Estado de los cinco hoteles" },
      { scroll: "text=Prepárame el informe" },
      { click: "role=button[name='Ribera Centro']" },
      { click: "role=button[name='Casa Ribera']" },
    ],
  }),
  chat({
    id: "chat-fallo",
    state: "Fallo con su causa",
    description:
      "Un turno que el proveedor no pudo responder, con la causa y sus números en el aviso.",
    views: withChat({ turn: [failedQuestion], work: undefined }),
    later: (views) =>
      notices(views, {
        code: "assistant_failed",
        data: { code: "assistant_context_exceeded", tokens: 141_203, max_tokens: 131_072 },
      }),
    ready: "text=/no cabe en el modelo/",
    steps: [
      { wait: "text=Estado de los cinco hoteles" },
      { scroll: "text=/Resúmeme todas las conversaciones/" },
    ],
  }),
  chat({
    id: "chat-sin-emparejar",
    state: "Sin emparejar",
    description:
      "El asistente de un Mac sin emparejar: no se puede escribir y dice dónde emparejar.",
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
    ready: "role=link",
  }),
];
