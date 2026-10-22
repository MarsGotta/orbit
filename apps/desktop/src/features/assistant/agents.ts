import type { Agent } from "../../bridge/generated/Agent";
import type { AppView } from "../../state/app-view";

/**
 * The agents a conversation can be with, in the hub's order. A hub without
 * agents has one; before the daemon says, none are known.
 */
export function agentsOf(snapshot?: AppView | null): Agent[] {
  return snapshot?.settings.agents ?? [];
}

/** The agent a conversation is with: its own, or else the default one. */
export function agentFor(
  snapshot: AppView | null | undefined,
  agentId?: string | null,
): Agent | undefined {
  const agents = agentsOf(snapshot);
  const id = agentId ?? snapshot?.settings.default_agent_id;
  return agents.find((agent) => agent.id === id);
}

/**
 * Whether an answer of a conversation with `agentId` can be read aloud: in
 * its agent's own voice, or in the default agent's, which the hub reads an
 * agent without one in. Only an agent that says it has no voice counts as
 * none: one of a hub without agents (0.9.0) does not say, and is offered.
 */
export function readsAloud(snapshot: AppView | null | undefined, agentId?: string | null): boolean {
  return agentFor(snapshot, agentId)?.voice !== false || agentFor(snapshot)?.voice !== false;
}

/**
 * The name every text uses: the open conversation's agent; before the
 * agents are known, the hub's `assistant_name` (and «Deimos» before that).
 */
export function conversationAgentName(snapshot?: AppView | null): string | undefined {
  return agentFor(snapshot, snapshot?.chat?.agent_id)?.name ?? snapshot?.settings.assistant_name;
}
