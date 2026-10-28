import { useCopy } from "../../i18n/CopyProvider";
import { useId } from "react";
import type { AppCommand } from "../../bridge/generated";
import type { Agent } from "../../bridge/generated/Agent";
import type { ChatSessionSummary } from "../../bridge/generated/ChatSessionSummary";
import type { DaemonPort } from "../../bridge/port";
import { AssistantAvatar } from "../../design/AssistantAvatar";
import { Icon } from "../../design/Icon";
import { Skeleton } from "../../design/Skeleton";

import type { AppView as AppSnapshot } from "../../state/app-view";
import type { AgentPort } from "../settings/agents/use-agent-config";
import { assistantHash } from "./assistant-route";
import { colorOf, statusLine } from "./assistants";
import { agentsOf } from "./agents";
import { insertIntoComposer } from "./composer-insert";
import { useFirstLines } from "./first-lines";
import { sessionWhen } from "./session-title";
import { SignInNotice } from "./SignInNotice";
import { useAssistantSessions } from "./useAssistantSessions";
import styles from "./AssistantPages.module.css";

type Send = (command: AppCommand) => unknown;

// «Seguir donde lo dejaste» shows the assistant's last few, as the annex draws them.
const EARLIER = 3;

/** Whose a listed conversation is: its own agent, or the primary for one without. */
export function sessionAgent(session: ChatSessionSummary, primaryId?: string): string | undefined {
  return session.agent_id ?? primaryId;
}

/** Opens a conversation with its agent, and names it in the route. */
export function openSession(session: ChatSessionSummary, send: Send): void {
  void send({
    command: "session_open",
    id: session.id,
    ...(session.agent_id !== undefined ? { agent_id: session.agent_id } : {}),
  });
}

/** Its status under its name, with the dot of its tone. */
export function StatusLine({ agent, nowMs }: { agent: Agent; nowMs: number }) {
  const status = statusLine(agent, nowMs);
  if (status === null) return null;
  return (
    <span className={styles.status} data-tone={status.tone}>
      <span aria-hidden="true" className={styles.statusDot} />
      {status.text}
    </span>
  );
}

/** «Seguir donde lo dejaste»: its last conversations, each opened with it. */
function Earlier({
  agent,
  primaryId,
  read,
  send,
}: {
  agent: Agent;
  primaryId?: string;
  read?: DaemonPort["readSessionsPage"];
  send: Send;
}) {
  const copy = useCopy().extras.assistantsCopy;
  const es = useCopy();
  const titleId = useId();
  const lines = useFirstLines();
  const { state } = useAssistantSessions(undefined, false, read);
  const now = Date.now();
  const mine = state.items
    .filter((session) => sessionAgent(session, primaryId) === agent.id)
    .slice(0, EARLIER);
  return (
    <section className={styles.homeSection}>
      <h2 className={styles.homeHeading} id={titleId}>
        {copy.continueWhere}
      </h2>
      {!state.loaded ? (
        <div aria-busy="true" className={styles.earlier}>
          <span className={styles.visuallyHidden} role="status">
            {copy.loadingEarlier}
          </span>
          <Skeleton height={12} widths={["60%", "44%"]} />
        </div>
      ) : mine.length === 0 ? (
        <p className={styles.earlierEmpty}>{state.error ?? copy.noEarlier(agent.name)}</p>
      ) : (
        <ul aria-labelledby={titleId} className={styles.earlier}>
          {mine.map((session) => (
            <li key={session.id}>
              <a
                className={styles.earlierRow}
                href={assistantHash({ screen: "conversation", sessionId: session.id })}
                onClick={() => openSession(session, send)}
              >
                <Icon name="conversation" size={15} />
                <span className={styles.earlierLine}>
                  {lines[session.id] ?? es.assistant.messageCount(session.message_count)}
                </span>
                <span className={styles.earlierWhen}>
                  {sessionWhen(session.created_at_ms, now)}
                </span>
              </a>
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}

/**
 * An assistant's own conversation before its first message (annex 33–36):
 * who it is and how it is, its session when it needs one, «Para empezar»
 * (three ideas that fill the box) and «Seguir donde lo dejaste».
 */
export function AssistantHome({
  agent,
  snapshot,
  port,
  readSessions,
  send,
  nowMs,
}: {
  agent: Agent;
  snapshot: AppSnapshot;
  port?: AgentPort;
  readSessions?: DaemonPort["readSessionsPage"];
  send: Send;
  nowMs: number;
}) {
  const copy = useCopy().extras.assistantsCopy;
  const agents = agentsOf(snapshot);
  const startersId = useId();
  const primaryId = snapshot.settings.default_agent_id;
  return (
    <div className={styles.home}>
      <div className={styles.homeHead}>
        <AssistantAvatar color={colorOf(agent, agents)} name={agent.name} size={56} />
        <div className={styles.homeName}>
          <div className={styles.homeTitle}>
            <h1>{agent.name}</h1>
            {agent.id === primaryId && agents.length > 1 && (
              <span className={styles.pill}>{copy.primaryBadge}</span>
            )}
          </div>
          {agent.description !== undefined && agent.description !== "" && (
            <p>{agent.description}</p>
          )}
        </div>
      </div>
      <StatusLine agent={agent} nowMs={nowMs} />
      <div className={styles.homeNotice}>
        <SignInNotice agent={agent} agents={agents} nowMs={nowMs} port={port} send={send} />
      </div>
      <section className={styles.homeSection}>
        <h2 className={styles.homeHeading} id={startersId}>
          {copy.starters}
        </h2>
        <div aria-labelledby={startersId} className={styles.starters} role="group">
          {copy.startersFor(agent.service).map((idea) => (
            <button
              className={styles.starter}
              key={idea}
              onClick={() => insertIntoComposer(idea)}
              type="button"
            >
              {idea}
            </button>
          ))}
        </div>
      </section>
      <Earlier agent={agent} primaryId={primaryId} read={readSessions} send={send} />
    </div>
  );
}
