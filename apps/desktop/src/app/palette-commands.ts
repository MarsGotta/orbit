import type { AppCommand } from "../bridge/generated";
import type { IconName } from "../design/Icon";
import { es as defaultCopy } from "../i18n/es";
import type { ErrorOrigin } from "../state/orbit-store";
import type { ViewId } from "../state/ui";

export type PaletteGroup = "actions" | "goTo" | "privacy";

/** One row of «Buscar y órdenes». `record` is the red dot of recording, not an icon. */
export interface PaletteCommand {
  id: string;
  group: PaletteGroup;
  icon: IconName | "record";
  title: string;
  subtitle?: string;
  shortcut?: string;
  run: () => void;
}

/** What the palette's rows do, from the window that holds it. */
export interface PaletteActions {
  navigate: (view: ViewId) => void;
  send: (command: AppCommand, origin?: ErrorOrigin) => void;
  openMeeting: () => void;
  /** The title of the next meeting, when there is one to prepare. */
  nextMeeting: string | null;
}

/** The order the groups are shown in, as the design lists them. */
export const paletteGroups: readonly PaletteGroup[] = ["actions", "goTo", "privacy"];

const views: readonly [ViewId, IconName][] = [
  ["briefing", "briefing"],
  ["calendar", "calendar"],
  ["notes", "notes"],
  ["assistant", "assistant"],
  ["settings", "settings"],
];

type Copy = typeof defaultCopy;

/** Acciones: record, call, a voice note and, when there is one, the next meeting. */
function actionCommands(actions: PaletteActions, copy: Copy["palette"]): PaletteCommand[] {
  const { navigate, send } = actions;
  const prepare: PaletteCommand[] =
    actions.nextMeeting === null
      ? []
      : [
          {
            id: "prepare",
            group: "actions",
            icon: "clock",
            title: copy.prepare.title,
            subtitle: actions.nextMeeting,
            run: () => navigate("briefing"),
          },
        ];
  return [
    {
      id: "record",
      group: "actions",
      icon: "record",
      ...copy.record,
      shortcut: copy.shortcuts.meeting,
      run: actions.openMeeting,
    },
    {
      id: "call",
      group: "actions",
      icon: "phone",
      ...copy.call,
      shortcut: copy.shortcuts.call,
      run: () => {
        navigate("assistant");
        // The call's refusal belongs to Assistant, where the palette leaves
        // the user, not to the view they opened the palette from.
        send({ command: "call_start" }, "assistant");
      },
    },
    {
      id: "voice",
      group: "actions",
      icon: "mic",
      ...copy.voice,
      shortcut: copy.shortcuts.ptt,
      run: () => navigate("assistant"),
    },
    ...prepare,
  ];
}

export function paletteCommands(
  actions: PaletteActions,
  /** The words, with the conversation agent's name. */
  es: Copy = defaultCopy,
): PaletteCommand[] {
  const copy = es.palette;
  return [
    ...actionCommands(actions, copy),
    ...views.map(([view, icon]): PaletteCommand => ({
      id: `go-${view}`,
      group: "goTo",
      icon,
      title: es.shell[view],
      run: () => actions.navigate(view),
    })),
    {
      id: "stop-all",
      group: "privacy",
      icon: "privacy",
      ...copy.stopAll,
      shortcut: copy.shortcuts.stop,
      run: () => actions.send({ command: "privacy_stop" }),
    },
  ];
}

/** Lower case without accents: «Reunión» and «REUNION» are the same search. */
function folded(text: string): string {
  return text
    .normalize("NFD")
    .replace(/\p{Diacritic}/gu, "")
    .toLowerCase();
}

/** The commands whose title or subtitle holds what was written, in their order. */
export function matchCommands(commands: PaletteCommand[], query: string): PaletteCommand[] {
  const wanted = folded(query.trim());
  if (wanted === "") return commands;
  return commands.filter((command) =>
    folded(`${command.title} ${command.subtitle ?? ""}`).includes(wanted),
  );
}

/** What matches no command goes to the agent as a new message, in Assistant. */
export function askAssistant(actions: PaletteActions, text: string): void {
  actions.navigate("assistant");
  actions.send({ command: "chat", text }, "assistant");
}
