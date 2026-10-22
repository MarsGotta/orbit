import { useId, useState } from "react";
import type { AppCommand } from "../../bridge/generated";
import { Button } from "../../design/Button";
import { Icon } from "../../design/Icon";
import { useModalFocus } from "../../design/use-modal-focus";
import { useSpanish } from "../../i18n/AssistantCopy";
import { MarkdownEditor } from "../editor/MarkdownEditor";
import { useCopy, copyStatus } from "./CopyButton";
import styles from "./AnswerEditor.module.css";

interface AnswerEditorProps {
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
  send,
  text,
}: {
  onClose: () => void;
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
              if (accepted !== false) onClose();
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
export default function AnswerEditor({ markdown, onClose, send }: AnswerEditorProps) {
  const es = useSpanish();
  const panel = useModalFocus(true);
  const titleId = useId();
  const [text, setText] = useState(markdown);
  return (
    <div
      className={styles.scrim}
      onMouseDown={(event) => {
        if (event.target === event.currentTarget) onClose();
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
          onClose();
        }}
        ref={panel}
        role="dialog"
      >
        <EditorHeader onClose={onClose} titleId={titleId} />
        <div className={styles.body}>
          <MarkdownEditor
            autoFocus
            label={es.chatActions.editorTextLabel}
            onChange={setText}
            status={es.editor.onlyHere}
            value={markdown}
          />
        </div>
        <EditorFooter onClose={onClose} send={send} text={text} />
      </section>
    </div>
  );
}
