import type { EditorView } from "@codemirror/view";
import { useState } from "react";
import { insertAt } from "./editing";
import type { MenuKeys } from "./editor-keys";
import { type MenuRow, type MenuSources, menuRows, slashOptions, type Trigger } from "./menus";

export interface OpenMenu {
  trigger: Trigger;
  rows: MenuRow[];
  active: number;
}

function keyOf(trigger: Trigger): string {
  return `${trigger.kind}:${trigger.from}:${trigger.query}`;
}

/**
 * The menu the cursor opened, the row chosen in it, and what choosing a row
 * writes: «/» puts its Markdown at the start of the line, «[[» the note's
 * title as a link, «@» the person's name. Esc closes it until another
 * trigger is typed.
 */
export function useEditorMenu(
  view: () => EditorView | null,
  trigger: Trigger | null,
  sources: MenuSources & { keepWriting?: (text: string) => void },
) {
  const [choice, setChoice] = useState({ key: "", index: 0 });
  const [dismissed, setDismissed] = useState("");
  const closed = trigger === null || dismissed === `${trigger.kind}:${trigger.from}`;
  const rows = closed ? null : menuRows(trigger, sources);
  const menu: OpenMenu | null =
    rows === null || trigger === null
      ? null
      : {
          trigger,
          rows,
          active:
            choice.key === keyOf(trigger)
              ? Math.min(choice.index, Math.max(0, rows.length - 1))
              : 0,
        };

  const choose = (index: number) => {
    if (menu) setChoice({ key: keyOf(menu.trigger), index });
  };

  const pick = (row: MenuRow) => {
    const editor = view();
    if (!editor || !menu) return;
    const { from, to, kind } = menu.trigger;
    if (kind === "slash") {
      const option = slashOptions(sources.assistant ?? null).find((item) => item.id === row.id);
      if (!option) return;
      insertAt(editor, from, to, option.before, option.after);
      if (option.id === "assistant") sources.keepWriting?.(editor.state.sliceDoc(0, from));
      return;
    }
    const atLineEnd = editor.state.doc.lineAt(to).to === to;
    const written = kind === "link" ? `[[${row.label}]]` : `@${row.label}`;
    insertAt(editor, from, to, written + (atLineEnd ? " " : ""));
  };

  const keys: MenuKeys = {
    move: (by) => {
      if (!menu || menu.rows.length === 0) return false;
      choose((menu.active + by + menu.rows.length) % menu.rows.length);
      return true;
    },
    pick: () => {
      const row = menu?.rows[menu.active];
      if (!row) return false;
      pick(row);
      return true;
    },
    dismiss: () => {
      if (!menu) return false;
      setDismissed(`${menu.trigger.kind}:${menu.trigger.from}`);
      return true;
    },
  };

  return { menu, choose, pick, keys };
}
