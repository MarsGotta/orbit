import type { NoticeData } from "../i18n/es/notices";

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
