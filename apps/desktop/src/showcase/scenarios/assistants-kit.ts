import type { AgentCall } from "../../bridge/generated/AgentCall";
import type { AgentOutcome } from "../../bridge/generated/AgentOutcome";
import {
  ERIS,
  handedAnswer,
  limited,
  PHOBOS,
  RATES,
  ratesConversation,
  ratesQuestion,
  RESETS,
  waitingAttempt,
  waitingHub,
  withAgent,
} from "../data/assistants";
import { at } from "../data/time";
import type { HubData, Scenario, Views } from "../scenario";
import { notices } from "./edit";

// Several assistants (annex 1, changes 1 and 2): «Tus asistentes», each one's
// page, «¿Con quién quieres hablar?», the pencil, the gear, signing in and
// its failures, the plan's limit and «Añadir un asistente».

export type Answer = { outcome: AgentOutcome; views?: Partial<Views> } | null;

export const assistants = (scenario: Omit<Scenario, "view">): Scenario => ({
  view: "Asistentes",
  ...scenario,
});

/** A conversation not begun, with `agentId`. */
export const fresh =
  (agentId: string, change: (views: Views) => Partial<Views> = () => ({})) =>
  (base: Views): Views => {
    const views = {
      ...base,
      assistant: {
        ...base.assistant,
        session_id: null,
        agent_id: agentId,
        turn: [],
        work: undefined,
      },
    };
    return { ...views, ...change(views) };
  };

export const roster = (id: string, change: Parameters<typeof withAgent>[2]) => (views: Views) =>
  withAgent(views, id, change);

/** The hub that finishes every sign-in at once, as the browser would. */
export function signsIn(change: Parameters<typeof withAgent>[2] = {}) {
  return (call: AgentCall, views: Views): Answer => {
    if (call.call !== "sign_in") return waitingHub(call);
    return {
      outcome: { outcome: "sign_in", attempt: { ...waitingAttempt(call.agent_id), state: "done" } },
      views: withAgent(views, call.agent_id, { account: "signed_in", ...change }),
    };
  };
}

/** The hub that ends every sign-in with `cause`. */
export const failsWith =
  (cause: string) =>
  (call: AgentCall): Answer =>
    call.call === "sign_in"
      ? {
          outcome: {
            outcome: "sign_in",
            attempt: { ...waitingAttempt(call.agent_id), state: "failed", cause },
          },
        }
      : waitingHub(call);

export const PENCIL = "role=button[name='Editar nombre y descripción de Phobos']";
export const sessionsWithRates = (hub: HubData): HubData => ({
  ...hub,
  sessions: [
    {
      id: RATES,
      agent_id: PHOBOS.id,
      created_at_ms: at(0, 10, 10),
      last_at_ms: at(0, 10, 12),
      message_count: 3,
    },
    ...hub.sessions,
  ],
  messages: {
    ...hub.messages,
    [RATES]: ratesConversation,
    "s-sigue": [{ ...ratesQuestion, id: "h1" }, handedAnswer],
  },
});

/** Phobos's conversation, its last turn stopped at the plan's limit. */
export const atLimit = (base: Views): Views => {
  const views = {
    ...base,
    assistant: {
      ...base.assistant,
      session_id: RATES,
      agent_id: PHOBOS.id,
      turn: [ratesQuestion],
      work: undefined,
    },
  };
  return { ...views, ...limited(views) };
};
export const limitNotice = (views: Views) =>
  notices(views, {
    code: "assistant_failed",
    data: { code: "agent_limit_reached", resets_at_ms: String(RESETS) },
  });

/** Eris among the assistants, as the hub makes it: signed out first, then signed in. */
export function addEris(
  views: Views,
  account: "signed_in" | "signed_out",
  name = "Claude",
): Partial<Views> {
  const agents = [
    ...views.status.settings.agents.filter((agent) => agent.id !== ERIS.id),
    { ...ERIS, name, account },
  ];
  return { status: { ...views.status, settings: { ...views.status.settings, agents } } };
}

/** The hub of «Añadir un asistente»: it makes Eris, signs it in at once and renames it. */
export function addingHub(call: AgentCall, views: Views): Answer {
  if (call.call === "add")
    return {
      outcome: { outcome: "added", agent_id: ERIS.id },
      views: addEris(views, "signed_out"),
    };
  if (call.call === "sign_in")
    return {
      outcome: { outcome: "sign_in", attempt: { ...waitingAttempt(call.agent_id), state: "done" } },
      views: addEris(views, "signed_in"),
    };
  if (call.call === "profile")
    return { outcome: { outcome: "done" }, views: addEris(views, "signed_in", call.name) };
  return waitingHub(call);
}

/** Steps 2 to 4 of «Añadir un asistente», each from the one before. */
export function addSteps(): Scenario[] {
  const connected = [
    { click: "role=button[name='Continuar']" },
    { click: "role=button[name='Iniciar sesión en Claude']" },
    { wait: "text=✓ Conectado" },
  ];
  const named = [
    ...connected,
    { click: "role=button[name='Continuar']" },
    { fill: "role=textbox[name='Nombre']", text: "Eris" },
  ];
  const base = {
    route: "#/assistant/add",
    agent: addingHub,
    ready: "text=¿Con qué va a funcionar?",
  };
  return [
    assistants({
      ...base,
      id: "asistentes-anadir-conectado",
      state: "Añadir: paso 2, conectado",
      description: "Sesión iniciada en Claude: «✓ Conectado» y como quién, con su plan.",
      steps: connected,
    }),
    assistants({
      ...base,
      id: "asistentes-anadir-ponle-nombre",
      state: "Añadir: paso 3",
      description:
        "«Ponle nombre»: nombre, color y qué puede usar, que se cambia luego en sus ajustes.",
      steps: named,
    }),
    assistants({
      ...base,
      id: "asistentes-anadir-listo",
      state: "Añadir: paso 4",
      description: "«Eris está listo», con «Hablar con Eris».",
      steps: [
        ...named,
        { click: "role=button[name='Crear asistente']" },
        { wait: "text=Eris está listo" },
      ],
    }),
  ];
}
