import { noticeClass } from "../../../bridge/generated/noticeClasses";
import { capture } from "./capture";
import { conversationCopy } from "./conversation";
import { system } from "./system";
import type { Entries, WordedCode } from "./types";

export type { NoticeAction, NoticeData } from "./types";

// Every visible code needs words; silent and failed codes need only Rust's class.

const classes = Object.fromEntries(
  Object.entries(noticeClass).map(([code, kind]) => [code, { class: kind }]),
);

// Object.fromEntries loses the relation between a code and its class.
export function noticesForAssistant(assistantName = "Deimos"): Entries {
  const worded: Entries<WordedCode> = { ...capture, ...conversationCopy(assistantName), ...system };
  return { ...classes, ...worded } as Entries;
}

export const notices = noticesForAssistant();
