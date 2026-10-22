import { useEffect } from "react";
import { create } from "zustand";
import type { ChatMessage } from "../../bridge/generated/ChatMessage";

/**
 * The first sentence of what the user wrote, as the list of conversations
 * shows it under its date: up to the first full stop, question or
 * exclamation mark, on one line. A voice note gives its transcript.
 */
export function firstSentence(text: string): string {
  const line = text.trim().split("\n")[0]?.trim() ?? "";
  const end = line.search(/[.?!…](\s|$)/);
  return end === -1 ? line : line.slice(0, end + 1);
}

/** What the user first said in a conversation, if it is among `messages`. */
export function openingLine(messages: ChatMessage[]): string | null {
  const first = messages.find((message) => message.role === "user");
  if (!first) return null;
  const text = first.voice ? (first.voice.transcript ?? "") : first.text;
  const sentence = firstSentence(text);
  return sentence === "" ? null : sentence;
}

/**
 * The first sentences known on this Mac, by conversation: the hub lists a
 * conversation by its dates and counts only, so its first sentence is known
 * once its start has been read in this run. Kept in memory, never written.
 */
const useFirstLinesStore = create<{ lines: Record<string, string> }>(() => ({ lines: {} }));

export function rememberFirstLine(sessionId: string, text: string): void {
  const line = firstSentence(text);
  if (line === "" || useFirstLinesStore.getState().lines[sessionId] === line) return;
  useFirstLinesStore.setState((state) => ({ lines: { ...state.lines, [sessionId]: line } }));
}

export function useFirstLines(): Record<string, string> {
  return useFirstLinesStore((state) => state.lines);
}

/**
 * Remembers what the user first said in `sessionId` once `start` (the
 * thread, when it holds the conversation's start) has it.
 */
export function useRememberedOpening(sessionId: string | null, start: ChatMessage[] | null): void {
  const opening = start === null ? null : openingLine(start);
  useEffect(() => {
    if (sessionId !== null && opening !== null) rememberFirstLine(sessionId, opening);
  }, [opening, sessionId]);
}
