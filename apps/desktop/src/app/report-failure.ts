import type { DaemonPort } from "../bridge/port";
import type { ReadPage } from "../bridge/reads";
import { refusalOf } from "../bridge/refusal";
import { newOp, refusalReport } from "./interceptor";

/**
 * A refusal met outside an order the store sent (a link the panel
 * could not open, a read) goes to the report log with its own code and no
 * text, like any refused order. The screen says what to do; the log keeps
 * which refusal it was. A lost report is a lost diagnostic, never a failure.
 */
export function reportRefused(port: DaemonPort, error: unknown): void {
  void port.report(refusalReport(newOp(), refusalOf(error))).catch(() => undefined);
}

function reported<A extends unknown[], T>(
  port: DaemonPort,
  read: ((...args: A) => Promise<ReadPage<T>>) | undefined,
): ((...args: A) => Promise<ReadPage<T>>) | undefined {
  if (!read) return undefined;
  return async (...args) => {
    try {
      const answer = await read.apply(port, args);
      if (answer.state === "absent")
        void port.report(refusalReport(newOp(), answer.refusal)).catch(() => undefined);
      return answer;
    } catch (error: unknown) {
      reportRefused(port, error);
      throw error;
    }
  };
}

/**
 * The assistant's reads, reporting what they could not read. An unknown
 * answer is not a failure and is not reported.
 */
export function reportingReads(
  port: DaemonPort,
): Pick<DaemonPort, "readSessionsPage" | "readSessionMessages"> {
  return {
    readSessionsPage: reported(port, port.readSessionsPage),
    readSessionMessages: reported(port, port.readSessionMessages),
  };
}

/**
 * Ajustes › Agentes' one call, reporting what it met: the Mac's refusal or
 * the hub's code, never the text of a personality or a skill. The screen
 * says the cause with its numbers.
 */
export function reportingAgent(port: DaemonPort): DaemonPort["agent"] {
  const agent = port.agent;
  if (!agent) return undefined;
  return async (call) => {
    try {
      const answer = await agent.call(port, call);
      const { outcome } = answer;
      if (outcome.outcome === "failed")
        void port
          .report(refusalReport(newOp(), refusalOf({ refusal: outcome.refusal })))
          .catch(() => undefined);
      else if (outcome.outcome === "refused")
        void port
          .report(refusalReport(newOp(), refusalOf({ refusal: { code: outcome.refusal.code } })))
          .catch(() => undefined);
      return answer;
    } catch (error: unknown) {
      reportRefused(port, error);
      throw error;
    }
  };
}
