import type { DaemonPort } from "../bridge/port";
import type { ReadPage } from "../bridge/reads";
import { refusalOf } from "../bridge/refusal";
import type { OrbitState } from "../state/orbit-store";
import type { DirectCall } from "../state/ceilings";
import { newOp, refusalReport } from "./interceptor";

/** Runs a call through the registry of what is in flight; as is, without one. */
type Track = OrbitState["track"];
const untracked: Track = (_call, _feature, work) => work();

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
  call: DirectCall,
  track: Track,
): ((...args: A) => Promise<ReadPage<T>>) | undefined {
  if (!read) return undefined;
  return async (...args) => {
    try {
      const answer = await track(call, "assistant", () => read.apply(port, args), "assistant");
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
  track: Track = untracked,
): Pick<DaemonPort, "readSessionsPage" | "readSessionMessages"> {
  return {
    readSessionsPage: reported(port, port.readSessionsPage, "sessions_page", track),
    readSessionMessages: reported(port, port.readSessionMessages, "session_messages", track),
  };
}

/**
 * Settings › Agents' one call, reporting what it met: the Mac's refusal or
 * the hub's code, never the text of a personality or a skill. The screen
 * says the cause with its numbers.
 */
export function reportingAgent(port: DaemonPort, track: Track = untracked): DaemonPort["agent"] {
  const agent = port.agent;
  if (!agent) return undefined;
  return async (call) => {
    try {
      const answer = await track("agent", "assistant", () => agent.call(port, call), "settings");
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
