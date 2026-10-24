import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import type { AppCommand } from "../../bridge/generated";
import type { ChatMessage } from "../../bridge/generated/ChatMessage";
import type { AssistantCursor, MessagesPage } from "../../bridge/reads";
import type { AppView as AppSnapshot } from "../../state/app-view";
import AssistantView from "./AssistantView";

const copied = vi.hoisted(() => ({ texts: [] as string[] }));
vi.mock("../../bridge/clipboard", () => ({
  copyText: (text: string) => {
    copied.texts.push(text);
    return Promise.resolve();
  },
}));

// Each answer can be copied, retried, heard and opened in the editor; each
// message of the user copied; the whole conversation copied as Markdown, and
// a fragment of an answer quoted in the composer. Nothing is sent by itself.

function message(id: string, role: ChatMessage["role"], text: string): ChatMessage {
  return { id, role, text, at_ms: 1_700_000_000_000, streaming: false, voice: null, receipts: [] };
}

function page(items: ChatMessage[], next: AssistantCursor | null = null): MessagesPage {
  return {
    state: "present",
    value: { items, next, total: 4, revision: 1 },
    as_of_ms: null,
  };
}

function snapshot(chat: Partial<AppSnapshot["chat"]> = {}): AppSnapshot {
  return {
    connection: { daemon: "ready", hub: "paired" },
    status_availability: { state: "current" },
    call_availability: { state: "current" },
    call: null,
    settings: { hub: { paired: true, connection: "online" } },
    chat: {
      session_id: "session-ficticia",
      turn: [],
      pending: null,
      voice: { level: 0, state: "idle", availability: { state: "current" } },
      availability: { state: "current" },
      ...chat,
    },
  } as unknown as AppSnapshot;
}

const older = [message("1", "user", "¿Qué sala?"), message("2", "assistant", "La **Norte**.")];
const newest = [
  message("3", "user", "¿Y el código?"),
  message("4", "assistant", "Así:\n\n```sh\npnpm test\n```\n\nLa **Sur** tiene 12 plazas."),
];

function show(chat: Partial<AppSnapshot["chat"]> = {}) {
  const sent: AppCommand[] = [];
  const read = vi.fn((_session: string, cursor: AssistantCursor | null) =>
    Promise.resolve(cursor === null ? page(newest, { kind: "index", index: 2 }) : page(older)),
  );
  render(
    <AssistantView
      error={null}
      readMessages={read}
      send={(command) => sent.push(command)}
      snapshot={snapshot(chat)}
    />,
  );
  return { read, sent };
}

async function answer(text: string) {
  return (await screen.findByText(text)).closest("article") as HTMLElement;
}

describe("copying", () => {
  it("copies an answer as its Markdown and a message of the user as written", async () => {
    copied.texts = [];
    show();
    const last = await answer("Sur");
    await userEvent.click(within(last).getByRole("button", { name: "Copiar respuesta" }));
    const question = (await screen.findByText("¿Y el código?")).closest("article") as HTMLElement;
    await userEvent.click(within(question).getByRole("button", { name: "Copiar mensaje" }));
    expect(copied.texts).toEqual([newest[1]!.text, "¿Y el código?"]);
    expect(within(last).getByRole("button", { name: "Copiado" })).toBeTruthy();
  });

  it("copies the whole conversation from the hub, older pages too", async () => {
    copied.texts = [];
    const { read } = show();
    await answer("Sur");
    await userEvent.click(screen.getByRole("button", { name: "Copiar conversación" }));
    await waitFor(() => expect(copied.texts).toHaveLength(1));
    expect(read).toHaveBeenCalledWith("session-ficticia", { kind: "index", index: 2 });
    expect(copied.texts[0]).toBe(
      "## Tú\n\n¿Qué sala?\n\n## Deimos\n\nLa **Norte**.\n\n" +
        `## Tú\n\n¿Y el código?\n\n## Deimos\n\n${newest[1]!.text}\n`,
    );
    expect(await screen.findByText("✓ Copiada")).toBeTruthy();
  });

  it("copies the last code block with ⌘⇧C", async () => {
    copied.texts = [];
    show();
    await answer("Sur");
    fireEvent.keyDown(window, { key: "c", code: "KeyC", metaKey: true, shiftKey: true });
    await waitFor(() => expect(copied.texts).toEqual(["pnpm test"]));
  });
});

