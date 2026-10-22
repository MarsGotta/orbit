import { useState } from "react";
import type { Agent } from "../../../bridge/generated/Agent";
import { Button } from "../../../design/Button";
import { SectionLabel } from "../../../design/SectionLabel";
import settingsStyles from "../SettingsView.module.css";
import { AccessCard } from "./AccessCard";
import styles from "./Agents.module.css";
import { copy, Panel } from "./parts";
import { PersonalityCard } from "./PersonalityCard";
import { SkillsCard } from "./SkillsCard";
import { type AgentPort, useAgentConfig } from "./use-agent-config";

function AgentPanel({ agent, target }: { agent: AgentPort; target: Agent }) {
  const { view, feedback, busy, reload, order, skill, say } = useAgentConfig(agent, target.id);
  if (view.state === "loading") return <p className={styles.hint}>{copy.loading(target.name)}</p>;
  if (view.state === "failed")
    return (
      <Panel>
        <p className={styles.alert} role="alert">
          {view.text}
        </p>
        <div>
          <Button onClick={reload}>{copy.readAgain}</Button>
        </div>
      </Panel>
    );
  const { config } = view;
  const props = { agentId: target.id, busy, feedback, order };
  return (
    <div className={styles.card}>
      <p className={styles.hint}>{copy.kind(config.kind)}</p>
      <PersonalityCard
        // A new text from the hub replaces the editor; one unchanged keeps
        // what the user is writing.
        key={config.personality.text}
        name={target.name}
        personality={config.personality}
        {...props}
      />
      <AccessCard access={config.access} {...props} />
      <SkillsCard read={skill} say={say} skills={config.skills} {...props} />
    </div>
  );
}

/**
 * Ajustes › Agentes: each agent of the hub with its personality, its skills
 * and the groups of Orbit's tools it may use, read from the hub and changed
 * there. A switch is configuration, not a confirmation: it is sent as it is
 * flipped, and what the agent's kind does not allow is shown with its cause.
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
  return (
    <section aria-label={copy.section} className={settingsStyles.section}>
      <SectionLabel>{copy.section}</SectionLabel>
      {!target ? (
        <Panel>
          <p>{copy.none}</p>
        </Panel>
      ) : !agent ? (
        <Panel>
          <p>{copy.unavailable}</p>
        </Panel>
      ) : (
        <>
          {agents.length > 1 && (
            <div className={styles.picker}>
              {agents.map((candidate) => (
                <Button
                  aria-pressed={candidate.id === target.id}
                  key={candidate.id}
                  onClick={() => setChosen(candidate.id)}
                >
                  {candidate.name}
                </Button>
              ))}
            </div>
          )}
          <AgentPanel agent={agent} key={target.id} target={target} />
        </>
      )}
    </section>
  );
}
