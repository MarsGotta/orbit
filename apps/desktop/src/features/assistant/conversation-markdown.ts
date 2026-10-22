import type { ChatMessage } from "../../bridge/generated/ChatMessage";

/** What a message says: a voice note by its transcript. */
function said(message: ChatMessage): string {
  return message.voice ? (message.voice.transcript ?? "") : message.text;
}

export interface SpeakerNames {
  user: string;
  assistant: string;
  voiceNote: string;
}

/** The conversation as Markdown: who spoke as a heading, each message as written. */
export function conversationMarkdown(messages: ChatMessage[], names: SpeakerNames): string {
  return messages
    .filter((message) => message.role !== "system")
    .map((message) => {
      const who = message.role === "user" ? names.user : names.assistant;
      const heading = message.voice ? `${who} ${names.voiceNote}` : who;
      return `## ${heading}\n\n${said(message).trim()}\n`;
    })
    .join("\n");
}

const FENCE = /^ {0,3}(`{3,}|~{3,})/;

/**
 * The fenced code blocks of `markdown`, in order. A block closes on a fence
 * of its own character at least as long as the one that opened it, as in
 * CommonMark; one still open at the end (an answer cut short) ends there.
 */
export function codeBlocks(markdown: string): string[] {
  const blocks: string[] = [];
  let open: { fence: string; lines: string[] } | null = null;
  for (const line of markdown.split("\n")) {
    const fence = FENCE.exec(line)?.[1];
    if (open === null) {
      if (fence !== undefined) open = { fence, lines: [] };
    } else if (
      fence !== undefined &&
      fence[0] === open.fence[0] &&
      fence.length >= open.fence.length &&
      line.trim() === fence
    ) {
      blocks.push(open.lines.join("\n"));
      open = null;
    } else {
      open.lines.push(line);
    }
  }
  if (open !== null) blocks.push(open.lines.join("\n"));
  return blocks;
}

/** The last code block of the newest answer that has one (⌘⇧C). */
export function lastCodeBlock(messages: ChatMessage[]): string | null {
  for (const message of [...messages].reverse()) {
    if (message.role !== "assistant") continue;
    const last = codeBlocks(message.text).at(-1);
    if (last !== undefined) return last;
  }
  return null;
}

/** A fragment of an answer as a Markdown quote, with a blank line to write after it. */
export function quoteFragment(fragment: string): string {
  const lines = fragment.trim().split("\n");
  return `${lines.map((line) => `> ${line}`).join("\n")}\n\n`;
}

/** What «Reintentar» sends for the answer at `index`: the user's message before it. */
export function retryText(messages: ChatMessage[], index: number): string | null {
  for (let at = index - 1; at >= 0; at -= 1) {
    const message = messages[at];
    if (message?.role !== "user") continue;
    const text = said(message).trim();
    return text === "" ? null : text;
  }
  return null;
}
