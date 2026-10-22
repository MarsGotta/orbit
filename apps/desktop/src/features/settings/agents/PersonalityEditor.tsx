import { EditorContent, useEditor } from "@tiptap/react";
import { useMemo, useRef } from "react";
import { markdownExtensions } from "../../assistant/markdown-extensions";
import editorStyles from "../../notes/NoteEditor.module.css";

interface PersonalityEditorProps {
  /** Accessible name of the text box. */
  label: string;
  markdown: string;
  /** `false` when the agent's kind does not let Orbit change it. */
  editable: boolean;
  /** The Markdown as the user left it; called only when they edit. */
  onChange: (markdown: string) => void;
}

/**
 * An agent's personality in the Markdown editor answers open in: its code
 * blocks, quotes and tables stay what they are. What is sent is the editor's
 * Markdown, and only once the user has changed something.
 */
export default function PersonalityEditor({
  editable,
  label,
  markdown,
  onChange,
}: PersonalityEditorProps) {
  const onChangeRef = useRef(onChange);
  onChangeRef.current = onChange;
  const options = useMemo(
    () => ({
      content: markdown,
      contentType: "markdown" as const,
      editable,
      extensions: markdownExtensions(),
      editorProps: {
        attributes: {
          "aria-label": label,
          "aria-multiline": "true",
          "aria-readonly": editable ? "false" : "true",
          class: editorStyles.editor ?? "",
          role: "textbox",
        },
      },
      onUpdate: ({ editor }: { editor: { getMarkdown: () => string } }) =>
        onChangeRef.current(editor.getMarkdown()),
    }),
    [editable, label, markdown],
  );
  const editor = useEditor(options);
  return <EditorContent className={editorStyles.frame} editor={editor} />;
}
