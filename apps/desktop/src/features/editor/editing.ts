import { EditorSelection, EditorState, type Extension } from "@codemirror/state";
import type { EditorView } from "@codemirror/view";

/**
 * Puts `open` and `close` around each selection, or takes them away when
 * they are already there: ⌘B twice leaves the text as it was.
 */
export function toggleWrap(view: EditorView, open: string, close = open): boolean {
  const { state } = view;
  const changes = state.changeByRange((range) => {
    const before = state.sliceDoc(range.from - open.length, range.from);
    const after = state.sliceDoc(range.to, range.to + close.length);
    if (before === open && after === close) {
      return {
        changes: [
          { from: range.from - open.length, to: range.from },
          { from: range.to, to: range.to + close.length },
        ],
        range: EditorSelection.range(range.from - open.length, range.to - open.length),
      };
    }
    return {
      changes: [
        { from: range.from, insert: open },
        { from: range.to, insert: close },
      ],
      range: EditorSelection.range(range.from + open.length, range.to + open.length),
    };
  });
  view.dispatch(state.update(changes, { userEvent: "input.format", scrollIntoView: true }));
  return true;
}

/** Replaces `from`–`to` with `before` and `after`, the cursor between them. */
export function insertAt(
  view: EditorView,
  from: number,
  to: number,
  before: string,
  after = "",
): void {
  view.dispatch({
    changes: { from, to, insert: before + after },
    selection: { anchor: from + before.length },
    userEvent: "input.complete",
    scrollIntoView: true,
  });
}

/** The characters of a text as the daemon counts them: code points, not UTF-16 units. */
export function characters(text: string): number {
  return Array.from(text).length;
}

/**
 * Refuses the changes that would take the text past `max` characters; a
 * change that shortens it, or keeps it under, always passes.
 */
export function characterLimit(max: number, onRefused: () => void): Extension {
  return EditorState.transactionFilter.of((transaction) => {
    if (!transaction.docChanged) return transaction;
    const next = transaction.newDoc;
    // A text can only hold more characters than UTF-16 units of length.
    if (next.length <= max) return transaction;
    if (next.length <= transaction.startState.doc.length) return transaction;
    if (characters(next.toString()) <= max) return transaction;
    queueMicrotask(onRefused);
    return [];
  });
}
