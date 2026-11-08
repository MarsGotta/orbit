import { getCopy } from "../i18n";
import { closedData } from "./report-data";
import type { Exceeded, Presented } from "./interceptor";
import { failureCode } from "./failure";
import type { Report } from "../bridge/port";

/** The report line of a wait past its ceiling: its stage, subject and numbers. */
export function exceededReport(incoming: Exceeded): Report {
  return {
    op: incoming.op,
    code: "ui_wait_exceeded",
    class: "failed",
    source: "ui",
    data: closedData({
      stage: incoming.stage,
      code: incoming.subject,
      elapsed_ms: incoming.elapsedMs,
      timeout_ms: incoming.ceilingMs,
    }),
  };
}

/**
 * A wait past its ceiling is a failure that says how long it waited. A
 * working message the daemon never closed is only logged: what it described
 * has ended and its outcome was said by its own code.
 */
export function presentExceeded(incoming: Exceeded): Presented {
  const report = exceededReport(incoming);
  if (incoming.stage === "working") return { op: incoming.op, class: "silent", text: "", report };
  const text =
    incoming.stage === "first_state"
      ? getCopy().extras.waits.firstState(incoming.feature, incoming.ceilingMs)
      : getCopy().extras.waits.exceeded(incoming.ceilingMs, failureCode(incoming.op));
  return {
    op: incoming.op,
    class: "failed",
    text,
    failure: { code: failureCode(incoming.op) },
    report,
  };
}
