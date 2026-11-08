import {
  deleteMarkupBackward,
  insertNewlineContinueMarkupCommand,
} from "@codemirror/lang-markdown";
import { type Extension, Prec } from "@codemirror/state";
import { keymap } from "@codemirror/view";
import { toggleWrap } from "./editing";

/** What the keys of an open menu do; each says whether a menu took the key. */
export interface MenuKeys {
  move: (by: 1 | -1) => boolean;
  pick: () => boolean;
  dismiss: () => boolean;
}

/**
 * While a menu is open, ↑ ↓ choose in it, ↵ and Tab insert, Esc closes it;
 * with none open the keys do what they always do.
 */
export function menuKeys(menu: () => MenuKeys): Extension {
  return Prec.highest(
    keymap.of([
      { key: "ArrowDown", run: () => menu().move(1) },
      { key: "ArrowUp", run: () => menu().move(-1) },
      { key: "Enter", run: () => menu().pick() },
      { key: "Tab", run: () => menu().pick() },
      { key: "Escape", run: () => menu().dismiss() },
    ]),
  );
}

/**
 * ⌘B, ⌘I, ⌘E, ⌘K and ⌘⇧H put their mark around the selection (proposal
 * of guide 6); ⌘/ shows the raw Markdown; ↵ goes on with a list or a quote
 * and ends it on an empty item; ⌫ just after a list's mark takes it off.
 */
export function editingKeys(toggleRaw: () => void): Extension {
  return Prec.high(
    keymap.of([
      { key: "Mod-b", run: (view) => toggleWrap(view, "**") },
      { key: "Mod-i", run: (view) => toggleWrap(view, "*") },
      { key: "Mod-e", run: (view) => toggleWrap(view, "`") },
      { key: "Mod-k", run: (view) => toggleWrap(view, "[[", "]]") },
      { key: "Mod-Shift-h", run: (view) => toggleWrap(view, "==") },
      {
        key: "Mod-/",
        run: () => {
          toggleRaw();
          return true;
        },
      },
      { key: "Enter", run: insertNewlineContinueMarkupCommand({ nonTightLists: false }) },
      { key: "Backspace", run: deleteMarkupBackward },
    ]),
  );
}
