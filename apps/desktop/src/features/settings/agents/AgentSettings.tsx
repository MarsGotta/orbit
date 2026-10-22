import { useState } from "react";
import type { Agent } from "../../../bridge/generated/Agent";
import type { AgentConfig } from "../../../bridge/generated/AgentConfig";
import { Button } from "../../../design/Button";
import { useFeedback } from "../../../design/feedback";
import { Skeleton } from "../../../design/Skeleton";
import { Tabs } from "../../../design/Tabs";
import { SkeletonRows } from "../layout";
import { AccessCard } from "./AccessCard";
import styles from "./Agents.module.css";
import { copy } from "./parts";
import { PersonalityCard } from "./PersonalityCard";
import { SkillsCard } from "./SkillsCard";
import { type AgentPort, useAgentConfig } from "./use-agent-config";

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

/** The configuration still coming from the hub: its shape, and what is being read. */
function Loading({ name }: { name: string }) {
  return (
    <div aria-busy="true" className={styles.panel}>
      <p className={styles.kind} role="status">
        {copy.loading(name)}
      </p>
      <div className={styles.personality}>
        <Skeleton widths={["92%", "78%", "40%"]} />
      </div>
      <SkeletonRows rows={5} />
    </div>
  );
}

function AgentPanel({ agent, target }: { agent: AgentPort; target: Agent }) {
  const { view, feedback, busy, reload, order, skill, say } = useAgentConfig(agent, target.id);
  const personalitySave = useFeedback();
  if (view.state === "loading") return <Loading name={target.name} />;
  if (view.state === "failed")
    return (
      <div className={styles.panel}>
        <p className={styles.alert} role="alert">
          {view.text}
        </p>
        <div>
          <Button onClick={reload} size={32}>
            {copy.readAgain}
          </Button>
        </div>
      </div>
    );
  const { config } = view;
  // Hermes's line already says where each part is changed.
  const explained = config.kind === "hermes";
  const props = { agentId: target.id, busy, feedback, order, explained };
  return (
    <div className={styles.panel}>
      <p className={styles.kind}>{kindLine(target.name, config)}</p>
      <PersonalityCard
        // A new text from the hub replaces the editor; one unchanged keeps
        // what the user is writing.
        key={config.personality.text}
        name={target.name}
        personality={config.personality}
        saving={personalitySave}
        {...props}
      />
      <AccessCard access={config.access} {...props} />
      <SkillsCard read={skill} say={say} skills={config.skills} {...props} />
    </div>
  );
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
  const panel = <AgentPanel agent={agent} key={target.id} target={target} />;
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
