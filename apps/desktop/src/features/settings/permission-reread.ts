import { useEffect, useRef } from "react";
import type { AppCommand } from "../../bridge/generated";

/**
 * Asks the helper what macOS grants without asking macOS for anything: a
 * `permissions` order with nothing requested is answered with the four.
 */
export const REREAD_PERMISSIONS: AppCommand = { command: "permissions", request: [] };

/**
 * While `active`, each time Orbit's window has the focus again (back from
 * System Settings, typically) macOS is asked again what it grants; `then`
 * runs once that order settles, whatever its answer. The daemon asks on its
 * own only for the screen and accessibility, and only while the engine needs
 * them, so a grant of the microphone or the calendars made there would
 * otherwise never reach the screen.
 */
export function useRereadOnReturn(
  send: (command: AppCommand) => unknown,
  active: boolean,
  then?: () => void,
): void {
  const latest = useRef({ send, then });
  latest.current = { send, then };
  useEffect(() => {
    if (!active) return;
    const reread = () => {
      void Promise.resolve(latest.current.send(REREAD_PERMISSIONS))
        .catch(() => undefined)
        .finally(() => latest.current.then?.());
    };
    window.addEventListener("focus", reread);
    return () => window.removeEventListener("focus", reread);
  }, [active]);
}
