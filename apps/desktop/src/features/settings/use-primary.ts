import { useEffect, useState } from "react";
import type { Agent } from "../../bridge/generated/Agent";
import { callAgent } from "../assistant/agent-calls";
import { colorOf } from "../assistant/assistants";
import type { AgentPort } from "./agents/use-agent-config";

/**
 * Choosing the primary assistant, which the hub keeps (hub-v1 `PUT
 * /v1/agents/{id}/profile`, whose profile carries all four fields). The one
 * chosen shows until the hub's own word says it is the primary; a refusal
 * says why and leaves the one there was.
 */
export function usePrimary(
  port: AgentPort | undefined,
  agents: readonly Agent[],
  primaryId: string | null | undefined,
) {
  const [chosen, setChosen] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [refusal, setRefusal] = useState<string | null>(null);
  useEffect(() => {
    if (chosen !== null && chosen === primaryId) setChosen(null);
  }, [chosen, primaryId]);
  const choose = async (agentId: string) => {
    const agent = agents.find((each) => each.id === agentId);
    if (agent === undefined || agentId === (chosen ?? primaryId) || saving) return;
    setChosen(agentId);
    setSaving(true);
    setRefusal(null);
    const result = await callAgent(port, {
      call: "profile",
      agent_id: agent.id,
      name: agent.name,
      description: agent.description ?? "",
      color: `c${colorOf(agent, agents)}`,
      primary: true,
    });
    setSaving(false);
    if (!result.ok) {
      setChosen(null);
      setRefusal(result.text);
    }
  };
  return { current: chosen ?? primaryId ?? null, saving, refusal, choose };
}
