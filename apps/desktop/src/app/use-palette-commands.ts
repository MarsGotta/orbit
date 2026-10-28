import { useMemo, type Dispatch, type SetStateAction } from "react";
import type { AppCommand } from "../bridge/generated";
import type { ErrorOrigin } from "../state/orbit-store";
import type { AppView } from "../state/app-view";
import type { ViewId } from "../state/ui";
import { primaryAgentName } from "../features/assistant/agents";
import { spanish } from "../i18n/es";
import { askAssistant, paletteCommands, type PaletteActions } from "./palette-commands";
import type { PaletteProps } from "./Palette";

/** What the window's palette offers, and where asking the agent goes. */
export type PaletteContent = Pick<PaletteProps, "commands" | "onAsk">;

export function usePaletteCommands(
  navigate: (view: ViewId) => void,
  /**
   * The store's send. The palette floats over the window, not inside a view:
   * an order without its own view says its refusal in the corner notice.
   */
  send: (command: AppCommand, origin?: ErrorOrigin) => Promise<unknown>,
  setMeetingOpen: Dispatch<SetStateAction<boolean>>,
  snapshot: AppView | null,
): PaletteContent {
  const assistantName = snapshot ? primaryAgentName(snapshot) : undefined;
  const nextMeeting = snapshot?.briefing?.next?.title ?? null;
  const copy = useMemo(() => spanish(assistantName), [assistantName]);
  return useMemo(() => {
    const actions: PaletteActions = {
      navigate,
      send: (command, origin) => void send(command, origin),
      openMeeting: () => setMeetingOpen(true),
      nextMeeting,
    };
    return {
      commands: paletteCommands(actions, copy),
      onAsk: (text: string) => askAssistant(actions, text),
    };
  }, [navigate, send, setMeetingOpen, nextMeeting, copy]);
}
