import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import type { AppCommand } from "../../bridge/generated";
import type { ChatMessage } from "../../bridge/generated/ChatMessage";
import { MessageList } from "./MessageList";

const copied = vi.hoisted(() => ({ texts: [] as string[] }));
vi.mock("../../bridge/clipboard", () => ({
  copyText: (text: string) => {
    copied.texts.push(text);
    return Promise.resolve();
  },
}));

// An answer opens in the Markdown editor; what comes out of it is Markdown
// again, with its code and its table. Copying or sending it back is the
// user's click; closing it sends nothing.

const ANSWER = [
  "## Plan",
  "",
  "Revisa **esto** y `aquello`.",
  "",
  "- uno",
  "- dos",
  "",
  "```ts",
  "const plazas = 8;",
  "```",
  "",
  "| Sala | Plazas |",
  "| --- | --- |",
  "| Norte | 8 |",
].join("\n");

/** Markdown with table padding and runs of blank lines evened out. */
function sameMarkdown(markdown: string): string {
  return markdown
    .trim()
    .replace(/\n{3,}/g, "\n\n")
    .split("\n")
    .map((line) =>
      line.startsWith("|") ? line.replace(/ +\|/g, " |").replace(/-{3,}/g, "---") : line,
    )
    .join("\n");
}

function answer(text: string): ChatMessage {
  return {
    id: "a1",
    role: "assistant",
    text,
    at_ms: 1,
    streaming: false,
    voice: null,
    receipts: [],
  };
}

async function openEditor(text: string, sent: AppCommand[] = []) {
  render(<MessageList messages={[answer(text)]} send={(command) => sent.push(command)} />);
  await userEvent.click(screen.getByRole("button", { name: "Abrir en el editor" }));
  const dialog = await screen.findByRole("dialog", { name: "Editar la respuesta" });
  const box = await within(dialog).findByRole("textbox", { name: "Respuesta" });
  return { box, dialog };
}

describe("the answer in the editor", () => {
  it("shows it formatted and copies the same Markdown back", async () => {
    copied.texts = [];
    const { box, dialog } = await openEditor(ANSWER);
    await waitFor(() => expect(box.querySelector("h2")?.textContent).toBe("Plan"));
    expect(box.querySelector("pre code")?.textContent).toBe("const plazas = 8;");
    expect(box.querySelector("table")).not.toBeNull();
    await userEvent.click(within(dialog).getByRole("button", { name: "Copiar" }));
    expect(copied.texts).toHaveLength(1);
    // The editor pads table cells to align them and leaves a blank line
    // more before a table: the same Markdown, written its way.
    expect(sameMarkdown(copied.texts[0] ?? "")).toBe(sameMarkdown(ANSWER));
  });

  it("sends the edited text back only when asked, and closing sends nothing", async () => {
    const sent: AppCommand[] = [];
    const { box, dialog } = await openEditor("Hola", sent);
    await waitFor(() => expect(box.textContent).toBe("Hola"));
    await userEvent.click(within(dialog).getByRole("button", { name: "Cerrar" }));
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(sent).toEqual([]);
    await userEvent.click(screen.getByRole("button", { name: "Abrir en el editor" }));
    const again = await screen.findByRole("dialog", { name: "Editar la respuesta" });
    await within(again).findByText("Hola");
    await userEvent.click(within(again).getByRole("button", { name: "Mandar a Deimos" }));
    expect(sent).toEqual([{ command: "chat", text: "Hola" }]);
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("keeps hostile Markdown from becoming elements or links", async () => {
    const { box } = await openEditor(
      '<script>alert(1)</script> [x](javascript:alert(1)) <img src=x onerror="alert(1)">',
    );
    await waitFor(() => expect(box.textContent).not.toBe(""));
    expect(box.querySelector("script, img")).toBeNull();
    expect(box.querySelector("a")).toBeNull();
  });
});
