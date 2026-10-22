import { EditorView } from "@codemirror/view";
import { act, fireEvent, screen } from "@testing-library/react";

/**
 * Drives the editor as a person does, in a test: jsdom lays nothing out and
 * has no input method, so a key typed goes in as the editor puts a typed
 * key in, and the keys it binds go through its own handling of keydown.
 */

/** The editor whose text is named `name`. */
export function editorNamed(name: string): EditorView {
  const box = screen.getByRole("textbox", { name });
  const view = EditorView.findFromDOM(box);
  if (!view) throw new Error(`«${name}» is not the Markdown editor`);
  return view;
}

/** Puts the focus in the text, with the cursor at `at` (its end by default). */
export function focusAt(view: EditorView, at = view.state.doc.length): void {
  act(() => {
    view.focus();
    view.dispatch({ selection: { anchor: at } });
  });
}

/** Types `text` where the cursor is, one key at a time; `\n` is ↵. */
export function typeText(view: EditorView, text: string): void {
  for (const key of text) {
    if (key === "\n") {
      press(view, "Enter");
      continue;
    }
    act(() => {
      view.dispatch({ ...view.state.replaceSelection(key), userEvent: "input.type" });
    });
  }
}

/** Pastes `text` where the cursor is, as ⌘V puts plain text in. */
export function paste(view: EditorView, text: string): void {
  act(() => {
    view.dispatch({ ...view.state.replaceSelection(text), userEvent: "input.paste" });
  });
}

/** Presses a key the editor may bind: «Enter», «ArrowDown», «Escape», «Mod-b»… */
export function press(view: EditorView, key: string): void {
  const parts = key.split("-");
  const name = parts.pop() ?? key;
  const mods = new Set(parts);
  // «Mod» is ⌘ on a Mac and Ctrl elsewhere, as the editor reads it.
  const mac = /Mac/.test(navigator.platform);
  act(() => {
    fireEvent.keyDown(view.contentDOM, {
      key: name,
      metaKey: mods.has("Mod") && mac,
      ctrlKey: mods.has("Ctrl") || (mods.has("Mod") && !mac),
      shiftKey: mods.has("Shift"),
    });
  });
}

/** Selects `from`–`to`, as a drag of the mouse leaves it. */
export function select(view: EditorView, from: number, to: number): void {
  act(() => {
    view.focus();
    view.dispatch({ selection: { anchor: from, head: to } });
  });
}

/** Takes the focus away from the text, as a click elsewhere does. */
export function blur(view: EditorView): void {
  act(() => {
    view.contentDOM.blur();
  });
}

/** Each line as it reads on screen: its visible text and its classes. */
export function lines(view: EditorView): { text: string; classes: string[] }[] {
  return [...view.contentDOM.querySelectorAll<HTMLElement>(".cm-line")].map((line) => ({
    text: line.textContent ?? "",
    classes: [...line.classList],
  }));
}
