import { editorLanguage } from "./editor-language";
import { getCopy } from "../../i18n";
import { type Extension, StateEffect, StateField } from "@codemirror/state";
import { Decoration, type DecorationSet, EditorView, WidgetType } from "@codemirror/view";

/**
 * What an assistant wrote for the note, shown under the line it was asked
 * from, marked «Escrito por {nombre}», until the person accepts or discards
 * it. It is never part of the document: nothing of it is saved unless
 * accepted, and accepting puts it in as text typed there.
 */
export interface Suggestion {
  author: string;
  text: string;
}

interface Placed extends Suggestion {
  /** The end of the line it hangs from, followed through every edit. */
  at: number;
}

export const showSuggestion = StateEffect.define<Placed | null>();

const placedSuggestion = StateField.define<Placed | null>({
  create: () => null,
  update(value, transaction) {
    let next = value && { ...value, at: transaction.changes.mapPos(value.at, 1) };
    for (const effect of transaction.effects) {
      if (effect.is(showSuggestion)) next = effect.value;
    }
    return next;
  },
});

/** Shows `suggestion` under the line of the cursor. */
export function placeSuggestion(view: EditorView, suggestion: Suggestion | null): void {
  const at = view.state.doc.lineAt(view.state.selection.main.head).to;
  view.dispatch({ effects: showSuggestion.of(suggestion && { ...suggestion, at }) });
}

function settle(view: EditorView, accepted: boolean): void {
  const placed = view.state.field(placedSuggestion, false);
  if (placed == null) return;
  if (!accepted) {
    view.dispatch({ effects: showSuggestion.of(null) });
    return;
  }
  view.dispatch({
    changes: { from: placed.at, insert: `\n${placed.text}` },
    selection: { anchor: placed.at + placed.text.length + 1 },
    effects: showSuggestion.of(null),
    userEvent: "input.suggestion",
  });
}

class SuggestionBlock extends WidgetType {
  readonly copy = getCopy();
  constructor(
    readonly placed: Placed,
    readonly settled: (accepted: boolean) => void,
  ) {
    super();
  }
  eq(other: SuggestionBlock): boolean {
    return (
      other.placed.text === this.placed.text &&
      other.placed.author === this.placed.author &&
      other.copy === this.copy
    );
  }
  toDOM(view: EditorView): HTMLElement {
    const by = getCopy().editor.writtenBy(this.placed.author);
    const block = document.createElement("div");
    block.className = "cm-md-suggestion";
    block.setAttribute("role", "group");
    block.setAttribute("aria-label", by);
    const title = document.createElement("div");
    title.className = "cm-md-suggestion-by";
    title.textContent = by;
    const body = document.createElement("div");
    body.className = "cm-md-suggestion-text";
    body.textContent = this.placed.text;
    const actions = document.createElement("div");
    actions.className = "cm-md-suggestion-actions";
    for (const [label, accepted] of [
      [getCopy().editor.accept, true],
      [getCopy().editor.discard, false],
    ] as const) {
      const button = document.createElement("button");
      button.type = "button";
      button.textContent = label;
      button.dataset.primary = String(accepted);
      button.addEventListener("click", () => {
        settle(view, accepted);
        this.settled(accepted);
      });
      actions.append(button);
    }
    block.append(title, body, actions);
    return block;
  }
  ignoreEvent(): boolean {
    return true;
  }
}

/** The pending suggestion and its block; `settled` hears what the person chose. */
export function suggestionBlock(settled: (accepted: boolean) => void): Extension {
  return [
    placedSuggestion,
    EditorView.decorations.compute([placedSuggestion, editorLanguage], (state): DecorationSet => {
      const placed = state.field(placedSuggestion);
      if (placed === null) return Decoration.none;
      const widget = new SuggestionBlock(placed, settled);
      return Decoration.set([Decoration.widget({ widget, block: true, side: 1 }).range(placed.at)]);
    }),
  ];
}
