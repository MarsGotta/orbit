import { Suspense, lazy, useEffect, useRef, useState, type RefObject } from "react";
import type { AppCommand } from "../../bridge/generated";
import type { ChatMessage } from "../../bridge/generated/ChatMessage";
import { Button } from "../../design/Button";
import { IconButton } from "../../design/IconButton";
import { es } from "../../i18n/es";
import { ChatMarkdown } from "./ChatMarkdown";
import { insertIntoComposer } from "./composer-insert";
import { quoteFragment } from "./conversation-markdown";
import { CopyButton } from "./CopyButton";
import { spokenText } from "./spoken-text";
import styles from "./MessageActions.module.css";

// The editor (CodeMirror and its Markdown) loads only when an answer is opened in it.
const AnswerEditor = lazy(() => import("./AnswerEditor"));

interface AnswerProps {
  message: ChatMessage;
  send: (command: AppCommand) => unknown;
  /** The user's message «Reintentar» sends again; null when there is none. */
  retry: string | null;
  /** A turn is in flight: retrying waits for it to end. */
  busy: boolean;
  /** The answer being read aloud, by its message id. */
  speaking: string | null;
  /** Some voice can read it: the conversation's agent's or the default one's. */
  listenable: boolean;
}

/** The selected text inside `root`, if the selection lies wholly in it. */
function fragmentIn(root: HTMLElement | null): string | null {
  const selection = window.getSelection();
  if (root === null || selection === null || selection.isCollapsed) return null;
  const range = selection.getRangeAt(0);
  if (!root.contains(range.commonAncestorContainer)) return null;
  const text = selection.toString().trim();
  return text === "" ? null : text;
}

/** «Responder a esto» shows while part of the answer is selected. */
function useFragment(content: RefObject<HTMLDivElement | null>) {
  const [fragment, setFragment] = useState<string | null>(null);
  useEffect(() => {
    if (fragment === null) return;
    const follow = () => setFragment(fragmentIn(content.current));
    document.addEventListener("selectionchange", follow);
    return () => document.removeEventListener("selectionchange", follow);
  }, [content, fragment]);
  return { fragment, observe: () => setFragment(fragmentIn(content.current)), setFragment };
}

/** «Escuchar», which turns to «Parar», in the accent, while the answer sounds. */
function ListenButton({
  id,
  speaking,
  text,
  send,
}: {
  id: string;
  speaking: string | null;
  text: () => string;
  send: AnswerProps["send"];
}) {
  const playing = speaking === id;
  return (
    <IconButton
      active={playing ? { icon: "stop", label: es.chatActions.stopListening } : undefined}
      aria-pressed={playing}
      icon="listen"
      label={es.chatActions.listen}
      onClick={() =>
        send(
          playing ? { command: "speak_stop" } : { command: "speak", message_id: id, text: text() },
        )
      }
      size={28}
    />
  );
}

/** «Responder a esto»: quotes the selected part of the answer in the composer. */
function ReplyToThis({ fragment, onDone }: { fragment: string; onDone: () => void }) {
  return (
    <Button
      onClick={() => {
        insertIntoComposer(quoteFragment(fragment));
        onDone();
      }}
      // Keeps the selection: a press on a button would otherwise clear it.
      onMouseDown={(event) => event.preventDefault()}
      size={28}
      variant="ghost"
    >
      {es.chatActions.replyToThis}
    </Button>
  );
}

/**
 * An answer of the assistant with what can be done with it, as icons with
 * their tooltip: copy its Markdown, hear it, ask again and open it in the
 * editor; and, while part of it is selected, quote that in the composer.
 * None of these sends anything by itself.
 */
export function AssistantAnswer({ busy, listenable, message, retry, send, speaking }: AnswerProps) {
  const content = useRef<HTMLDivElement>(null);
  const { fragment, observe, setFragment } = useFragment(content);
  const [editing, setEditing] = useState(false);
  return (
    <>
      <div onKeyUp={observe} onMouseUp={observe} ref={content}>
        <ChatMarkdown source={message.text} streaming={message.streaming} />
      </div>
      {!message.streaming && (
        <div className={styles.actions}>
          <CopyButton label={es.chatActions.copyAnswer} text={() => message.text} />
          {listenable && (
            <ListenButton
              id={message.id}
              send={send}
              speaking={speaking}
              text={() => (content.current === null ? message.text : spokenText(content.current))}
            />
          )}
          <IconButton
            disabled={busy || retry === null}
            icon="refresh"
            label={es.assistant.askAgain}
            onClick={() => retry !== null && send({ command: "chat", text: retry })}
            size={28}
          />
          <IconButton
            icon="edit"
            label={es.chatActions.openEditor}
            onClick={() => setEditing(true)}
            size={28}
          />
          {fragment !== null && (
            <ReplyToThis fragment={fragment} onDone={() => setFragment(null)} />
          )}
        </div>
      )}
      {editing && (
        <Suspense fallback={<p role="status">{es.chatActions.openingEditor}</p>}>
          <AnswerEditor
            markdown={message.text}
            messageId={message.id}
            onClose={() => setEditing(false)}
            send={send}
          />
        </Suspense>
      )}
    </>
  );
}
