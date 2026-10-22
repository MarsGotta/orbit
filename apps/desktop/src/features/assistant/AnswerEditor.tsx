import { useId, useState } from "react";
import type { AppCommand } from "../../bridge/generated";
import { Button } from "../../design/Button";
import { Icon } from "../../design/Icon";
import { useModalFocus } from "../../design/use-modal-focus";
import { useSpanish } from "../../i18n/AssistantCopy";
import { ChatMarkdown } from "./ChatMarkdown";
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

/** The count of characters, «Cerrar», «Copiar» (→ «✓ Copiado») and «Mandar a Deimos». */
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
  const failure = copyStatus(state);
  return (
    <footer className={styles.footer}>
      <span className={styles.count}>{es.chatActions.characters(text.length)}</span>
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
        disabled={text.trim() === ""}
        onClick={() => {
          const message = text.trim();
          if (message === "") return;
          send({ command: "chat", text: message });
          onClose();
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
 * An answer opened to change it: its Markdown on the left and, on the
 * right, how it will look, at once. Then copy it, or send it to the agent
 * as a new message. Closing it (×, «Cerrar», Esc or outside) sends nothing.
 */
export default function AnswerEditor({ markdown, onClose, send }: AnswerEditorProps) {
  const es = useSpanish();
  const panel = useModalFocus(true);
  const titleId = useId();
  const textId = useId();
  const previewId = useId();
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
        <div className={styles.columns}>
          <div className={styles.column}>
            <span aria-hidden="true" className={styles.label}>
              {es.chatActions.editorText}
            </span>
            <label className={styles.hidden} htmlFor={textId}>
              {es.chatActions.editorTextLabel}
            </label>
            <textarea
              className={styles.text}
              id={textId}
              onChange={(event) => setText(event.target.value)}
              spellCheck={false}
              value={text}
            />
          </div>
          <section aria-labelledby={previewId} className={styles.preview}>
            <h3 className={styles.label} id={previewId}>
              {es.chatActions.editorPreview}
            </h3>
            <div className={styles.rendered}>
              <ChatMarkdown source={text} />
            </div>
          </section>
        </div>
        <EditorFooter onClose={onClose} send={send} text={text} />
      </section>
    </div>
  );
}
