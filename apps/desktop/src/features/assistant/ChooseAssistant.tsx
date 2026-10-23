import { useId } from "react";
import type { AppCommand } from "../../bridge/generated";
import type { Agent } from "../../bridge/generated/Agent";
import { AssistantAvatar } from "../../design/AssistantAvatar";
import { assistantsCopy as copy } from "../../i18n/es/assistants";
import type { AppView as AppSnapshot } from "../../state/app-view";
import { useHeldChoice } from "../../state/use-held-choice";
import type { AgentPort } from "../settings/agents/use-agent-config";
import { colorOf } from "./assistants";
import { agentFor, agentsOf } from "./agents";
import { StatusLine } from "./AssistantHome";
import { SignInNotice } from "./SignInNotice";
import styles from "./Assistants.module.css";

type Send = (command: AppCommand) => unknown;

/** One assistant to choose: named by its name, described by the rest. */
function AssistantCard({
  agent,
  agents,
  primary,
  chosen,
  nowMs,
  onChoose,
}: {
  agent: Agent;
  agents: readonly Agent[];
  primary: boolean;
  chosen: boolean;
  nowMs: number;
  onChoose: () => void;
}) {
  const id = useId();
  return (
    <button
      aria-checked={chosen}
      aria-describedby={`${id}-more`}
      aria-labelledby={id}
      className={styles.card}
      onClick={onChoose}
      role="radio"
      type="button"
    >
      <span className={styles.cardHead}>
        <AssistantAvatar color={colorOf(agent, agents)} name={agent.name} size={36} />
        <span className={styles.cardName}>
          <span className={styles.cardTitle}>
            <span id={id}>{agent.name}</span>
            {primary && <span className={styles.pill}>{copy.principal}</span>}
          </span>
          <span className={styles.cardService}>{copy.serviceCard(agent.service)}</span>
        </span>
      </span>
      <span className={styles.cardMore} id={`${id}-more`}>
        {agent.description !== undefined && agent.description !== "" && (
          <span className={styles.cardText}>{agent.description}</span>
        )}
        <StatusLine agent={agent} nowMs={nowMs} />
      </span>
    </button>
  );
}

/**
 * «¿Con quién quieres hablar?» (annex 32): a card per assistant, the one
 * chosen marked; the choice changes until the first message is sent.
 */
export function ChooseAssistant({
  snapshot,
  port,
  send,
  nowMs,
}: {
  snapshot: AppSnapshot;
  port?: AgentPort;
  send: Send;
  nowMs: number;
}) {
  const agents = agentsOf(snapshot);
  const primaryId = snapshot.settings.default_agent_id;
  const current = agentFor(snapshot, snapshot.chat.agent_id);
  const choice = useHeldChoice(current?.id, send);
  const chosen = agents.find((agent) => agent.id === choice.shown) ?? current;
  const primary = agents.find((agent) => agent.id === primaryId);
  const other = agents.find((agent) => agent.id !== primaryId);
  return (
    <div className={styles.choose}>
      <h1 className={styles.chooseTitle}>{copy.chooseTitle}</h1>
      <p className={styles.chooseLead}>{copy.chooseLead}</p>
      <div aria-label={copy.talkingTo} className={styles.cards} role="radiogroup">
        {agents.map((agent) => (
          <AssistantCard
            agent={agent}
            agents={agents}
            chosen={agent.id === chosen?.id}
            key={agent.id}
            nowMs={nowMs}
            onChoose={() => choice.choose(agent.id, { command: "new_session", agent_id: agent.id })}
            primary={agent.id === primaryId}
          />
        ))}
      </div>
      <p className={styles.chooseWake}>{copy.chooseWake(primary?.name, other?.name)}</p>
      {chosen !== undefined && (
        <div className={styles.homeNotice}>
          <SignInNotice
            agent={chosen}
            agents={agents}
            key={chosen.id}
            nowMs={nowMs}
            port={port}
            send={send}
          />
        </div>
      )}
    </div>
  );
}
