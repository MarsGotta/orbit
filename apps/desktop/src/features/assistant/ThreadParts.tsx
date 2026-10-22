import type { AppCommand } from "../../bridge/generated";
import type { ChatMessage } from "../../bridge/generated/ChatMessage";
import { Button } from "../../design/Button";
import { EmptyState } from "../../design/EmptyState";
import { Segmented } from "../../design/Segmented";
import { Skeleton } from "../../design/Skeleton";
import { useSpanish } from "../../i18n/AssistantCopy";
import { noticesForAssistant } from "../../i18n/es/notices";
import { waits } from "../../i18n/es/waits";
import type { AppView as AppSnapshot } from "../../state/app-view";
import { useHeldChoice } from "../../state/use-held-choice";
import { agentFor, agentsOf } from "./agents";
import styles from "./AssistantView.module.css";

type Send = (command: AppCommand) => unknown;
type Copy = ReturnType<typeof useSpanish>;

// The notices that say a turn ended without its answer, and why.
const TURN_FAILURES = new Set([
  "assistant_failed",
  "assistant_turn_lost_by_restart",
  "assistant_turn_lost_by_disconnect",
  "assistant_outcome_unknown",
]);

/** The question of the turn in progress, or of the last one. */
export function turnQuestion(chat: AppSnapshot["chat"]): ChatMessage | undefined {
  return chat.turn.find((message) => message.role === "user");
}

/**
 * Why the last turn ended without its answer, in the words of its notice:
 * only while its question is the thread's last message and nothing is under
 * way. A cause without words of its own says the agent could not answer.
 */
export function turnFailure(
  snapshot: AppSnapshot,
  messages: ChatMessage[],
  es: Copy,
): string | null {
  const { chat, notice } = snapshot;
  const question = turnQuestion(chat);
  if (!question || chat.pending !== null || chat.activity !== undefined) return null;
  if (chat.turn.some((message) => message.role === "assistant")) return null;
  if (messages.at(-1)?.id !== question.id) return null;
  if (!notice || !TURN_FAILURES.has(notice.code)) return null;
  const entry = noticesForAssistant(es.chat.roles.assistant)[notice.code];
  const text = "text" in entry ? entry.text?.(notice.data ?? {}) : undefined;
  return text ?? es.assistant.turnFailed;
}

/** «Deimos está pensando…», with its three dots. */
export function ActivityLine({ activity }: { activity: AppSnapshot["chat"]["activity"] }) {
  const es = useSpanish();
  if (!activity) return null;
  const labels = {
    transcribing: es.assistant.transcribingVoice,
    thinking: es.assistant.thinking,
    responding: es.assistant.responding,
  };
  return (
    <p className={styles.activity} role="status">
      <span aria-hidden="true" className={styles.dots}>
        <i />
        <i />
        <i />
      </span>
      {labels[activity]}
    </p>
  );
}

/** The shape of a conversation while its page comes from the hub, and what it reads. */
export function ThreadSkeleton() {
  return (
    <div className={styles.threadSkeleton}>
      <span className={styles.visuallyHidden} role="status">
        {waits.readingConversation}
      </span>
      <span className={styles.skeletonBubble}>
        <Skeleton widths={["100%"]} />
      </span>
      <Skeleton widths={["28%", "92%", "86%", "64%"]} />
      <span className={styles.skeletonBubble}>
        <Skeleton widths={["100%"]} />
      </span>
      <Skeleton widths={["24%", "88%", "52%"]} />
    </div>
  );
}

/** A conversation without messages: what to do and, before the first one, with whom. */
export function EmptyConversation({ send, snapshot }: { send: Send; snapshot: AppSnapshot }) {
  const es = useSpanish();
  const agents = agentsOf(snapshot);
  const current = agentFor(snapshot, snapshot.chat.agent_id);
  // The agent chosen stays shown until the new conversation says it.
  const agent = useHeldChoice(current?.id, send);
  const choose = snapshot.chat.session_id === null && agents.length > 1 && current !== undefined;
  return (
    <div className={styles.empty}>
      <EmptyState icon="assistant" title={es.assistant.emptyTitle}>
        {es.assistant.emptyText}
      </EmptyState>
      {choose && (
        <div className={styles.agentChoice}>
          <span aria-hidden="true">{es.assistant.agentPicker}</span>
          <Segmented
            label={es.assistant.agentPicker}
            onChange={(id) => agent.choose(id, { command: "new_session", agent_id: id })}
            options={agents.map((each) => ({ value: each.id, label: each.name }))}
            value={agent.shown ?? current.id}
          />
        </div>
      )}
    </div>
  );
}

/** The failed turn's cause under its question, and «Volver a preguntar». */
export function TurnFailure({
  text,
  retry,
  send,
}: {
  text: string;
  retry: string | null;
  send: Send;
}) {
  const es = useSpanish();
  return (
    <div className={styles.failure} role="alert">
      <p>{text}</p>
      {retry !== null && (
        <Button onClick={() => send({ command: "chat", text: retry })} size={28}>
          {es.assistant.askAgain}
        </Button>
      )}
    </div>
  );
}
