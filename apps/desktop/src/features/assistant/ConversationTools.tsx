import { useEffect, useRef, useState } from "react";
import type { ChatMessage } from "../../bridge/generated/ChatMessage";
import type { DaemonPort } from "../../bridge/port";
import type { AssistantCursor } from "../../bridge/reads";
import { Button } from "../../design/Button";
import { useSpanish } from "../../i18n/AssistantCopy";
import { copyText } from "../../bridge/clipboard";
import { conversationView, withOlder } from "./conversation";
import { conversationMarkdown, lastCodeBlock } from "./conversation-markdown";
import styles from "./MessageActions.module.css";

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

interface ToolsProps {
  /** What the thread shows now: ⌘⇧C copies from here. */
  messages: ChatMessage[];
  read?: Reader;
  sessionId: string | null;
  turn: ChatMessage[];
  inFlight: boolean;
}

/** «Copiar conversación» and ⌘⇧C for the last code block, with what each did. */
export function ConversationTools({ inFlight, messages, read, sessionId, turn }: ToolsProps) {
  const es = useSpanish();
  const [status, setStatus] = useState<string | null>(null);
  const names = {
    user: es.chatActions.you,
    assistant: es.chat.roles.assistant,
    voiceNote: es.chatActions.voiceNoteSuffix,
  };
  const copy = (text: string, done: string) =>
    copyText(text).then(
      () => setStatus(done),
      (error: unknown) => setStatus(es.chatActions.copyFailed(String(error))),
    );

  // ⌘⇧C reads the thread as it is drawn now. The handler is renewed on each
  // render and the listener stays, so a press right after an answer appears
  // never meets the listener of the render before it.
  const copyLastCode = useRef<() => void>(() => undefined);
  copyLastCode.current = () => {
    const code = lastCodeBlock(messages);
    if (code === null) setStatus(es.chatActions.noCodeBlock);
    else void copy(code, es.chatActions.lastCodeCopied);
  };
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (!event.metaKey || !event.shiftKey || event.code !== "KeyC") return;
      event.preventDefault();
      copyLastCode.current();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  const copyConversation = async () => {
    const stored =
      read && sessionId !== null
        ? await readAll(read, sessionId).catch(() => null)
        : { messages: [], total: 0, complete: true };
    if (stored === null) {
      setStatus(es.assistant.historyFailed);
      return;
    }
    // Without any page of the hub there is no conversation to copy, only its turn.
    if (!stored.complete && stored.messages.length === 0) {
      setStatus(es.assistant.historyFailed);
      return;
    }
    const all = conversationView(stored.messages, turn, inFlight);
    await copy(
      conversationMarkdown(all, names),
      stored.complete
        ? es.chatActions.conversationCopied
        : es.chatActions.conversationPartial(stored.messages.length, stored.total),
    );
  };

  if (messages.length === 0) return null;
  return (
    <div className={styles.tools}>
      {status !== null && <span role="status">{status}</span>}
      <Button onClick={() => void copyConversation()} size={28} variant="ghost">
        {es.chatActions.copyConversation}
      </Button>
    </div>
  );
}
