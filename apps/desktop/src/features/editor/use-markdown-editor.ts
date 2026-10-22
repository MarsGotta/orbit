import type { EditorView, ViewUpdate } from "@codemirror/view";
import { useEffect, useRef, useState } from "react";
import { es } from "../../i18n/es";
import {
  createEditor,
  editableOf,
  EditorParts,
  external,
  labelOf,
  previewOf,
} from "./create-editor";
import { characters } from "./editing";
import type { MenuKeys } from "./editor-keys";
import { type Trigger, triggerAt } from "./menus";
import { placeSuggestion, type Suggestion } from "./suggestion";

export interface EditorOptions {
  label: string;
  value: string;
  placeholder?: string;
  readOnly?: boolean;
  autoFocus?: boolean;
  maxChars?: number;
  suggestion?: Suggestion | null;
  onChange?: (markdown: string) => void;
  onBlur?: () => void;
  onSuggestionSettled?: (accepted: boolean) => void;
}

/** What the editor's frame draws from: the cursor's menu, the selection, the count. */
interface Reading {
  trigger: Trigger | null;
  selection: { from: number; to: number } | null;
}

/**
 * The CodeMirror view behind the editor, made once: a newer `value` from
 * outside replaces its text only while nobody is typing in it; the raw view,
 * the name and read-only are reconfigured in place.
 */
export function useMarkdownEditor(options: EditorOptions, menu: () => MenuKeys) {
  const holder = useRef<HTMLDivElement>(null);
  const view = useRef<EditorView | null>(null);
  const latest = useRef(options);
  latest.current = options;
  const [parts] = useState(() => new EditorParts());
  const [raw, setRaw] = useState(false);
  const [reading, setReading] = useState<Reading>({ trigger: null, selection: null });
  const [count, setCount] = useState(() => characters(options.value));
  const [refused, setRefused] = useState(false);

  const updated = (update: ViewUpdate) => {
    if (update.docChanged) {
      const text = update.state.doc.toString();
      setCount(characters(text));
      setRefused(false);
      if (!update.transactions.some((transaction) => transaction.annotation(external) === true)) {
        latest.current.onChange?.(text);
      }
    }
    if (update.docChanged || update.selectionSet || update.focusChanged) {
      const focused = update.view.hasFocus;
      const main = update.state.selection.main;
      setReading({
        trigger: focused ? triggerAt(update.state) : null,
        selection: focused && !main.empty ? { from: main.from, to: main.to } : null,
      });
    }
  };

  useEffect(() => {
    if (!holder.current) return undefined;
    const made = createEditor({
      parent: holder.current,
      value: latest.current.value,
      label: latest.current.label,
      placeholder: latest.current.placeholder ?? es.editor.placeholder,
      readOnly: latest.current.readOnly === true,
      maxChars: latest.current.maxChars,
      parts,
      menu,
      toggleRaw: () => setRaw((current) => !current),
      refused: () => setRefused(true),
      settled: (accepted) => latest.current.onSuggestionSettled?.(accepted),
      blurred: () => latest.current.onBlur?.(),
      updated,
    });
    view.current = made;
    if (latest.current.autoFocus === true) {
      made.focus();
      made.dispatch({ selection: { anchor: made.state.doc.length } });
    }
    return () => {
      view.current = null;
      made.destroy();
    };
    // Made once: `latest` carries what changes, the effects below the rest.
  }, []);

  useOutsideChanges(view, options, parts, raw);
  return { holder, view, raw, setRaw, ...reading, count, refused };
}

/** What comes from outside after the editor was made. */
function useOutsideChanges(
  view: { current: EditorView | null },
  options: EditorOptions,
  parts: EditorParts,
  raw: boolean,
) {
  const { label, readOnly, suggestion, value } = options;
  useEffect(() => {
    const editor = view.current;
    if (!editor || editor.hasFocus) return;
    const current = editor.state.doc.toString();
    if (current === value) return;
    editor.dispatch({
      changes: { from: 0, to: current.length, insert: value },
      annotations: external.of(true),
    });
  }, [view, value]);

  useEffect(() => {
    view.current?.dispatch({
      effects: [
        parts.preview.reconfigure(previewOf(raw)),
        parts.label.reconfigure(labelOf(raw ? es.editor.rawLabel(label) : label)),
      ],
    });
  }, [view, parts, label, raw]);

  useEffect(() => {
    view.current?.dispatch({ effects: parts.editable.reconfigure(editableOf(readOnly === true)) });
  }, [view, parts, readOnly]);

  useEffect(() => {
    const editor = view.current;
    if (editor && suggestion !== undefined) placeSuggestion(editor, suggestion);
  }, [view, suggestion]);
}
