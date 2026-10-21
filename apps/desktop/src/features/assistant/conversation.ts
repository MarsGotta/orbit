import type { AppView as AppSnapshot } from "../../state/app-view";
import type { ChatMessage } from "../../bridge/generated/ChatMessage";

/** A turn is in flight while Deimos thinks or answers it. */
export function turnInFlight(chat: AppSnapshot["chat"]): boolean {
  return chat.activity === "thinking" || chat.activity === "responding";
}

/**
 * What the thread shows. `stored` is the conversation as the hub
 * keeps it (the pages read, in order); `turn` the turn in progress, or the
 * last one, as the assistant topic carries it.
 *
 * The turn sits after what the hub holds before it. Once it has ended and
 * the page read again holds what followed the user's message, the hub's rows are
 * the truth and the turn steps aside: until then it stays in view, so a
 * finished answer never blinks out while its page is being read.
 */
export function conversationView(
  stored: ChatMessage[],
  turn: ChatMessage[],
  streaming: boolean,
): ChatMessage[] {
  const question = turn.find((message) => message.role === "user");
  const at = question ? stored.findIndex((message) => message.id === question.id) : -1;
  if (at === -1) return [...stored, ...turn];
  if (!streaming && at < stored.length - 1) return stored;
  return [...stored.slice(0, at), ...turn];
}

/** `older` before `known`, each message once. */
export function withOlder(known: ChatMessage[], older: ChatMessage[]): ChatMessage[] {
  const ids = new Set(known.map((message) => message.id));
  return [...older.filter((message) => !ids.has(message.id)), ...known];
}

/** The newest page read again replaces what it holds; what came before stays. */
export function withNewest(known: ChatMessage[], newest: ChatMessage[]): ChatMessage[] {
  const ids = new Set(newest.map((message) => message.id));
  return [...known.filter((message) => !ids.has(message.id)), ...newest];
}
