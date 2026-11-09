import type { AppCommand } from "../../bridge/generated";

/** How the note screen gives the daemon an order: the store's `send`, whose
 * promise resolves `false` when the daemon refused it (its notice says why). */
export type Send = (command: AppCommand) => unknown;

/**
 * The order's promise, for a control that waits until it is taken: it
 * rejects when the order was refused, so the control goes back to what it was.
 */
export function sent(send: Send, command: AppCommand): Promise<void> {
  return Promise.resolve(send(command)).then((accepted) => {
    if (accepted === false) throw new Error("refused");
  });
}
