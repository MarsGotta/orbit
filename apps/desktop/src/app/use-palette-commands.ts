import { useMemo, type Dispatch, type SetStateAction } from "react";
import type { AppCommand } from "../bridge/generated";
import type { ErrorOrigin } from "../state/orbit-store";
import type { ViewId } from "../state/ui";
import { spanish } from "../i18n/es";
import { paletteCommands } from "./palette";

export function usePaletteCommands(
  navigate: (view: ViewId) => void,
  send: (command: AppCommand) => Promise<unknown>,
  sendWithOrigin: (command: AppCommand, origin?: ErrorOrigin) => Promise<unknown>,
  setMeetingOpen: Dispatch<SetStateAction<boolean>>,
  assistantName?: string,
) {
  const copy = useMemo(() => spanish(assistantName), [assistantName]);
  return useMemo(
    () =>
      paletteCommands(
        navigate,
        (command, origin) =>
          void (origin === undefined ? send(command) : sendWithOrigin(command, origin)),
        () => setMeetingOpen(true),
        copy,
      ),
    [navigate, send, sendWithOrigin, setMeetingOpen, copy],
  );
}
