import { getCopy } from "../../i18n";
import { type EditorView, WidgetType } from "@codemirror/view";

/** What the formatted view draws in place of Markdown: a task's box, a bullet, a rule. */

export class TaskBox extends WidgetType {
  readonly label = getCopy().editor.markTask;
  constructor(
    readonly checked: boolean,
    /** Where its `[ ]` starts, which a press turns into `[x]` and back. */
    readonly at: number,
  ) {
    super();
  }
  eq(other: TaskBox): boolean {
    return other.checked === this.checked && other.at === this.at && other.label === this.label;
  }
  toDOM(view: EditorView): HTMLElement {
    const box = document.createElement("span");
    box.className = "cm-md-taskbox";
    box.setAttribute("role", "checkbox");
    box.setAttribute("aria-checked", String(this.checked));
    box.setAttribute("aria-label", this.label);
    box.dataset.checked = String(this.checked);
    box.addEventListener("mousedown", (event) => {
      // The press toggles the task without moving the cursor into its line.
      event.preventDefault();
      toggleTask(view, this.at);
    });
    return box;
  }
  ignoreEvent(): boolean {
    return true;
  }
}

export class Glyph extends WidgetType {
  constructor(
    readonly text: string,
    readonly className: string,
  ) {
    super();
  }
  eq(other: Glyph): boolean {
    return other.text === this.text && other.className === this.className;
  }
  toDOM(): HTMLElement {
    const glyph = document.createElement("span");
    glyph.className = this.className;
    glyph.setAttribute("aria-hidden", "true");
    glyph.textContent = this.text;
    return glyph;
  }
}

/** `[ ]` ↔ `[x]` of the task whose marker starts at `at`. */
export function toggleTask(view: EditorView, at: number): boolean {
  const marker = view.state.sliceDoc(at, at + 3);
  if (!/^\[[ xX]\]$/.test(marker)) return false;
  view.dispatch({
    changes: { from: at + 1, to: at + 2, insert: marker[1] === " " ? "x" : " " },
    userEvent: "input.toggle",
  });
  return true;
}
