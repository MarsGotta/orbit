import type { AgentWork } from "../../bridge/generated/AgentWork";
import type { ChatMessage } from "../../bridge/generated/ChatMessage";
import type { ChatSessionSummary } from "../../bridge/generated/ChatSessionSummary";
import type { Views } from "../scenario";
import { mailAnswer, prepAnswer, queryAnswer } from "./chat-answers";
import { at, MINUTE } from "./time";

export const SESSION = "s-ribera";

type Draft = Pick<ChatMessage, "id" | "role" | "text" | "at_ms"> & Partial<ChatMessage>;

export function message(draft: Draft): ChatMessage {
  return { streaming: false, voice: null, receipts: [], ...draft };
}

export const conversation: ChatMessage[] = [
  message({
    id: "m1",
    role: "user",
    text: "Prepárame la llamada de esta tarde con Hoteles Ribera. ¿Qué quedó pendiente?",
    at_ms: at(0, 9, 31),
  }),
  message({
    id: "m2",
    role: "assistant",
    text: prepAnswer,
    at_ms: at(0, 9, 32),
    receipts: [
      {
        kind: "read",
        label: "Leí la nota «Hoteles Ribera · arranque del piloto»",
        path: "notes/rec-ribera-09",
      },
      {
        kind: "read",
        label: "Leí la nota «Revisión del diseño de reservas»",
        path: "notes/rec-review-14",
      },
    ],
  }),
  message({
    id: "m3",
    role: "user",
    text: "Genial. Y necesito la ocupación por hotel y día de la semana del piloto para el informe.",
    at_ms: at(0, 9, 40),
  }),
  message({ id: "m4", role: "assistant", text: queryAnswer, at_ms: at(0, 9, 41) }),
  message({
    id: "m5",
    role: "user",
    text: "Escríbele a Pablo confirmando el lunes 26 y la formación de Ribera Playa.",
    at_ms: at(0, 10, 2),
  }),
  message({
    id: "m6",
    role: "assistant",
    text: mailAnswer,
    at_ms: at(0, 10, 3),
    receipts: [
      { kind: "written", label: "Escribí un borrador en Mail", path: "mail/drafts/ribera" },
    ],
  }),
];

/** What Deimos did for the last question: looked up the note and wrote in Mail. */
export const mailWork: AgentWork = {
  turn_id: "m5",
  started_at_ms: at(0, 10, 2),
  ended_at_ms: at(0, 10, 2) + 52_000,
  tools: 3,
  subjects: ["tus notas", "Mail"],
  items: [
    {
      id: "w1",
      kind: "reasoning",
      phase: "completed",
      label:
        "Busco la formación pendiente en la nota del piloto y redacto un correo corto, sin enviarlo.",
      steps: [],
    },
    {
      id: "w2",
      kind: "tool",
      phase: "completed",
      label: "Leer la nota «Hoteles Ribera · arranque del piloto»",
      steps: [],
    },
    { id: "w3", kind: "tool", phase: "completed", label: "Abrir Mail", steps: [] },
    {
      id: "w4",
      kind: "tool",
      phase: "completed",
      label: "Escribir un borrador para Pablo Medina",
      steps: [],
    },
  ],
};

export const sessions: ChatSessionSummary[] = [
  {
    id: SESSION,
    agent_id: "deimos",
    created_at_ms: at(0, 9, 31),
    last_at_ms: at(0, 10, 3),
    message_count: 6,
  },
  {
    id: "s-code",
    agent_id: "claude-code",
    created_at_ms: at(-1, 16, 0),
    last_at_ms: at(-1, 16, 40),
    message_count: 12,
  },
  {
    id: "s-ocupacion",
    agent_id: "deimos",
    created_at_ms: at(-1, 11, 15),
    last_at_ms: at(-1, 11, 20),
    message_count: 4,
  },
  {
    id: "s-codex",
    agent_id: "codex",
    created_at_ms: at(-3, 18, 2),
    last_at_ms: at(-3, 18, 30),
    message_count: 8,
  },
  {
    id: "s-viaje",
    agent_id: "deimos",
    created_at_ms: at(-6, 20, 10),
    last_at_ms: at(-6, 20, 14),
    message_count: 2,
  },
];

export const chat: Views["assistant"] = {
  availability: { state: "current" },
  session_id: SESSION,
  agent_id: "deimos",
  turn: [],
  pending: null,
  voice: { state: "idle", level: 0, availability: { state: "current" } },
  work: mailWork,
};

/** A conversation not yet begun: no session, nothing said. */
export const emptyChat: Views["assistant"] = {
  ...chat,
  session_id: null,
  turn: [],
  work: undefined,
};

export const minutesAgo = (minutes: number) => at(0, 10, 40) - minutes * MINUTE;
