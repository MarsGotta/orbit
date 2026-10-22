import { useId, useState } from "react";
import type { AppCommand } from "../../bridge/generated";
import { Button } from "../../design/Button";
import { Icon } from "../../design/Icon";
import { useModalFocus } from "../../design/use-modal-focus";
import { useSpanish } from "../../i18n/AssistantCopy";
import { asTextOver, useLocalDraft } from "../../state/local-drafts";
import { MarkdownEditor } from "../editor/MarkdownEditor";
import { useCopy, copyStatus } from "./CopyButton";
import styles from "./AnswerEditor.module.css";

interface AnswerEditorProps {
  /** The answer's message: its changes are kept on this Mac under it. */
  messageId: string;
  markdown: string;
  onClose: () => void;
  send: (command: AppCommand) => unknown;
}

/** «Editar la respuesta», what it is for, and ×. */
function EditorHeader({ onClose, titleId }: { onClose: () => void; titleId: string }) {
  const es = useSpanish();
  return (
    <header className={styles.header}>
      <h2 id={titleId}>{es.chatActions.editorTitle}</h2>
      <span id={`${titleId}-sub`}>{es.assistant.editorSubtitle}</span>
      <button
        aria-label={es.chatActions.closeEditor}
        className={styles.x}
        onClick={onClose}
        type="button"
      >
        <Icon name="close" size={16} />
      </button>
    </header>
  );
}

/**
 * The count of characters, «Cerrar», «Copiar» (→ «✓ Copiado») and «Mandar
 * a {agente}». The window closes once the message was taken: refused, it
 * stays open with what was written, and the view says why.
 */
function EditorFooter({
  onClose,
  onSent,
  send,
  text,
}: {
  onClose: () => void;
  /** The message was taken: what was written goes with it. */
  onSent: () => void;
  send: AnswerEditorProps["send"];
  text: string;
}) {
  const es = useSpanish();
  const { copy, state } = useCopy();
  const [sending, setSending] = useState(false);
  const failure = copyStatus(state);
  return (
    <footer className={styles.footer}>
      <span className={styles.count}>{es.chatActions.charactersCopied(text.length)}</span>
      {failure !== null && (
        <span className={styles.failure} role="alert">
          {failure}
        </span>
      )}
      <Button onClick={onClose} size={36} variant="ghost">
        {es.chatActions.close}
      </Button>
      <Button
        className={state.kind === "copied" ? styles.copied : undefined}
        onClick={() => void copy(text)}
        size={36}
      >
        <span aria-live="polite">
          {state.kind === "copied" ? es.chatActions.copied : es.chatActions.copy}
        </span>
      </Button>
      <Button
        disabled={text.trim() === "" || sending}
        onClick={() => {
          const message = text.trim();
          if (message === "") return;
          setSending(true);
          void Promise.resolve(send({ command: "chat", text: message }))
            .then((accepted) => {
              if (accepted !== false) onSent();
            })
            .catch(() => undefined)
            .finally(() => setSending(false));
        }}
        size={36}
        variant="accent"
      >
        {es.chatActions.sendBack}
      </Button>
    </footer>
  );
}

/**
 * An answer opened to change it, in the notes' Markdown editor: copy it,
 * or send it to the agent as a new message. Closing it (×, «Cerrar», Esc
 * or outside) sends nothing.
 */
/**
 * The answer as changed, kept on this Mac over the answer it was changed
 * from, so a quit or a fall with the window open loses nothing; closing the
 * window, or sending, lets it go.
 */
function useKeptAnswer(messageId: string, markdown: string) {
  const kept = useLocalDraft(`answer.${messageId}`, null, asTextOver);
  const [opened] = useState(() => (kept.value?.from === markdown ? kept.value.text : markdown));
  const [recovered] = useState(() => kept.recovered && opened !== markdown);
  return {
    opened,
    recovered,
    change: (text: string) => kept.set(text === markdown ? null : { from: markdown, text }),
    drop: () => kept.set(null),
  };
}

export default function AnswerEditor({ markdown, messageId, onClose, send }: AnswerEditorProps) {
  const es = useSpanish();
  const panel = useModalFocus(true);
  const titleId = useId();
  const kept = useKeptAnswer(messageId, markdown);
  const [text, setText] = useState(kept.opened);
  const close = () => {
    kept.drop();
    onClose();
  };
  return (
    <div
      className={styles.scrim}
      onMouseDown={(event) => {
        if (event.target === event.currentTarget) close();
      }}
    >
      <section
        aria-describedby={`${titleId}-sub`}
        aria-labelledby={titleId}
        aria-modal="true"
        className={styles.dialog}
        onKeyDown={(event) => {
          if (event.key !== "Escape") return;
          event.stopPropagation();
          close();
        }}
        ref={panel}
        role="dialog"
      >
        <EditorHeader onClose={close} titleId={titleId} />
        <div className={styles.body}>
          <MarkdownEditor
            autoFocus
            label={es.chatActions.editorTextLabel}
            onChange={(next) => {
              setText(next);
              kept.change(next);
            }}
            status={kept.recovered ? es.controls.draftRecovered : es.editor.onlyHere}
            value={kept.opened}
          />
        </div>
        <EditorFooter onClose={close} onSent={close} send={send} text={text} />
      </section>
    </div>
  );
}
