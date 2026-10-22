import type { DaemonPort } from "../bridge/port";
import { commandError } from "../bridge/refusal";
import { commandFeature } from "../bridge/generated/commandFeatures";
import { newOp } from "../app/interceptor";
import { orderCeilingMs } from "./ceilings";
import { intercept } from "./daemon-watchers";
import type { Get, OrbitState, Set } from "./orbit-store";
import type { WaitRegistry } from "./wait-registry";

/**
 * Every order leaves here: with a new op, as a wait of the registry until the
 * screen shows its receipt, and with its refusal, or its ceiling, said by the
 * interceptor where it was sent from.
 */
export function sender(
  port: DaemonPort,
  get: Get,
  set: Set,
  waits: WaitRegistry,
): OrbitState["send"] {
  return async (command, origin) => {
    const feature = commandFeature[command.command];
    const op = newOp();
    // Its context, should it go past its ceiling: said and cleared as a refusal.
    const context = { place: origin, feature, command };
    let exceeded: () => void = () => undefined;
    const ceiling = new Promise<"exceeded">((resolve) => {
      exceeded = () => resolve("exceeded");
    });
    waits.open(
      {
        op,
        kind: "order",
        feature,
        command: command.command,
        origin,
        ceilingMs: orderCeilingMs(command.command),
      },
      {
        context,
        then: () => {
          // One refusal per function, as for any other.
          set({
            messages: get().messages.filter(
              (message) => message.feature !== feature || message.op === op,
            ),
          });
          exceeded();
        },
      },
    );
    let receipt: Awaited<ReturnType<DaemonPort["send"]>> | "exceeded";
    try {
      receipt = await Promise.race([port.send(command, op), ceiling]);
    } catch (error: unknown) {
      // Past its ceiling the order was already said as failed: a late answer is not shown.
      if (!waits.isOpen(op)) return false;
      waits.close(op);
      const { refusal } = commandError(error, command);
      // One refusal per function, as before: the newest replaces the last.
      set({ messages: get().messages.filter((message) => message.feature !== feature) });
      intercept(port, get, set, { kind: "refusal", op, refusal }, context);
      return false;
    }
    if (receipt === "exceeded" || !waits.isOpen(op)) return false;
    // Closed when the screen shows what the receipt says: at once, or with its frames.
    waits.receipt(op, receipt?.revision);
    // An accepted order clears its function; rearm also lifts privacy refusals elsewhere.
    set({
      messages: get().messages.filter(
        (message) =>
          message.feature !== feature &&
          !(command.command === "rearm" && message.refusalCode === "privacy_stopped"),
      ),
    });
    return true;
  };
}
