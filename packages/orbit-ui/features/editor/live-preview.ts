import { editorLanguage } from "./editor-language";
import { getCopy } from "../../i18n";
import { syntaxTree } from "@codemirror/language";
import type { EditorState, Range } from "@codemirror/state";
import {
  Decoration,
  type DecorationSet,
  type EditorView,
  ViewPlugin,
  type ViewUpdate,
} from "@codemirror/view";

import { Glyph, TaskBox } from "./preview-widgets";

/**
 * The note as it reads: every line but the one being written hides its
 * Markdown and shows it formatted; the line with the cursor shows its marks,
 * in grey. Only decorations: the document stays the text as typed.
 */

/** What each kind of inline node looks like, by its class. */
const INLINE_CLASS: Record<string, string> = {
  StrongEmphasis: "cm-md-strong",
  Emphasis: "cm-md-em",
  InlineCode: "cm-md-code",
  Strikethrough: "cm-md-strike",
  Highlight: "cm-md-highlight",
  WikiLink: "cm-md-link",
  Mention: "cm-md-mention",
};

/** The marks a line hides when the cursor is elsewhere. */
const INLINE_MARKS = new Set([
  "EmphasisMark",
  "StrikethroughMark",
  "HighlightMark",
  "WikiLinkMark",
]);

const HEADINGS: Record<string, string> = {
  ATXHeading1: "cm-md-h1",
  ATXHeading2: "cm-md-h2",
  ATXHeading3: "cm-md-h3",
  ATXHeading4: "cm-md-h3",
  ATXHeading5: "cm-md-h3",
  ATXHeading6: "cm-md-h3",
};

const markClass = Decoration.mark({ class: "cm-md-mark" });
const hidden = Decoration.replace({});

/**
 * The lines with a cursor, while the editor has the focus. A line with a
 * selection stays formatted, as the design draws it, so the format bar
 * marks the text as it reads.
 */
function activeLines(state: EditorState, focused: boolean): Set<number> {
  const lines = new Set<number>();
  if (!focused) return lines;
  for (const range of state.selection.ranges) {
    if (range.empty) lines.add(state.doc.lineAt(range.head).number);
  }
  return lines;
}

/** One space after a mark goes with it: `## ` and `> ` hide as one. */
function withSpace(state: EditorState, to: number): number {
  return state.sliceDoc(to, to + 1) === " " ? to + 1 : to;
}

function build(view: EditorView): DecorationSet {
  const { state } = view;
  const active = activeLines(state, view.hasFocus);
  const isActive = (pos: number) => active.has(state.doc.lineAt(pos).number);
  const out: Range<Decoration>[] = [];
  const lineClass = (pos: number, className: string) =>
    out.push(Decoration.line({ class: className }).range(state.doc.lineAt(pos).from));

  for (const line of active) lineClass(state.doc.line(line).from, "cm-md-active");

  for (const { from, to } of view.visibleRanges) {
    syntaxTree(state).iterate({
      from,
      to,
      enter: (node) => {
        const name = node.name;
        const heading = HEADINGS[name];
        if (heading !== undefined) {
          lineClass(node.from, heading);
          return;
        }
        const inline = INLINE_CLASS[name];
        if (inline !== undefined && node.to > node.from) {
          out.push(Decoration.mark({ class: inline }).range(node.from, node.to));
        }
        if (name === "HeaderMark" || name === "QuoteMark") {
          if (isActive(node.from)) out.push(markClass.range(node.from, node.to));
          else out.push(hidden.range(node.from, withSpace(state, node.to)));
          if (name === "QuoteMark" && !isActive(node.from)) lineClass(node.from, "cm-md-quote");
          return;
        }
        if (
          INLINE_MARKS.has(name) ||
          (name === "CodeMark" && node.node.parent?.name === "InlineCode")
        ) {
          out.push((isActive(node.from) ? markClass : hidden).range(node.from, node.to));
          return;
        }
        if (name === "ListMark") {
          listMark(state, node.from, node.to, isActive(node.from), out, lineClass);
          return;
        }
        if (name === "HorizontalRule") {
          if (isActive(node.from)) out.push(markClass.range(node.from, node.to));
          else
            out.push(
              Decoration.replace({ widget: new Glyph("", "cm-md-rule") }).range(node.from, node.to),
            );
          return false;
        }
        if (name === "FencedCode") {
          fencedCode(state, node.from, node.to, active, out);
          return false;
        }
        return undefined;
      },
    });
  }
  return Decoration.set(out, true);
}

function listMark(
  state: EditorState,
  from: number,
  to: number,
  active: boolean,
  out: Range<Decoration>[],
  lineClass: (pos: number, className: string) => void,
) {
  const line = state.doc.lineAt(from);
  const after = state.sliceDoc(to, line.to);
  const task = /^( +)\[([ xX])\]( ?)/.exec(after);
  const ordered = /^\d/.test(state.sliceDoc(from, to));
  if (task) {
    const at = to + (task[1]?.length ?? 1);
    const checked = task[2] !== " ";
    const end = at + 3 + (task[3]?.length ?? 0);
    lineClass(from, checked && !active ? "cm-md-task cm-md-done" : "cm-md-task");
    if (active) out.push(markClass.range(from, at + 3));
    else out.push(Decoration.replace({ widget: new TaskBox(checked, at) }).range(from, end));
    return;
  }
  lineClass(from, "cm-md-item");
  if (active) {
    out.push(markClass.range(from, to));
    return;
  }
  const glyph = ordered
    ? new Glyph(state.sliceDoc(from, to), "cm-md-number")
    : new Glyph("•", "cm-md-bullet");
  out.push(Decoration.replace({ widget: glyph }).range(from, withSpace(state, to)));
}

function fencedCode(
  state: EditorState,
  from: number,
  to: number,
  active: Set<number>,
  out: Range<Decoration>[],
) {
  const first = state.doc.lineAt(from);
  const last = state.doc.lineAt(to);
  const closed = last.number > first.number && /^\s*(`{3,}|~{3,})\s*$/.test(last.text);
  let writing = false;
  for (let number = first.number; number <= last.number; number++) {
    if (active.has(number)) writing = true;
  }
  for (let number = first.number; number <= last.number; number++) {
    const line = state.doc.line(number);
    const edge =
      number === first.number
        ? " cm-md-fence-open"
        : number === last.number && closed
          ? " cm-md-fence-close"
          : "";
    out.push(Decoration.line({ class: `cm-md-codeblock${edge}` }).range(line.from));
  }
  if (writing) return;
  const language = first.text.replace(/^\s*(`{3,}|~{3,})/, "").trim();
  out.push(
    Decoration.replace({
      widget: new Glyph(language || getCopy().editor.code, "cm-md-lang"),
    }).range(first.from, first.to),
  );
  if (closed && last.to > last.from) {
    out.push(hidden.range(last.from, last.to));
    out.push(Decoration.line({ class: "cm-md-fence-hidden" }).range(last.from));
  }
}

/** The formatted view; left out, the editor shows the raw Markdown. */
export const livePreview = ViewPlugin.fromClass(
  class {
    decorations: DecorationSet;
    constructor(view: EditorView) {
      this.decorations = build(view);
    }
    update(update: ViewUpdate) {
      if (
        update.startState.facet(editorLanguage) !== update.state.facet(editorLanguage) ||
        update.docChanged ||
        update.selectionSet ||
        update.viewportChanged ||
        update.focusChanged ||
        syntaxTree(update.startState) !== syntaxTree(update.state)
      ) {
        this.decorations = build(update.view);
      }
    }
  },
  { decorations: (plugin) => plugin.decorations },
);
