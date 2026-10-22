import { fireEvent, render, screen, within } from "@testing-library/react";
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

// An answer opens in the editor: its Markdown on the left, how it will look
// on the right as it changes. Copying or sending it back is the user's
// click; closing it sends nothing.

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
  await userEvent.click(screen.getByRole("button", { name: "Editar y copiar" }));
  const dialog = await screen.findByRole("dialog", { name: "Editar la respuesta" });
  const box = within(dialog).getByRole("textbox", { name: "Texto de la respuesta" });
  const preview = within(dialog).getByRole("region", { name: "Así se verá" });
  return { box: box as HTMLTextAreaElement, dialog, preview };
}

describe("the answer in the editor", () => {
  it("shows its Markdown beside how it will look, and copies what is written", async () => {
    copied.texts = [];
    const { box, dialog, preview } = await openEditor(ANSWER);
    expect(box.value).toBe(ANSWER);
    expect(preview.querySelector("h4")?.textContent).toBe("Plan");
    expect(preview.querySelector("pre code")?.textContent).toBe("const plazas = 8;");
    expect(preview.querySelector("table")).not.toBeNull();
    expect(within(dialog).getByText(`${ANSWER.length} caracteres`)).toBeTruthy();
    await userEvent.click(within(dialog).getByRole("button", { name: "Copiar" }));
    expect(copied.texts).toEqual([ANSWER]);
    expect(await within(dialog).findByText("✓ Copiado")).toBeTruthy();
  });

  it("previews each change at once", async () => {
    const { box, preview } = await openEditor("Hola");
    await userEvent.type(box, " **mundo**");
    expect(preview.querySelector("strong")?.textContent).toBe("mundo");
  });

  it("sends the edited text back only when asked, and closing sends nothing", async () => {
    const sent: AppCommand[] = [];
    const { dialog } = await openEditor("Hola", sent);
    await userEvent.click(within(dialog).getByRole("button", { name: "Cerrar" }));
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(sent).toEqual([]);
    await userEvent.click(screen.getByRole("button", { name: "Editar y copiar" }));
    const again = await screen.findByRole("dialog", { name: "Editar la respuesta" });
    await userEvent.click(within(again).getByRole("button", { name: "Mandar a Deimos" }));
    expect(sent).toEqual([{ command: "chat", text: "Hola" }]);
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("closes with Esc", async () => {
    const { dialog } = await openEditor("Hola");
    fireEvent.keyDown(dialog, { key: "Escape" });
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("keeps hostile Markdown from becoming elements or links in the preview", async () => {
    const { preview } = await openEditor(
      '<script>alert(1)</script> [x](javascript:alert(1)) <img src=x onerror="alert(1)">',
    );
    expect(preview.textContent).not.toBe("");
    expect(preview.querySelector("script, img")).toBeNull();
    expect(preview.querySelector("a")).toBeNull();
  });
});
