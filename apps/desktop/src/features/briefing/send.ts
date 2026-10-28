import type { AppCommand } from "../../bridge/generated";

/**
 * How Today sends an order. A promise, when there is one, says whether the
 * daemon took it: a refused mark goes back to what the hub has.
 */
export type Send = (command: AppCommand) => void | Promise<unknown>;
