import { useCopy } from "../../i18n/CopyProvider";
import type { ReactNode } from "react";
import type { AppCommand } from "../../bridge/generated";
import type { Agent } from "../../bridge/generated/Agent";
import { TopBarLeading } from "../../app/shell-slots";
import { Button } from "../../design/Button";
import { Icon } from "../../design/Icon";

import type { AppView as AppSnapshot } from "../../state/app-view";
import type { AgentPort } from "../settings/agents/use-agent-config";
import { healthOf } from "./assistants";
import { turnInFlight } from "./conversation";
import { AssistantIdentity } from "./AssistantIdentity";
import { sessionWhen } from "./session-title";
import styles from "./AssistantView.module.css";

type Copy = ReturnType<typeof useCopy>;

/** What the header says the agent is doing, or when the conversation began. */
function headerState(chat: AppSnapshot["chat"], began: number | null, es: Copy): string {
  if (chat.pending?.kind === "approval") return es.assistant.waitsPermission;
  if (chat.pending?.kind === "clarify") return es.assistant.asksSomething;
  if (chat.activity !== undefined) return es.assistant.working;
  if (chat.session_id === null) return es.assistant.newConversationState;
  if (began === null) return "";
  return sessionWhen(began, Date.now()).replace(/^(Hoy|Ayer)/, (word) => word.toLowerCase());
}

/** With whom and what it does; «Conversación nueva · elige con quién hablar» while choosing. */
function HeaderTitle({
  agent,
  began,
  choosing,
  handedFrom,
  port,
  snapshot,
}: {
  agent?: Agent;
  began: number | null;
  choosing: boolean;
  handedFrom?: Agent;
  port?: AgentPort;
  snapshot: AppSnapshot;
}) {
  const copy = useCopy().extras.assistantsCopy;
  const es = useCopy();
  const { chat } = snapshot;
  const health = agent === undefined ? null : healthOf(agent, Date.now());
  const limit = health?.kind === "limit" ? health : null;
  const state =
    limit !== null
      ? copy.limitState(limit.until)
      : handedFrom !== undefined && chat.activity === undefined && chat.pending === null
        ? copy.continuesFrom(handedFrom.name)
        : headerState(chat, began, es);
  if (choosing)
    return (
      <span className={styles.identity}>
        <h2>{copy.newConversation}</h2>
        <span className={styles.headerState}>{copy.newConversationHint}</span>
      </span>
    );
  if (agent === undefined)
    return (
      <span className={styles.identity}>
        <h2>{es.chat.roles.assistant}</h2>
        <span className={styles.headerState}>{state}</span>
      </span>
    );
  return (
    <AssistantIdentity
      agent={agent}
      port={port}
      snapshot={snapshot}
      state={state}
      tone={limit !== null ? "warn" : undefined}
    />
  );
}

/**
 * The conversation's header, in the window's top bar (one bar, not two):
 * with whom and what it does, «Interrumpir» while it answers, «Llamar» and,
 * as `children`, «Copiar conversación». «Conversación nueva · elige con
 * quién hablar» while the assistant is being chosen.
 */
export function ConversationHeader({
  agent,
  began,
  calling,
  canCall,
  children,
  choosing = false,
  handedFrom,
  onCall,
  port,
  send,
  snapshot,
}: {
  /** Whom it is with; absent before the hub says its agents. */
  agent?: Agent;
  /** When the conversation began, once its start has been read. */
  began: number | null;
  calling: boolean;
  canCall: boolean;
  children?: ReactNode;
  /** «¿Con quién quieres hablar?» is open: it says so instead of a name. */
  choosing?: boolean;
  /** The assistant this conversation continues from, when it was handed over. */
  handedFrom?: Agent;
  onCall: () => void;
  port?: AgentPort;
  send: (command: AppCommand) => unknown;
  snapshot: AppSnapshot;
}) {
  const es = useCopy();
  const { chat } = snapshot;
  return (
    <TopBarLeading>
      <div className={styles.header} data-conversation-header="">
        <HeaderTitle
          agent={agent}
          began={began}
          choosing={choosing}
          handedFrom={handedFrom}
          port={port}
          snapshot={snapshot}
        />
        <span className={styles.spacer} />
        {turnInFlight(chat) && chat.pending === null && (
          <Button onClick={() => send({ command: "interrupt" })} size={32}>
            {es.chat.interrupt}
          </Button>
        )}
        <Button
          aria-label={calling ? es.design.inCall : es.design.callAssistant}
          disabled={!calling && !canCall}
          onClick={onCall}
          size={32}
          variant="icon"
        >
          <Icon name="phone" size={16} />
        </Button>
        {children}
      </div>
    </TopBarLeading>
  );
}
