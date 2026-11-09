import { getCopy } from "../../i18n";
import { useCopy } from "../../i18n/CopyProvider";
import type { AppCommand } from "../../bridge/generated";
import type { ChatMessage } from "../../bridge/generated/ChatMessage";
import { Button } from "../../design/Button";
import { Skeleton } from "../../design/Skeleton";

import type { AppView as AppSnapshot } from "../../state/app-view";
import styles from "./AssistantView.module.css";

type Send = (command: AppCommand) => unknown;
type Copy = ReturnType<typeof useCopy>;

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
  const entry = getCopy().extras.noticesForAssistant(es.chat.roles.assistant)[notice.code];
  const text = "text" in entry ? entry.text?.(notice.data ?? {}) : undefined;
  return text ?? es.assistant.turnFailed;
}

/** The cause of the last turn's failure (`agent_limit_reached`…), as the hub named it. */
export function turnFailureCode(snapshot: AppSnapshot): string | undefined {
  const code = snapshot.notice?.data?.code;
  return typeof code === "string" ? code : undefined;
}

/** «Deimos está pensando…», with its three dots. */
export function ActivityLine({
  activity,
  pending,
}: {
  activity: AppSnapshot["chat"]["activity"];
  pending?: AppSnapshot["chat"]["pending"];
}) {
  const es = useCopy();
  if (!activity) return null;
  const labels = {
    transcribing: es.assistant.transcribingVoice,
    thinking: es.assistant.thinking,
    responding: es.assistant.responding,
    retrying_openai: es.assistant.retryingOpenai,
    retrying_anthropic: es.assistant.retryingAnthropic,
  };
  return (
    <p className={styles.activity} role="status">
      <span aria-hidden="true" className={styles.dots}>
        <i />
        <i />
        <i />
      </span>
      {pending
        ? pending.kind === "approval"
          ? es.assistant.awaitingPermission
          : es.assistant.awaitingAnswer
        : labels[activity]}
    </p>
  );
}

/** The shape of a conversation while its page comes from the hub, and what it reads. */
export function ThreadSkeleton() {
  const waits = useCopy().extras.waits;
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
  const es = useCopy();
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
