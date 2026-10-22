import { noticeClass } from "../../../bridge/generated/noticeClasses";
import { assistantFailed } from "./assistantFailed";
import { capture } from "./capture";
import { captureFailed } from "./captureFailed";
import { conversationCopy } from "./conversation";
import { delivery, uploadRefused } from "./delivery";
import { system } from "./system";
import type { Entries, WordedCode } from "./types";

export type { NoticeAction, NoticeData } from "./types";

// Every visible code needs words; silent and failed codes need only Rust's class.

const classes = Object.fromEntries(
  Object.entries(noticeClass).map(([code, kind]) => [code, { class: kind }]),
);

// Object.fromEntries loses the relation between a code and its class.
export function noticesForAssistant(assistantName = "Deimos"): Entries {
  const worded: Entries<WordedCode> = {
    ...capture,
    ...conversationCopy(assistantName),
    ...system,
    ...delivery,
  };
  return {
    ...classes,
    ...worded,
    ...captureFailed,
    ...assistantFailed(assistantName),
    ...uploadRefused,
  } as Entries;
}

export const notices = noticesForAssistant();
