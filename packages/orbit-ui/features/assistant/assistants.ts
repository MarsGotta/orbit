import { getCopy } from "../../i18n";
import type { Agent } from "../../bridge/generated/Agent";
import type { AssistantColor } from "../../design/AssistantAvatar";

/**
 * What an assistant can do now, from what the hub last said of it
 * (hub-v1 «Assistants from Orbit»): fine, waiting for the person to sign
 * in, at its plan's limit, or not known yet. An account the hub has not
 * learned is unknown, never «signed in».
 */
export type Health =
  | { kind: "ok" }
  | { kind: "sign_in"; reason: "expired" | "signed_out"; vendor: string }
  | { kind: "limit"; until?: number }
  | { kind: "unknown" };

/** The hub's limit holds until its time passes; one without a time holds until the hub lifts it. */
export function healthOf(agent: Agent, nowMs: number): Health {
  if (agent.account === "expired" || agent.account === "signed_out")
    return {
      kind: "sign_in",
      reason: agent.account,
      vendor:
        getCopy().extras.vendorOf(agent.service) ??
        getCopy().extras.assistantsCopy.serviceName(agent.service) ??
        agent.name,
    };
  if (agent.limit !== undefined) {
    const until = agent.limit.until_ms;
    if (until === undefined) return { kind: "limit" };
    if (until > nowMs) return { kind: "limit", until };
  }
  if (agent.account === "unknown") return { kind: "unknown" };
  return { kind: "ok" };
}

/** The colour it has, or until someone chooses one the four in turn by position, as the hub does. */
export function colorOf(agent: Agent, agents: readonly Agent[]): AssistantColor {
  const chosen = /^c([1-4])$/.exec(agent.color ?? "")?.[1];
  if (chosen !== undefined) return Number(chosen) as AssistantColor;
  const position = Math.max(
    0,
    agents.findIndex((each) => each.id === agent.id),
  );
  return ((position % 4) + 1) as AssistantColor;
}

/** The amber words at the right of its row in «Tus asistentes», when something is wrong. */
export function rowWarning(agent: Agent, nowMs: number): string | null {
  const health = healthOf(agent, nowMs);
  if (health.kind === "sign_in") return getCopy().extras.assistantsCopy.rowSignIn;
  if (health.kind === "limit")
    return health.until === undefined
      ? getCopy().extras.assistantsCopy.rowLimit
      : getCopy().extras.assistantsCopy.rowUntil(health.until);
  return null;
}

/**
 * Its status under its name: available (with the plan's usage only when
 * the provider reported it), or what stops it. Unknown says nothing.
 */
export function statusLine(
  agent: Agent,
  nowMs: number,
): { tone: "ok" | "warn"; text: string } | null {
  const health = healthOf(agent, nowMs);
  switch (health.kind) {
    case "sign_in":
      return {
        tone: "warn",
        text:
          health.reason === "expired"
            ? getCopy().extras.assistantsCopy.expired(health.vendor)
            : getCopy().extras.assistantsCopy.signedOut(health.vendor),
      };
    case "limit":
      return {
        tone: "warn",
        text:
          health.until === undefined
            ? getCopy().extras.assistantsCopy.limitUnknown
            : getCopy().extras.assistantsCopy.limitUntil(health.until, nowMs),
      };
    case "unknown":
      return null;
    case "ok":
      if (agent.usage !== undefined)
        return {
          tone: "ok",
          text: getCopy().extras.assistantsCopy.availableUsage(
            Math.round(agent.usage.used_percent),
            agent.usage.window,
          ),
        };
      return {
        tone: "ok",
        text:
          agent.service === "hermes"
            ? getCopy().extras.assistantsCopy.availableHub
            : getCopy().extras.assistantsCopy.available,
      };
  }
}

/** Why the composer cannot send to this assistant now; what is written stays. */
export function composerBlock(agent: Agent | undefined, nowMs: number): string | null {
  if (agent === undefined) return null;
  const health = healthOf(agent, nowMs);
  if (health.kind === "sign_in")
    return getCopy().extras.assistantsCopy.composerSignIn(agent.name, health.vendor);
  if (health.kind === "limit")
    return getCopy().extras.assistantsCopy.composerLimit(agent.name, health.until);
  return null;
}

/** The hub's ceiling on a name (hub-v1 `PUT /v1/agents/{id}/profile`). */
export const NAME_MAX = 24;
/** The hub's ceiling on a description. */
export const DESCRIPTION_MAX = 90;

/** A name as the wake phrase hears it, and as the hub compares it: no case, no accents. */
export function nameKey(name: string): string {
  return name.trim().normalize("NFD").replace(/\p{M}/gu, "").toLocaleLowerCase("es");
}

/** What is wrong with a name for `agentId`, or null; the hub checks the same. */
export function profileProblem(
  name: string,
  agentId: string,
  agents: readonly Agent[],
): "empty" | "too_long" | "taken" | null {
  const trimmed = name.trim();
  if (trimmed === "") return "empty";
  if (Array.from(trimmed).length > NAME_MAX) return "too_long";
  const key = nameKey(trimmed);
  if (agents.some((agent) => agent.id !== agentId && nameKey(agent.name) === key)) return "taken";
  return null;
}
