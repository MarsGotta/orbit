import { getCopy } from "../../i18n";
import { useCopy } from "../../i18n/CopyProvider";
import type { EditorView } from "@codemirror/view";
import { type ReactNode, useLayoutEffect, useRef, useState } from "react";
import { Tooltip } from "../../design/Tooltip";

import { EditorMenu } from "./EditorMenu";
import type { MenuKeys } from "./editor-keys";
import { FormatBar } from "./FormatBar";
import { type LinkTarget, menuWords, type Person } from "./menus";
import type { Suggestion } from "./suggestion";
import { useEditorMenu } from "./use-editor-menu";
import { useMarkdownEditor } from "./use-markdown-editor";
import styles from "./MarkdownEditor.module.css";

/** The principal assistant, for «Preguntar a…» and «Pedir a… que siga escribiendo». */
export interface EditorAssistant {
  name: string;
  /** The text chosen, to ask about it. */
  ask: (selection: string) => void;
  /** The text up to the cursor, to go on from it. */
  keepWriting: (text: string) => void;
}

export interface MarkdownEditorProps {
  /** Accessible name of the text. */
  label: string;
  /** The Markdown it opens with, and any newer one while it has no focus. */
  value: string;
  /** The Markdown exactly as typed, on every change. */
  onChange?: (markdown: string) => void;
  /** The focus left the text: the moment to save what is waiting. */
  onBlur?: () => void;
  /** What its saving says, over the text on the left. */
  status?: ReactNode;
  /** What the empty text says; «Escribe o pulsa / para insertar…» by default. */
  placeholder?: string;
  readOnly?: boolean;
  autoFocus?: boolean;
  /** The help line under it, where the editor is the view's main text. */
  help?: boolean;
  /** The most characters the text can hold; said before it is reached. */
  maxChars?: number;
  /** What «[[» lists. */
  notes?: readonly LinkTarget[];
  /** What «@» lists. */
  people?: readonly Person[];
  assistant?: EditorAssistant;
  /** What the assistant wrote, shown marked until accepted or discarded. */
  suggestion?: Suggestion | null;
  onSuggestionSettled?: (accepted: boolean) => void;
}

/** The share of the ceiling left when the editor starts saying how much is left. */
const NEAR_LIMIT_SHARE = 0.1;

const NO_MENU: MenuKeys = { move: () => false, pick: () => false, dismiss: () => false };

function limitWords(count: number, refused: boolean, max?: number): string | null {
  if (max === undefined) return null;
  if (refused || count >= max) return getCopy().editor.atLimit(max);
  if (max - count <= max * NEAR_LIMIT_SHARE) return getCopy().editor.nearLimit(max - count, max);
  return null;
}

/** The top and bottom of a position of the text, from the top left of the editor's box. */
function placeOf(view: EditorView | null, box: HTMLElement | null, pos: number | undefined) {
  const at = view && pos !== undefined ? view.coordsAtPos(pos) : null;
  const frame = box?.getBoundingClientRect();
  if (!at || !frame) return { top: 0, bottom: 0, left: 0 };
  return { top: at.top - frame.top, bottom: at.bottom - frame.top, left: at.left - frame.left };
}

/** Its saving on the left and «Markdown» on the right, which shows the raw text. */
function EditorBar({
  raw,
  status,
  toggle,
}: {
  raw: boolean;
  status: ReactNode;
  toggle: () => void;
}) {
  const inlineCopy = useCopy().editorInline;
  const es = useCopy();
  return (
    <div className={styles.bar} data-quiet={status == null || status === false || undefined}>
      <span aria-live="polite" className={styles.status}>
        {status}
      </span>
      <Tooltip text={raw ? es.editor.showFormatted : es.editor.showMarkdown}>
        <button
          aria-pressed={raw}
          className={styles.rawToggle}
          onClick={toggle}
          onMouseDown={(event) => event.preventDefault()}
          type="button"
        >
          <span aria-hidden="true" className={styles.md}>
            {inlineCopy.markdownBadge}
          </span>
          {raw ? es.editor.backToText : es.editor.markdown}
        </button>
      </Tooltip>
    </div>
  );
}

function HelpLine() {
  const es = useCopy();
  return (
    <p aria-label={es.editor.helpLabel} className={styles.help}>
      {es.editor.help.map(([keys, what]) => (
        <span key={keys}>
          <span className={styles.keys}>{keys}</span> {what}
        </span>
      ))}
    </p>
  );
}

/** Where the menu and the format bar hang: under the trigger, over the selection. */
function usePlaces(view: () => EditorView | null, menuAt?: number, barAt?: number) {
  const box = useRef<HTMLDivElement>(null);
  const [places, setPlaces] = useState({
    menu: { top: 0, bottom: 0, left: 0 },
    bar: { top: 0, bottom: 0, left: 0 },
  });
  useLayoutEffect(() => {
    setPlaces({
      menu: placeOf(view(), box.current, menuAt),
      bar: placeOf(view(), box.current, barAt),
    });
    // `view` reads a ref: what moves them is where they hang from.
  }, [menuAt, barAt]);
  return { box, places };
}

/**
 * The one Markdown editor of Orbit (guide 6, «El editor»): what it holds is
 * the Markdown as typed; every line but the cursor's shows it formatted.
 * «/» inserts, «[[» links a note by its title, «@» mentions a person, a
 * selection opens the format bar and ⌘/ shows the raw Markdown.
 */
export function MarkdownEditor(props: MarkdownEditorProps) {
  const keys = useRef<MenuKeys>(NO_MENU);
  const editor = useMarkdownEditor(props, () => keys.current);
  const current = () => editor.view.current;
  const {
    menu,
    choose,
    pick,
    keys: menuKeys,
  } = useEditorMenu(current, editor.trigger, {
    notes: props.notes,
    people: props.people,
    assistant: props.assistant?.name,
    keepWriting: props.assistant?.keepWriting,
  });
  keys.current = menuKeys;
  const selection = editor.selection;
  const { box, places } = usePlaces(current, menu?.trigger.from, selection?.from);
  const limit = limitWords(editor.count, editor.refused, props.maxChars);
  return (
    <div className={styles.editor} data-raw={editor.raw || undefined}>
      <EditorBar
        raw={editor.raw}
        status={props.status}
        toggle={() => {
          editor.setRaw(!editor.raw);
          current()?.focus();
        }}
      />
      {limit !== null && (
        <p className={styles.limit} role="status">
          {limit}
        </p>
      )}
      <div className={styles.box} ref={box}>
        <div className={styles.text} ref={editor.holder} />
        {menu !== null && (
          <EditorMenu
            active={menu.active}
            empty={menuWords(menu.trigger).empty}
            onHover={choose}
            onPick={pick}
            place={places.menu}
            rows={menu.rows}
            title={menuWords(menu.trigger).title}
          />
        )}
        {selection !== null && menu === null && props.readOnly !== true && (
          <FormatBar
            assistant={props.assistant}
            place={places.bar}
            selection={selection}
            view={current}
          />
        )}
      </div>
      {props.help === true && <HelpLine />}
    </div>
  );
}
