import type { ReactNode } from "react";
import type { Agent } from "../../../bridge/generated/Agent";
import type { AgentConfig } from "../../../bridge/generated/AgentConfig";
import { Button } from "../../../design/Button";
import { useFeedback } from "../../../design/feedback";
import { Skeleton } from "../../../design/Skeleton";
import { SkeletonRows } from "../layout";
import { AccessCard } from "./AccessCard";
import styles from "./Agents.module.css";
import { copy } from "./parts";
import { PersonalityCard } from "./PersonalityCard";
import { SkillsCard } from "./SkillsCard";
import { type AgentPort, useAgentConfig } from "./use-agent-config";

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

/**
 * One agent's personality, what it may use and its skills, read from the hub
 * and changed there; `lead` goes above them once the configuration is read.
 * Ajustes › Asistente and each assistant's own settings (its gear) show it.
 */
export function AgentCards({
  agent,
  target,
  lead,
}: {
  agent: AgentPort;
  target: Agent;
  lead?: (config: AgentConfig) => ReactNode;
}) {
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
      {lead?.(config)}
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
