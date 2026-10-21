import { useMemo, type Dispatch, type SetStateAction } from "react";
import type { AppCommand } from "../bridge/generated";
import type { ErrorOrigin } from "../state/orbit-store";
import type { ViewId } from "../state/ui";
import { paletteCommands } from "./palette";

export function usePaletteCommands(
  navigate: (view: ViewId) => void,
  send: (command: AppCommand) => Promise<void>,
  sendWithOrigin: (command: AppCommand, origin?: ErrorOrigin) => Promise<void>,
  setMeetingOpen: Dispatch<SetStateAction<boolean>>,
) {
  return useMemo(
    () =>
      paletteCommands(
        navigate,
        (command, origin) =>
          void (origin === undefined ? send(command) : sendWithOrigin(command, origin)),
        () => setMeetingOpen(true),
      ),
    [navigate, send, sendWithOrigin, setMeetingOpen],
  );
}
