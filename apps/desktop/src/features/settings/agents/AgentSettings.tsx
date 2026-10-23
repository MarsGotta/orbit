import { useState } from "react";
import type { Agent } from "../../../bridge/generated/Agent";
import type { AgentConfig } from "../../../bridge/generated/AgentConfig";
import { Tabs } from "../../../design/Tabs";
import { AgentCards } from "./AgentCards";
import styles from "./Agents.module.css";
import { copy } from "./parts";
import type { AgentPort } from "./use-agent-config";

/**
 * Who changes the agent: Hermes in its profile, or Orbit here. A kind that
 * lets nothing be changed says so with the hub's cause.
 */
function kindLine(name: string, config: AgentConfig): string {
  if (config.kind === "hermes") return copy.kind(config.kind, name);
  const { personality, access, skills } = config;
  if (personality.editable || access.editable || skills.editable)
    return copy.kind(config.kind, name);
  return copy.notEditable(personality.cause);
}

/**
 * Ajustes › Asistente: a tab per agent of the hub with its personality, what
 * it may use and its skills, read from the hub and changed there. A switch is
 * configuration, not a confirmation: it is sent as it is flipped, and what
 * the agent's kind does not allow is shown with its cause.
 */
export function AgentSettings({
  agents,
  defaultAgentId,
  agent,
}: {
  agents: Agent[];
  defaultAgentId?: string | null;
  agent?: AgentPort;
}) {
  const [chosen, setChosen] = useState<string | null>(null);
  const target =
    agents.find((candidate) => candidate.id === chosen) ??
    agents.find((candidate) => candidate.id === defaultAgentId) ??
    agents[0];
  if (!target) return <p className={styles.kind}>{copy.none}</p>;
  if (!agent) return <p className={styles.kind}>{copy.unavailable}</p>;
  const panel = (
    <AgentCards
      agent={agent}
      key={target.id}
      lead={(config) => <p className={styles.kind}>{kindLine(target.name, config)}</p>}
      target={target}
    />
  );
  if (agents.length === 1) return panel;
  return (
    <div className={styles.tabs}>
      <Tabs
        label={copy.tabs}
        onSelect={setChosen}
        selected={target.id}
        tabs={agents.map((candidate) => ({ id: candidate.id, label: candidate.name }))}
      >
        {panel}
      </Tabs>
    </div>
  );
}