describe("under an answer", () => {
  it("only icons, each named by the tooltip it shows on hover and on focus", async () => {
    show();
    const last = await answer("Sur");
    const tools = ["Copiar", "Escuchar", "Volver a preguntar", "Editar y copiar"];
    const buttons = tools.map(
      (tip) =>
        within(last).getByRole("button", {
          name: tip === "Copiar" ? "Copiar respuesta" : tip,
        }) as HTMLElement,
    );
    for (const [index, button] of buttons.entries()) {
      expect(button.textContent, tools[index]).toBe("");
      expect(button.querySelector("svg"), tools[index]).not.toBeNull();
      fireEvent.mouseEnter(button.parentElement as HTMLElement);
      // The tip is drawn over the whole window, outside the answer.
      expect(screen.getByText(tools[index]!, { selector: "[data-tooltip]" })).toBeTruthy();
      fireEvent.mouseLeave(button.parentElement as HTMLElement);
    }
    fireEvent.focus(buttons[3]!);
    expect(screen.getByText("Editar y copiar", { selector: "[data-tooltip]" })).toBeTruthy();
  });

  it("Copiar turns to ✓ «Copiado» and says it once", async () => {
    copied.texts = [];
    show();
    const last = await answer("Sur");
    await userEvent.click(within(last).getByRole("button", { name: "Copiar respuesta" }));
    const done = within(last).getByRole("button", { name: "Copiado" });
    expect(done.getAttribute("data-echo")).toBe("done");
    expect(within(done).getByRole("status").textContent).toBe("Copiado");
  });
});

describe("retrying and listening", () => {
  it("retries an answer by sending the user's message again as a new turn", async () => {
    const { sent } = show();
    const last = await answer("Sur");
    await userEvent.click(within(last).getByRole("button", { name: "Volver a preguntar" }));
    expect(sent).toEqual([{ command: "chat", text: "¿Y el código?" }]);
  });

  it("offers no asking again while a turn is in flight", async () => {
    show({ activity: "thinking" });
    const last = await answer("Sur");
    expect(within(last).getByRole("button", { name: "Volver a preguntar" })).toHaveProperty(
      "disabled",
      true,
    );
  });

  it("reads an answer aloud through the daemon, and stops it", async () => {
    const { sent } = show();
    const last = await answer("Sur");
    await userEvent.click(within(last).getByRole("button", { name: "Escuchar" }));
    expect(sent).toEqual([
      { command: "speak", message_id: "4", text: "Así:\npnpm test\nLa Sur tiene 12 plazas." },
    ]);
  });
});

describe("what is being read aloud", () => {
  it("shows «Parar» on the answer being read, and stops it", async () => {
    const { sent } = show({ speaking: "4" } as Partial<AppSnapshot["chat"]>);
    const last = await answer("Sur");
    const stop = within(last).getByRole("button", { name: "Parar" });
    expect(stop.getAttribute("aria-pressed")).toBe("true");
    await userEvent.click(stop);
    expect(sent).toEqual([{ command: "speak_stop" }]);
    expect(within(last).queryByRole("button", { name: "Escuchar" })).toBeNull();
  });
});

describe("replying to a fragment", () => {
  it("quotes the selected fragment in the composer without sending it", async () => {
    const { sent } = show();
    const last = await answer("Sur");
    const bold = within(last).getByText("Sur");
    const range = document.createRange();
    range.selectNodeContents(bold.parentElement as HTMLElement);
    const selection = window.getSelection();
    selection?.removeAllRanges();
    selection?.addRange(range);
    fireEvent.mouseUp(bold);
    await userEvent.click(await within(last).findByRole("button", { name: "Responder a esto" }));
    const composer = screen.getByRole("textbox", {
      name: "Mensaje para Deimos",
    }) as HTMLTextAreaElement;
    expect(composer.value).toBe("> La Sur tiene 12 plazas.\n\n");
    expect(sent).toEqual([]);
  });
});
