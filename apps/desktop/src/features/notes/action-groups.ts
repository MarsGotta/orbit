import type { NoteActionSnapshot } from "../../bridge/generated/NoteActionSnapshot";
import { es } from "../../i18n/es";

export interface ActionGroup {
  key: string;
  label: string;
  mine: boolean;
  items: NoteActionSnapshot[];
}

const normal = (name: string) => name.trim().toLocaleLowerCase("es");

/**
 * Tasks by owner: the user first (their account's name or «Yo»), then each person
 * in the order the note names them, and «Sin asignar» last.
 */
export function groupActions(actions: NoteActionSnapshot[], ownerName: string): ActionGroup[] {
  const mine = new Set([normal(ownerName), normal("Yo")]);
  const groups = new Map<string, ActionGroup>();
  const order: string[] = [];
  for (const action of actions) {
    const owner = action.owner?.trim() ?? "";
    const key = owner === "" ? "" : mine.has(normal(owner)) ? "\u0000mine" : normal(owner);
    if (!groups.has(key)) {
      groups.set(key, {
        key,
        label:
          key === "" ? es.noteScreen.unassigned : key === "\u0000mine" ? es.noteScreen.you : owner,
        mine: key === "\u0000mine",
        items: [],
      });
      order.push(key);
    }
    groups.get(key)?.items.push(action);
  }
  const rank = (key: string) => (key === "\u0000mine" ? 0 : key === "" ? 2 : 1);
  return order
    .sort((left, right) => rank(left) - rank(right))
    .flatMap((key) => {
      const group = groups.get(key);
      return group ? [group] : [];
    });
}

/** All tasks as a bullet list, for «Copiar todas». */
export function actionsMarkdown(groups: ActionGroup[]): string {
  return groups
    .flatMap((group) =>
      group.items.map((action) => {
        const meta = [group.key === "" ? null : group.label, action.due].filter(Boolean);
        const mark = action.done ? "[x]" : "[ ]";
        return `- ${mark} ${action.text}${meta.length > 0 ? ` (${meta.join(" · ")})` : ""}`;
      }),
    )
    .join("\n");
}
