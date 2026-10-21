import type { ChatMessage } from "./generated/ChatMessage";
import type { ChatSessionSummary } from "./generated/ChatSessionSummary";
import type { Refusal } from "./refusal";

export type AssistantCursor = { kind: "id"; id: string } | { kind: "index"; index: number };

export type ReadPage<T> =
  | {
      state: "present";
      value: { items: T[]; next: AssistantCursor | null; total: number; revision: number };
      as_of_ms: number | null;
    }
  | { state: "absent"; refusal: Refusal }
  | { state: "unknown"; why: unknown };

export type SessionPage = ReadPage<ChatSessionSummary>;
/** One page of a conversation, read from the hub. */
export type MessagesPage = ReadPage<ChatMessage>;
