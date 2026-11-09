import { useCallback, useEffect, useState } from "react";
import type { AppCommand } from "../bridge/generated";
import { HUB_CONFIRMATION_MS } from "./ceilings";

/**
 * A choice the person made (a box, a dropdown, an answer to Deimos), shown as
 * made while its order travels and until what it changes is confirmed, so a
 * control does not snap back to the old value in between. The hold ends when
 * the confirmed value changes (to the choice or to anything else: what is
 * known wins), when the order is refused (`send` resolved to `false`; its
 * message says why), or past `HUB_CONFIRMATION_MS`.
 *
 * `confirmed` is compared with `Object.is`: pass a primitive.
 */
export function useHeldChoice<T>(confirmed: T, send: (command: AppCommand) => unknown) {
  const [held, setHeld] = useState<{ value: T; over: T } | null>(null);
  useEffect(() => {
    if (held !== null && !Object.is(held.over, confirmed)) setHeld(null);
  }, [confirmed, held]);
  useEffect(() => {
    if (held === null) return;
    const timer = setTimeout(() => setHeld(null), HUB_CONFIRMATION_MS);
    return () => clearTimeout(timer);
  }, [held]);
  const choose = useCallback(
    (value: T, command: AppCommand) => {
      const hold = { value, over: confirmed };
      setHeld(hold);
      void Promise.resolve(send(command)).then((accepted) => {
        if (accepted === false) setHeld((current) => (current === hold ? null : current));
      });
    },
    [confirmed, send],
  );
  return { shown: held === null ? confirmed : held.value, pending: held !== null, choose };
}
