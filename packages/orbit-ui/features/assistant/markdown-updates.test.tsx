import { render, screen } from "@testing-library/react";
import { expect, it, vi } from "vitest";
import type { ChatMessage } from "../../bridge/generated/ChatMessage";
import { CopyProvider } from "../../i18n/CopyProvider";
import { MessageList } from "./MessageList";

// Instrument the public Markdown parser boundary while running the real parser.
const parsed = vi.hoisted(() => [] as string[]);
vi.mock("react-markdown", async (original) => {
  const actual = await original<typeof import("react-markdown")>();
  return {
    ...actual,
    default: (props: Parameters<typeof actual.default>[0]) => {
      parsed.push(props.children ?? "");
      return actual.default(props);
    },
  };
});

it("does not parse old answers on unrelated frames, but updates changed inputs and language", () => {
  const text = "| Room | Seats |\n| --- | --- |\n| North | 8 |";
  const answer = (source: string, streaming = false): ChatMessage => ({
    id: "answer",
    role: "assistant",
    text: source,
    at_ms: 1,
    streaming,
    voice: null,
    receipts: [],
  });
  const view = (language: "en" | "es", source = text, streaming = false) => (
    <CopyProvider choice={language}>
      <MessageList messages={[answer(source, streaming)]} send={() => true} />
    </CopyProvider>
  );
  parsed.length = 0;
  const shown = render(view("es"));
  expect(screen.getByRole("button", { name: "Copiar tabla" })).toBeTruthy();
  const first = parsed.length;
  shown.rerender(view("es"));
  shown.rerender(view("es"));
  expect(parsed.length).toBe(first);
  shown.rerender(view("en"));
  expect(screen.getByRole("button", { name: "Copy table" })).toBeTruthy();
  shown.rerender(view("en", `${text}\n\nUpdated`, true));
  expect(screen.getByText("Updated")).toBeTruthy();
  expect(parsed.at(-1)).toContain("Updated");
  const streaming = parsed.length;
  shown.rerender(view("en", `${text}\n\nUpdated`, false));
  expect(parsed.length).toBeGreaterThan(streaming);
});
