import type { ReactNode } from "react";
import type { AppCommand } from "../../bridge/generated";
import { Button } from "../../design/Button";
import { Icon } from "../../design/Icon";
import { useSpanish } from "../../i18n/AssistantCopy";
import type { AppView as AppSnapshot } from "../../state/app-view";
import { turnInFlight } from "./conversation";
import { sessionWhen } from "./session-title";
import styles from "./AssistantView.module.css";

type Copy = ReturnType<typeof useSpanish>;

/** What the header says the agent is doing, or when the conversation began. */
function headerState(chat: AppSnapshot["chat"], began: number | null, es: Copy): string {
  if (chat.pending?.kind === "approval") return es.assistant.waitsPermission;
  if (chat.pending?.kind === "clarify") return es.assistant.asksSomething;
  if (chat.activity !== undefined) return es.assistant.working;
  if (chat.session_id === null) return es.assistant.newConversationState;
  if (began === null) return "";
  return sessionWhen(began, Date.now()).replace(/^(Hoy|Ayer)/, (word) => word.toLowerCase());
}

/**
 * The conversation's header, fixed above the thread: the agent's name and
 * what it is doing («espera tu permiso»), «Interrumpir» while it answers,
 * «Llamar» and, as `children`, «Copiar conversación».
 */
export function ConversationHeader({
  began,
  calling,
  canCall,
  children,
  onCall,
  send,
  snapshot,
}: {
  /** When the conversation began, once its start has been read. */
  began: number | null;
  calling: boolean;
  canCall: boolean;
  children?: ReactNode;
  onCall: () => void;
  send: (command: AppCommand) => unknown;
  snapshot: AppSnapshot;
}) {
  const es = useSpanish();
  const { chat } = snapshot;
  return (
    <header className={styles.header}>
      <h2>{es.chat.roles.assistant}</h2>
      <span className={styles.headerState}>{headerState(chat, began, es)}</span>
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
    </header>
  );
}
