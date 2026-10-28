import { getCopy } from "../../i18n";
import type { Agent } from "../../bridge/generated/Agent";

import type { AppView as AppSnapshot } from "../../state/app-view";
import { agentsOf } from "../assistant/agents";

/**
 * Who is who in the first run: the hub's own assistant (the one Orbit does
 * not sign in), the ones with a subscription, and the primary. Before the
 * hub has said its agents, the hub's assistant goes by the name the hub
 * last gave it.
 */
export interface Cast {
  agents: Agent[];
  hubName: string;
  others: Agent[];
  primaryId: string | null;
  primaryName: string;
}

export function castOf(snapshot: AppSnapshot): Cast {
  const agents = agentsOf(snapshot);
  const hubAgent = agents.find((agent) => agent.account === undefined);
  const hubName = hubAgent?.name ?? snapshot.settings.assistant_name ?? getCopy().settings.deimos;
  const primaryId = snapshot.settings.default_agent_id ?? null;
  const primaryName = agents.find((agent) => agent.id === primaryId)?.name ?? hubName;
  const others = agents.filter((agent) => agent.account !== undefined);
  return { agents, hubName, others, primaryId, primaryName };
}

/**
 * The assistants that can answer now: the hub's once this Mac is paired,
 * one with a subscription once it is signed in. Before the hub has said its
 * agents, a paired Mac has the hub's own.
 */
export function readyAgents(cast: Cast, paired: boolean): Pick<Agent, "id" | "name">[] {
  if (cast.agents.length === 0) return paired ? [{ id: "", name: cast.hubName }] : [];
  return cast.agents.filter((agent) =>
    agent.account === undefined ? paired : agent.account === "signed_in",
  );
}
