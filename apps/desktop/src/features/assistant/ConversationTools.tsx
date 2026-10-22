import { useEffect, useRef, useState } from "react";
import type { ChatMessage } from "../../bridge/generated/ChatMessage";
import type { DaemonPort } from "../../bridge/port";
import type { AssistantCursor } from "../../bridge/reads";
import { Button } from "../../design/Button";
import { useSpanish } from "../../i18n/AssistantCopy";
import { copyText } from "../../bridge/clipboard";
import { conversationView, withOlder } from "./conversation";
import { conversationMarkdown, lastCodeBlock } from "./conversation-markdown";
import styles from "./AssistantView.module.css";

type Reader = NonNullable<DaemonPort["readSessionMessages"]>;

// The hub pages a conversation 200 messages at a time; 100 reads is 20 000
// messages. The bound only stops a cursor that never ends from reading
// forever; a longer conversation is copied as far as it got, and says so.
const MAX_COPY_READS = 100;

/** The whole conversation as the hub keeps it, newest page first, then older ones. */
async function readAll(read: Reader, sessionId: string) {
  let messages: ChatMessage[] = [];
  let cursor: AssistantCursor | null = null;
  let total = 0;
  for (let reads = 0; reads < MAX_COPY_READS; reads += 1) {
    const answer = await read(sessionId, cursor);
    if (answer.state !== "present") break;
    messages = withOlder(messages, answer.value.items);
    total = Math.max(total, answer.value.total);
    cursor = answer.value.next;
    if (cursor === null) return { messages, total, complete: true };
  }
  return { messages, total, complete: false };
}

/** The conversation as the hub keeps it; null when it could not be read. */
function storedConversation(read: Reader | undefined, sessionId: string | null) {
  if (!read || sessionId === null)
    return Promise.resolve({ messages: [] as ChatMessage[], total: 0, complete: true });
  return readAll(read, sessionId).catch(() => null);
}

/**
 * ⌘⇧C, wherever the focus is. The handler is renewed on each render and the
 * listener stays, so a press right after an answer appears never meets the
 * listener of the render before it.
 */
function useLastCodeShortcut(copyLastCode: () => void) {
  const latest = useRef(copyLastCode);
  latest.current = copyLastCode;
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (!event.metaKey || !event.shiftKey || event.code !== "KeyC") return;
      event.preventDefault();
      latest.current();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);
}

interface ToolsProps {
  /** What the thread shows now: ⌘⇧C copies from here. */
  messages: ChatMessage[];
  read?: Reader;
  sessionId: string | null;
  turn: ChatMessage[];
  inFlight: boolean;
}

// How long «✓ Copiada» and what ⌘⇧C did stay before the header is quiet again.
const SAID_MS = 1_800;

/**
 * «Copiar conversación» in the header, which says «✓ Copiada» (or how much
 * the hub gave, or why not), and ⌘⇧C for the last code block.
 */
export function ConversationTools({ inFlight, messages, read, sessionId, turn }: ToolsProps) {
  const es = useSpanish();
  const [status, setStatus] = useState<{ text: string; done: boolean } | null>(null);
  useEffect(() => {
    if (status === null || !status.done) return;
    const timer = window.setTimeout(() => setStatus(null), SAID_MS);
    return () => window.clearTimeout(timer);
  }, [status]);
  const copy = (text: string, done: string, complete = true) =>
    copyText(text).then(
      () => setStatus({ text: done, done: complete }),
      (error: unknown) =>
        setStatus({ text: es.chatActions.copyFailed(String(error)), done: false }),
    );

  useLastCodeShortcut(() => {
    const code = lastCodeBlock(messages);
    if (code === null) setStatus({ text: es.chatActions.noCodeBlock, done: true });
    else void copy(code, es.chatActions.lastCodeCopied);
  });

  const copyConversation = async () => {
    const stored = await storedConversation(read, sessionId);
    // Without any page of the hub there is no conversation to copy, only its turn.
    if (stored === null || (!stored.complete && stored.messages.length === 0)) {
      setStatus({ text: es.assistant.historyFailed, done: false });
      return;
    }
    await copy(
      conversationMarkdown(conversationView(stored.messages, turn, inFlight), {
        user: es.chatActions.you,
        assistant: es.chat.roles.assistant,
        voiceNote: es.chatActions.voiceNoteSuffix,
      }),
      stored.complete
        ? es.chatActions.copiedConversation
        : es.chatActions.conversationPartial(stored.messages.length, stored.total),
      stored.complete,
    );
  };

  const copied = status?.text === es.chatActions.copiedConversation;
  if (messages.length === 0) return null;
  return (
    <>
      {status !== null && !copied && (
        <span className={styles.status} role="status">
          {status.text}
        </span>
      )}
      <Button
        aria-label={es.chatActions.copyConversation}
        className={copied ? styles.copied : styles.copyAll}
        onClick={() => void copyConversation()}
        size={32}
        variant="ghost"
      >
        <span aria-live="polite">
          {copied ? es.chatActions.copiedConversation : es.chatActions.copyConversation}
        </span>
      </Button>
    </>
  );
}
