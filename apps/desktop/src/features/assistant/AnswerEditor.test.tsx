import { act, fireEvent, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import type { AppCommand } from "../../bridge/generated";
import type { ChatMessage } from "../../bridge/generated/ChatMessage";
import { editorNamed, focusAt, typeText } from "../editor/test-editor";
import { MessageList } from "./MessageList";

const copied = vi.hoisted(() => ({ texts: [] as string[] }));
vi.mock("../../bridge/clipboard", () => ({
  copyText: (text: string) => {
    copied.texts.push(text);
    return Promise.resolve();
  },
}));

// An answer opens in the notes' editor, in a window of its own: changed
// there, it is copied as Markdown or sent to the agent as a new message.
// Copying or sending is the user's click; closing it sends nothing.

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

type Sender = (command: AppCommand) => unknown;

async function openEditor(text: string, send: Sender = () => true) {
  render(<MessageList messages={[answer(text)]} send={send} />);
  await userEvent.click(screen.getByRole("button", { name: "Editar y copiar" }));
  const dialog = await screen.findByRole("dialog", { name: "Editar la respuesta" });
  return { dialog, editor: editorNamed("Texto de la respuesta") };
}

describe("the answer in the editor", () => {
  it("is one editor holding its Markdown as it came, and copies what is written", async () => {
    copied.texts = [];
    const { dialog, editor } = await openEditor(ANSWER);
    expect(editor.state.doc.toString()).toBe(ANSWER);
    expect(within(dialog).queryByRole("region", { name: "Así se verá" })).toBeNull();
    expect(
      within(dialog).getByText("Los cambios solo están aquí hasta que copies o mandes"),
    ).toBeTruthy();
    expect(
      within(dialog).getByText(`${ANSWER.length} caracteres · se copia como Markdown`),
    ).toBeTruthy();
    await userEvent.click(within(dialog).getByRole("button", { name: "Copiar" }));
    expect(copied.texts).toEqual([ANSWER]);
    expect(await within(dialog).findByText("✓ Copiado")).toBeTruthy();
  });

  it("copies and sends the text as changed in it", async () => {
    copied.texts = [];
    const sent: AppCommand[] = [];
    const { dialog, editor } = await openEditor("Hola", (command) => {
      sent.push(command);
      return Promise.resolve(true);
    });
    focusAt(editor);
    typeText(editor, " **mundo**");
    await userEvent.click(within(dialog).getByRole("button", { name: "Copiar" }));
    expect(copied.texts).toEqual(["Hola **mundo**"]);
    await userEvent.click(within(dialog).getByRole("button", { name: "Mandar a Deimos" }));
    expect(sent).toEqual([{ command: "chat", text: "Hola **mundo**" }]);
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("stays open with what was written when the message is refused", async () => {
    const { dialog, editor } = await openEditor("Hola", () => Promise.resolve(false));
    focusAt(editor);
    typeText(editor, " otra vez");
    await act(async () => {
      fireEvent.click(within(dialog).getByRole("button", { name: "Mandar a Deimos" }));
    });
    expect(screen.getByRole("dialog", { name: "Editar la respuesta" })).toBeTruthy();
    expect(editorNamed("Texto de la respuesta").state.doc.toString()).toBe("Hola otra vez");
  });

  it("closing sends nothing", async () => {
    const sent: AppCommand[] = [];
    const { dialog } = await openEditor("Hola", (command) => sent.push(command));
    await userEvent.click(within(dialog).getByRole("button", { name: "Cerrar" }));
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(sent).toEqual([]);
  });

  it("closes with Esc", async () => {
    const { dialog } = await openEditor("Hola");
    fireEvent.keyDown(dialog, { key: "Escape" });
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("keeps hostile Markdown as text, never elements or links", async () => {
    const hostile =
      '<script>alert(1)</script> [x](javascript:alert(1)) <img src=x onerror="alert(1)">';
    const { dialog, editor } = await openEditor(hostile);
    expect(editor.state.doc.toString()).toBe(hostile);
    expect(dialog.querySelector("script, img")).toBeNull();
    expect(editor.contentDOM.querySelector("a")).toBeNull();
  });
});

describe("an answer being changed when Orbit quits or falls", () => {
  const KEY = "orbit.draft.answer.a1";

  it("is kept on this Mac as it is changed, and let go of when the window is closed", async () => {
    const { dialog, editor } = await openEditor("Hola");
    focusAt(editor);
    typeText(editor, " Bruno");
    expect(JSON.parse(window.localStorage.getItem(KEY) ?? "null")).toEqual({
      from: "Hola",
      text: "Hola Bruno",
    });
    await userEvent.click(within(dialog).getByRole("button", { name: "Cerrar" }));
    expect(window.localStorage.getItem(KEY)).toBeNull();
  });

  it("comes back, and says so, when the answer is opened again after Orbit fell", async () => {
    window.localStorage.setItem(KEY, JSON.stringify({ from: "Hola", text: "Hola, Celia" }));
    vi.resetModules();
    const { MessageList: Reopened } = await import("./MessageList");
    render(<Reopened messages={[answer("Hola")]} send={() => true} />);
    await userEvent.click(screen.getByRole("button", { name: "Editar y copiar" }));
    const dialog = await screen.findByRole("dialog", { name: "Editar la respuesta" });
    expect(within(dialog).getByRole("textbox", { name: "Texto de la respuesta" }).textContent).toBe(
      "Hola, Celia",
    );
    expect(
      within(dialog).getByText("Recuperado lo que escribiste y no se llegó a guardar."),
    ).toBeTruthy();
  });

  it("is let go of once the message was taken", async () => {
    const { dialog, editor } = await openEditor("Hola", () => Promise.resolve(true));
    focusAt(editor);
    typeText(editor, "!");
    await userEvent.click(within(dialog).getByRole("button", { name: "Mandar a Deimos" }));
    expect(window.localStorage.getItem(KEY)).toBeNull();
  });
});
