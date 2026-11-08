import type { Refusal } from "orbit-ui/bridge/refusal";
import type { NoticeData } from "orbit-ui/i18n";

const TOKEN = /^[a-z0-9_.-]{1,64}$/;

/** Only numbers, booleans and tokens leave for the log; anything else is dropped. */
export function closedData(data: NoticeData | undefined): NoticeData {
  const kept: NoticeData = {};
  for (const [key, value] of Object.entries(data ?? {}) as [keyof NoticeData, unknown][]) {
    if (typeof value === "number" || typeof value === "boolean") kept[key] = value;
    else if (typeof value === "string" && TOKEN.test(value)) kept[key] = value;
  }
  return kept;
}

export function token(code: string): string {
  return TOKEN.test(code) ? code : "unrecognized";
}

/** Which wait a `timeout` refusal names (a code of the daemon's) and its numbers. */
export function timeoutData({ operation, waited_ms, deadline_ms }: Refusal): NoticeData {
  const data: NoticeData = {};
  if (typeof operation === "string") data.operation = operation;
  if (typeof waited_ms === "number") data.elapsed_ms = waited_ms;
  if (typeof deadline_ms === "number") data.timeout_ms = deadline_ms;
  return closedData(data);
}
