import { EditorContent, useEditor } from "@tiptap/react";
import { useMemo } from "react";
import type { AppCommand } from "../../bridge/generated";
import { Button } from "../../design/Button";
import { useModalFocus } from "../../design/use-modal-focus";
import { es } from "../../i18n/es";
import { useSpanish } from "../../i18n/AssistantCopy";
import editorStyles from "../notes/NoteEditor.module.css";
import { useCopy, copyStatus } from "./CopyButton";
import styles from "./AnswerEditor.module.css";
import { markdownExtensions } from "./markdown-extensions";

interface AnswerEditorProps {
  markdown: string;
  onClose: () => void;
  send: (command: AppCommand) => void;
}

function EditorActions({
  current,
  onClose,
  send,
}: {
  current: () => string;
  onClose: () => void;
  send: AnswerEditorProps["send"];
}) {
  const { copy, state } = useCopy();
  const failure = copyStatus(state);
  const sendBack = useSpanish().chatActions.sendBack;
  return (
    <>
      {failure !== null && <p role="alert">{failure}</p>}
      <footer className={styles.actions}>
        <Button onClick={onClose} variant="ghost">
          {es.chatActions.close}
        </Button>
        <Button onClick={() => void copy(current())}>
          {state.kind === "copied" ? es.chatActions.copied : es.chatActions.copy}
        </Button>
        <Button
          onClick={() => {
            const text = current().trim();
            if (text === "") return;
            send({ command: "chat", text });
            onClose();
          }}
          variant="accent"
        >
          {sendBack}
        </Button>
      </footer>
    </>
  );
}

/**
 * An answer opened in the Markdown editor, to change it and then copy it or
 * send it back to the assistant as a new message. Closing it sends nothing.
 */
export default function AnswerEditor({ markdown, onClose, send }: AnswerEditorProps) {
  const panel = useModalFocus(true);
  const options = useMemo(
    () => ({
      content: markdown,
      contentType: "markdown" as const,
      extensions: markdownExtensions(),
      editorProps: {
        attributes: {
          "aria-label": es.chatActions.editorLabel,
          "aria-multiline": "true",
          class: editorStyles.editor ?? "",
          role: "textbox",
        },
      },
    }),
    [markdown],
  );
  const editor = useEditor(options);
  return (
    <div className={styles.backdrop} role="presentation">
      <section
        aria-label={es.chatActions.editorTitle}
        aria-modal="true"
        className={styles.dialog}
        onKeyDown={(event) => {
          if (event.key === "Escape") {
            event.stopPropagation();
            onClose();
          }
        }}
        ref={panel}
        role="dialog"
      >
        <h2>{es.chatActions.editorTitle}</h2>
        <EditorContent
          className={`${editorStyles.frame ?? ""} ${styles.frame ?? ""}`}
          editor={editor}
        />
        <EditorActions current={() => editor.getMarkdown()} onClose={onClose} send={send} />
      </section>
    </div>
  );
}
