import { describe, expect, it } from "vitest";
import type { ChatMessage } from "../../bridge/generated/ChatMessage";
import {
  conversationMarkdown,
  lastCodeBlock,
  quoteFragment,
  retryText,
} from "./conversation-markdown";

function message(id: string, role: ChatMessage["role"], text: string): ChatMessage {
  return { id, role, text, at_ms: 1, streaming: false, voice: null, receipts: [] };
}

const names = { user: "Tú", assistant: "Deimos", voiceNote: "(nota de voz)" };

describe("the conversation as Markdown", () => {
  it("names who said each message and keeps each one as written", () => {
    const messages = [
      message("1", "user", "¿Qué sala hay libre?"),
      message("2", "assistant", "La **Norte**.\n\n- 8 plazas"),
    ];
    expect(conversationMarkdown(messages, names)).toBe(
      "## Tú\n\n¿Qué sala hay libre?\n\n## Deimos\n\nLa **Norte**.\n\n- 8 plazas\n",
    );
  });

  it("writes a voice note by its transcript", () => {
    const voice: ChatMessage = {
      ...message("1", "user", ""),
      voice: { duration_ms: 2_000, transcript: "Llama a Ana Prueba" },
    };
    expect(conversationMarkdown([voice], names)).toBe(
      "## Tú (nota de voz)\n\nLlama a Ana Prueba\n",
    );
  });
});

describe("the last code block", () => {
  it("is the last fenced block of the newest answer that has one", () => {
    const messages = [
      message("1", "assistant", "```sh\nprimero\n```"),
      message("2", "assistant", "```py\nuno = 1\n```\n\ntexto\n\n~~~\ndos\n~~~"),
      message("3", "user", "```\nde la usuaria\n```"),
      message("4", "assistant", "Sin código."),
    ];
    expect(lastCodeBlock(messages)).toBe("dos");
  });

  it("keeps a longer fence's inner backticks and ends a block left open at the end", () => {
    expect(lastCodeBlock([message("1", "assistant", "````md\n```\nx\n```\n````")])).toBe(
      "```\nx\n```",
    );
    expect(lastCodeBlock([message("1", "assistant", "```ts\nlet a = 1;")])).toBe("let a = 1;");
  });

  it("is null when no answer has code", () => {
    expect(lastCodeBlock([message("1", "assistant", "Nada")])).toBeNull();
  });
});

describe("replying to a fragment and retrying", () => {
  it("quotes every line of the fragment and leaves a blank line to write after it", () => {
    expect(quoteFragment("  La sala Norte\ntiene 8 plazas  ")).toBe(
      "> La sala Norte\n> tiene 8 plazas\n\n",
    );
  });

  it("retries an answer with the user's message just before it", () => {
    const voice: ChatMessage = {
      ...message("3", "user", ""),
      voice: { duration_ms: 1_000, transcript: "Dímelo otra vez" },
    };
    const messages = [
      message("1", "user", "Primera"),
      message("2", "assistant", "Uno"),
      voice,
      message("4", "assistant", "Dos"),
    ];
    expect(retryText(messages, 1)).toBe("Primera");
    expect(retryText(messages, 3)).toBe("Dímelo otra vez");
    expect(retryText([message("9", "assistant", "Sin pregunta")], 0)).toBeNull();
  });
});
