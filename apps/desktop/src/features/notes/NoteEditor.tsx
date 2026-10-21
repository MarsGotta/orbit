import Link from "@tiptap/extension-link";
import Placeholder from "@tiptap/extension-placeholder";
import TaskItem from "@tiptap/extension-task-item";
import TaskList from "@tiptap/extension-task-list";
import { EditorContent, Extension, useEditor } from "@tiptap/react";
import StarterKit from "@tiptap/starter-kit";
import { useEffect, useMemo, useRef, useState } from "react";
import { safeHref } from "./markdown";
import { docToMarkdown, markdownToDoc } from "./markdown-doc";
import styles from "./NoteEditor.module.css";

interface NoteEditorProps {
  /** Accessible name of the text box. */
  label: string;
  markdown: string;
  onChange: (markdown: string) => void;
  placeholder: string;
  autoFocus?: boolean;
}

/**
 * The note's rich text editor (P9-04). Its document is always the markdown
 * subset of `markdown.ts`: the markdown shortcuts as you type (`#`, `-`, `1.`,
 * `[ ]`, `**`) come from the extensions, and what is saved is our own
 * serialization, never the editor's HTML.
 */
/**
 * Tab inside a list keeps the list flat: the notes' markdown has no nested
 * lists, and a nested item would be saved merged into its parent. Elsewhere
 * Tab moves the focus as usual.
 */
const FlatLists = Extension.create({
  name: "flatLists",
  priority: 1_000,
  addKeyboardShortcuts() {
    return {
      Tab: () => this.editor.isActive("listItem") || this.editor.isActive("taskItem"),
    };
  },
});

/** The subset's extensions: nothing the markdown of `markdown.ts` cannot say. */
function extensionsFor(placeholder: string) {
  return [
    FlatLists,
    StarterKit.configure({
      blockquote: false,
      code: false,
      codeBlock: false,
      heading: { levels: [1, 2, 3] },
      horizontalRule: false,
      link: false,
      underline: false,
    }),
    Link.configure({
      autolink: true,
      defaultProtocol: "https",
      isAllowedUri: (url) => safeHref(url) !== null,
      openOnClick: false,
    }),
    TaskList,
    TaskItem.configure({ nested: false }),
    Placeholder.configure({ placeholder }),
  ];
}

export function NoteEditor({ autoFocus, label, markdown, onChange, placeholder }: NoteEditorProps) {
  const written = useRef(markdown);
  const onChangeRef = useRef(onChange);
  onChangeRef.current = onChange;
  // The options stay the same objects across renders: Tiptap compares them on
  // every render and applies any that changed to the live editor, so options
  // built inline would be applied again on each render of the note.
  const [content] = useState(() => markdownToDoc(markdown));
  const extensions = useMemo(() => extensionsFor(placeholder), [placeholder]);
  const editorProps = useMemo(
    () => ({
      attributes: {
        "aria-label": label,
        "aria-multiline": "true",
        class: styles.editor ?? "",
        role: "textbox",
      },
    }),
    [label],
  );
  const editor = useEditor({
    autofocus: autoFocus === true ? "end" : false,
    content,
    extensions,
    editorProps,
    onUpdate: ({ editor: current }) => {
      const next = docToMarkdown(current.getJSON());
      if (next === written.current) return;
      written.current = next;
      onChangeRef.current(next);
    },
  });
  // A newer text from the daemon replaces the document only while Mars is
  // not typing in it, and only when it differs from what she wrote.
  useEffect(() => {
    if (editor.isFocused || markdown === written.current) return;
    written.current = markdown;
    editor.commands.setContent(markdownToDoc(markdown), { emitUpdate: false });
  }, [editor, markdown]);
  return <EditorContent className={styles.frame} editor={editor} />;
}
