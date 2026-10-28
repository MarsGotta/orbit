import { formatQuantity } from "../locale";
import type { Refusal } from "../../bridge/refusal";
import { fields } from "./refusalDetails";

export function interrupted(refusal: Refusal): string {
  const cause = fields(refusal.cause);
  let reason: string;
  if (cause.kind === "generation_changed") {
    reason = `The read started in generation ${formatQuantity(cause.started_generation)}; the Mac is on generation ${formatQuantity(cause.current_generation)}.`;
  } else if (cause.kind === "inventory_dispatch") {
    reason = `Only ${formatQuantity(cause.sent_reads)} of ${formatQuantity(cause.total_reads)} inventory requests were sent. There is no complete list.`;
  } else if (cause.kind === "calendar_changed") {
    reason = `The calendar changed ${formatQuantity(cause.changes)} times; ${formatQuantity(cause.max_restarts)} restarts per read are allowed.`;
  } else if (cause.kind === "calendar_expired") {
    reason = `The agenda read exceeded its ${formatQuantity(cause.deadline_ms)} ms deadline.`;
  } else if (cause.kind === "helper_restarted") {
    reason = `The helper restarted; previous generation: ${String(cause.previous_generation ?? "not observed")}; current: ${String(cause.current_generation ?? "not observed")}.`;
  } else {
    const reasons: Record<string, string> = {
      cancelled: "The read was canceled.",
      authority: "The operation’s authorization changed.",
      conversation_changed: "The conversation changed during the read.",
      calendar_helper_stopped: "The calendar reader disconnected.",
      calendar_read_restarted: "Another agenda read started.",
      calendar_generation_changed: "The calendar reader’s generation changed.",
      calendar_reply_replaced: "Another response replaced the agenda read.",
      source_reply_replaced: "Another response replaced the source read.",
      capture_stopped: "Capture stopped during the read.",
      computer_revoked: "Computer control was withdrawn during the read.",
    };
    reason =
      reasons[String(cause.kind)] ??
      "The read was interrupted for a cause this version doesn’t recognize.";
  }
  return `${reason} Request it again if still needed.`;
}

export function calendarLimit(refusal: Refusal): string {
  const cause = fields(refusal.cause);
  if (cause.kind === "page")
    return `An agenda page exceeds the ${formatQuantity(cause.max_bytes)} byte limit. Request a shorter range.`;
  if (cause.kind === "fields")
    return `The calendar read exceeds its limits: titles up to ${formatQuantity(cause.max_title_bytes)} bytes, labels up to ${formatQuantity(cause.max_label_bytes)} bytes and time zones up to ${formatQuantity(cause.max_timezone_bytes)} bytes. Request a shorter range.`;
  return "The calendar read exceeds a limit this version doesn’t recognize.";
}

export function daemonReply(refusal: Refusal): string {
  const issue = fields(refusal.issue);
  const reason =
    issue.kind === "count_mismatch"
      ? `received ${formatQuantity(issue.delivered)} rows of ${formatQuantity(issue.total)} declared`
      : issue.kind === "cursor_repeated"
        ? "the cursor repeated"
        : issue.kind === "missing"
          ? "the response is missing"
          : "the response couldn’t be parsed";
  return `Orbit’s service response doesn’t match the contract: ${String(refusal.field)}; ${reason}.`;
}
