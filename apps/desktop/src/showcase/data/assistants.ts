import type { Agent } from "../../bridge/generated/Agent";
import type { AgentCall } from "../../bridge/generated/AgentCall";
import type { AgentOutcome } from "../../bridge/generated/AgentOutcome";
import type { SignInAttempt } from "../../bridge/generated/SignInAttempt";
import type { Views } from "../scenario";
import { message } from "./chat";
import { ARES, DEIMOS, PHOBOS } from "./status";
import { at } from "./time";

/** Phobos's conversation about the rates, its second question stopped at the plan's limit. */
export const RATES = "s-tarifas";
export const RESETS = at(0, 18, 40);

export const ratesAsked = message({
  id: "t1",
  role: "user",
  text: "Dame las tarifas medias por hotel de esta semana.",
  at_ms: at(0, 10, 10),
});

export const ratesQuestion = message({
  id: "t3",
  role: "user",
  text: "Compáralas con las de septiembre y dime qué hotel ha subido más.",
  at_ms: at(0, 10, 12),
});

export const ratesConversation = [
  ratesAsked,
  message({
    id: "t2",
    role: "assistant",
    text: "La media de la semana es de 112 € por noche. Casa Ribera es la más cara (148 €) y Ribera Aeropuerto la más barata (84 €).",
    at_ms: at(0, 10, 11),
  }),
  ratesQuestion,
];

/** Deimos's answer once the conversation was handed over to it. */
export const handedAnswer = message({
  id: "h2",
  role: "assistant",
  text: "Respecto a septiembre, el que más sube es **Ribera Playa**: de 96 € a 108 € (+12,5 %). Casa Ribera sube un 6 % y Ribera Aeropuerto baja un 3 %.",
  at_ms: at(0, 10, 14),
});

/** The roster with one assistant changed. */
export function withAgent(views: Views, id: string, change: Partial<Agent>): Partial<Views> {
  const agents = views.status.settings.agents.map((agent) =>
    agent.id === id ? { ...agent, ...change } : agent,
  );
  return { status: { ...views.status, settings: { ...views.status.settings, agents } } };
}

export const limited = (views: Views) =>
  withAgent(views, PHOBOS.id, { limit: { until_ms: RESETS } });

/** A sign-in the hub keeps waiting: Claude's page asks for its code back, Codex's shows one. */
export function waitingAttempt(agentId: string): SignInAttempt {
  const claude = agentId === ARES.id || agentId.startsWith("claude");
  return {
    agent_id: agentId,
    state: "waiting",
    expects_code: claude,
    url: claude ? "https://claude.example/oauth/authorize" : "https://auth.example/codex/device",
    ...(claude ? {} : { code: "ABCD-1234" }),
    expires_at_ms: at(0, 10, 55),
  };
}

/** The hub as these scenarios need it: it waits for every sign-in, and takes every other call. */
export function waitingHub(call: AgentCall): { outcome: AgentOutcome } | null {
  if (call.call === "sign_in" || call.call === "sign_in_status")
    return { outcome: { outcome: "sign_in", attempt: waitingAttempt(call.agent_id) } };
  if (call.call === "config" || call.call === "skill") return null;
  return { outcome: { outcome: "done" } };
}

/** Eris, the Claude assistant «Añadir un asistente» makes, once signed in. */
export const ERIS: Agent = {
  id: "claude-0a0b0c0d",
  name: "Claude",
  voice: false,
  service: "claude-code",
  color: "c4",
  removable: true,
  account: "signed_in",
  account_email: "lucia.ferrer@nubaria.example",
  plan: "Claude Pro",
};

export { ARES, DEIMOS, PHOBOS };
