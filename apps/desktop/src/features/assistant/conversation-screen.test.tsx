import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { AppCommand } from "../../bridge/generated";
import type { Agent } from "../../bridge/generated/Agent";
import type { ChatMessage } from "../../bridge/generated/ChatMessage";
import type { Notice } from "../../bridge/generated/Notice";
import type { MessagesPage } from "../../bridge/reads";
import { AssistantCopy } from "../../i18n/AssistantCopy";
import type { AppView as AppSnapshot } from "../../state/app-view";
import AssistantView from "./AssistantView";
import { conversationAgentName } from "./agents";

vi.mock("../../bridge/clipboard", () => ({ copyText: () => Promise.resolve() }));

// The conversation as the redesign draws it: the agent and what it waits for
// in the header, the thread that loads before it says anything is missing,
// the approval with its four answers, a failed turn with its cause, and a
// composer that keeps what is written while it cannot send.

afterEach(cleanup);

function message(id: string, role: ChatMessage["role"], text: string): ChatMessage {
  return { id, role, text, at_ms: 1_700_000_000_000, streaming: false, voice: null, receipts: [] };
}

function page(items: ChatMessage[]): MessagesPage {
  return {
    state: "present",
    value: { items, next: null, total: items.length, revision: 1 },
    as_of_ms: null,
  };
}

const ONE: Agent[] = [{ id: "deimos", name: "Deimos" }];
const TWO: Agent[] = [
  { id: "deimos", name: "Deimos" },
  { id: "atlas", name: "Atlas" },
];

function snapshot(
  chat: Partial<AppSnapshot["chat"]> = {},
  options: { agents?: Agent[]; paired?: boolean; notice?: Notice | null } = {},
): AppSnapshot {
  const paired = options.paired ?? true;
  return {
    connection: { daemon: "ready", hub: paired ? "paired" : "unpaired" },
    status_availability: { state: "current" },
    call_availability: { state: "current" },
    call: null,
    notice: options.notice ?? null,
    settings: {
      hub: { paired, connection: paired ? "online" : "offline" },
      agents: options.agents ?? ONE,
      default_agent_id: "deimos",
    },
    chat: {
      session_id: "s-1",
      turn: [],
      pending: null,
      voice: { level: 0, state: "idle", availability: { state: "current" } },
      availability: { state: "current" },
      ...chat,
    },
  } as unknown as AppSnapshot;
}

function show(
  snap: AppSnapshot,
  read: (session: string) => Promise<MessagesPage> = () => Promise.resolve(page([])),
) {
  const sent: AppCommand[] = [];
  const view = (current: AppSnapshot) => (
    <AssistantCopy name={conversationAgentName(current)}>
      <AssistantView
        error={null}
        readMessages={read}
        send={(command) => sent.push(command)}
        snapshot={current}
      />
    </AssistantCopy>
  );
  const rendered = render(view(snap));
  return { sent, rerender: (next: AppSnapshot) => rendered.rerender(view(next)) };
}

const approval = {
  kind: "approval" as const,
  id: "ap-1",
  text: "Enviar desde Mail el correo «Arranque del piloto» a pablo@hoteles.example",
  choices: ["once", "session", "always", "deny"],
};

describe("the header", () => {
  it("names the agent and says it waits for permission", async () => {
    show(
      snapshot(
        {
          agent_id: "atlas",
          pending: approval,
          activity: "thinking",
          turn: [message("t1", "user", "Envíaselo a Pablo.")],
        },
        { agents: TWO },
      ),
    );
    const header = screen.getByRole("heading", { name: "Atlas" }).closest("header") as HTMLElement;
    expect(within(header).getByRole("heading", { name: "Atlas" })).toBeTruthy();
    expect(header.textContent).toContain("espera tu permiso");
    expect(within(header).getByRole("button", { name: "Llamar a Atlas" })).toBeTruthy();
    expect(within(header).getByRole("button", { name: "Copiar conversación" })).toBeTruthy();
  });

  it("«Copiar conversación» confirms it was copied", async () => {
    show(snapshot(), () => Promise.resolve(page([message("1", "user", "Hola")])));
    await screen.findByText("Hola");
    await userEvent.click(screen.getByRole("button", { name: "Copiar conversación" }));
    expect(await screen.findByText("✓ Copiada")).toBeTruthy();
  });
});

describe("the thread", () => {
  it("shows the shape of the messages while the page loads, never «Aún no hay mensajes»", () => {
    show(snapshot(), () => new Promise(() => undefined));
    expect(screen.getByRole("log", { busy: true })).toBeTruthy();
    expect(screen.queryByText(/Aún no hay mensajes/)).toBeNull();
  });

  it("an empty new conversation invites to write to its agent", () => {
    show(snapshot({ session_id: null }));
    expect(screen.getByRole("heading", { name: "Pregúntale a Deimos" })).toBeTruthy();
    expect(screen.queryByRole("radiogroup")).toBeNull();
  });

  it("a new conversation with several agents chooses which one", async () => {
    const { sent } = show(snapshot({ session_id: null }, { agents: TWO }));
    const choice = screen.getByRole("radiogroup", { name: "Hablar con" });
    expect(within(choice).getByRole("radio", { name: "Deimos" }).getAttribute("aria-checked")).toBe(
      "true",
    );
    await userEvent.click(within(choice).getByRole("radio", { name: "Atlas" }));
    expect(sent).toEqual([{ command: "new_session", agent_id: "atlas" }]);
  });

  it("labels each answer with its agent, not «ORBIT»", async () => {
    show(snapshot({ agent_id: "atlas" }, { agents: TWO }), () =>
      Promise.resolve(page([message("1", "user", "¿Sala?"), message("2", "assistant", "Norte.")])),
    );
    const answer = (await screen.findByText("Norte.")).closest("article") as HTMLElement;
    expect(answer.getAttribute("aria-label")).toBe("Respuesta de Atlas");
    expect(screen.queryByText("ORBIT")).toBeNull();
  });
});

describe("an approval", () => {
  it("shows what the agent wants with its four answers and remembers the one given", async () => {
    const question = message("t1", "user", "Envíaselo a Pablo.");
    const { sent, rerender } = show(
      snapshot({ turn: [question], pending: approval, activity: "thinking" }),
    );
    const card = screen.getByRole("region", { name: "Deimos necesita tu permiso" });
    expect(card.textContent).toContain("pablo@hoteles.example");
    expect(
      within(card)
        .getAllByRole("button")
        .map((button) => button.textContent),
    ).toEqual(["Permitir una vez", "En esta conversación", "Siempre", "No permitir"]);
    await userEvent.click(within(card).getByRole("button", { name: "No permitir" }));
    expect(sent).toEqual([{ command: "approval", approved: false, choice: "deny" }]);
    rerender(snapshot({ turn: [question], pending: null, activity: "thinking" }));
    expect(screen.getByText("No lo permitiste.")).toBeTruthy();
  });

  it("a clarification asks in the agent's name", () => {
    show(
      snapshot({
        turn: [message("t1", "user", "Prepárame el informe")],
        activity: "thinking",
        pending: {
          kind: "clarify",
          id: "cl-1",
          text: "Antes, una pregunta.",
          choices: ["PDF", "Hoja de cálculo"],
          questions: [],
        },
      }),
    );
    const card = screen.getByRole("region", { name: "Deimos te pregunta" });
    expect(within(card).getByRole("button", { name: "PDF" })).toBeTruthy();
    expect(within(card).getByRole("textbox", { name: "Respuesta" })).toBeTruthy();
  });
});

describe("a turn that failed", () => {
  it("says why under the question and asks it again", async () => {
    const question = message("t1", "user", "Resúmeme todas las conversaciones del mes.");
    const { sent } = show(
      snapshot(
        { turn: [question] },
        {
          notice: {
            code: "assistant_failed",
            op: "op-1",
            sequence: 9,
            data: { code: "assistant_context_exceeded", tokens: 141_203, max_tokens: 131_072 },
          },
        },
      ),
    );
    const failure = await screen.findByRole("alert");
    expect(failure.textContent).toContain("no cabe en el modelo de Deimos");
    await userEvent.click(within(failure).getByRole("button", { name: "Volver a preguntar" }));
    expect(sent).toEqual([{ command: "chat", text: "Resúmeme todas las conversaciones del mes." }]);
  });
});

describe("the composer while the agent answers", () => {
  it("cannot send, says why and keeps the text until it can", async () => {
    const question = message("t1", "user", "Envíaselo a Pablo.");
    const { sent, rerender } = show(
      snapshot({ turn: [question], pending: approval, activity: "thinking" }),
    );
    const box = screen.getByRole("textbox", { name: "Mensaje para Deimos" });
    await userEvent.type(box, "Y compara la ocupación{Enter}");
    expect(sent).toEqual([]);
    expect((box as HTMLTextAreaElement).value).toBe("Y compara la ocupación");
    const send = screen.getByRole("button", { name: "Enviar" });
    expect(send).toHaveProperty("disabled", true);
    expect(
      screen.getByText(
        "Deimos espera tu permiso. Lo que escribas se queda aquí hasta que puedas enviarlo.",
      ),
    ).toBeTruthy();
    rerender(snapshot({ turn: [question] }));
    await act(async () => undefined);
    await userEvent.click(screen.getByRole("button", { name: "Enviar" }));
    expect(sent).toEqual([{ command: "chat", text: "Y compara la ocupación" }]);
  });
});

describe("a Mac that is not paired", () => {
  it("says how to talk to the agent and leads to pairing", () => {
    show(snapshot({ session_id: null }, { paired: false }));
    expect(screen.getByText("Para hablar con Deimos, empareja este Mac con tu hub.")).toBeTruthy();
    expect(screen.getByRole("link", { name: "Emparejar este Mac" }).getAttribute("href")).toBe(
      "#/settings",
    );
  });
});

describe("nothing written is lost", () => {
  function composerWith(answer: (command: AppCommand) => Promise<boolean>) {
    const sent: AppCommand[] = [];
    const send = (command: AppCommand) => {
      sent.push(command);
      return answer(command);
    };
    const view = (error: string | null) => (
      <AssistantView
        error={error}
        readMessages={() => Promise.resolve(page([]))}
        send={send}
        snapshot={snapshot()}
      />
    );
    const rendered = render(view(null));
    const box = screen.getByRole("textbox", { name: "Mensaje para Deimos" }) as HTMLTextAreaElement;
    return { box, sent, showError: (text: string) => rendered.rerender(view(text)) };
  }

  it("a message refused because a turn is under way stays, and the view says why", async () => {
    const { box, sent, showError } = composerWith(async () => false);
    await userEvent.type(box, "Y compara la ocupación");
    await userEvent.click(screen.getByRole("button", { name: "Enviar" }));
    expect(sent).toHaveLength(1);
    // The store shows the refusal of the order in the view that sent it.
    showError("Deimos aún responde a tu pregunta anterior. Vuelve a preguntar cuando acabe.");
    expect(box.value).toBe("Y compara la ocupación");
    expect(screen.getByRole("alert").textContent).toContain("aún responde");
    expect(screen.getByRole("button", { name: "Enviar" })).toHaveProperty("disabled", false);
  });

  it("an oversized message the daemon refuses stays whole in the box", async () => {
    const { box, sent } = composerWith(async () => false);
    const long = "Una línea larga de prueba. ".repeat(1_500);
    fireEvent.change(box, { target: { value: long } });
    await userEvent.click(screen.getByRole("button", { name: "Enviar" }));
    await waitFor(() => expect(sent).toHaveLength(1));
    await act(async () => undefined);
    expect(box.value).toBe(long);
  });

  it("keeps the text while the message travels, and empties only once accepted", async () => {
    let accept: (accepted: boolean) => void = () => undefined;
    const { box } = composerWith(
      () =>
        new Promise<boolean>((resolve) => {
          accept = resolve;
        }),
    );
    await userEvent.type(box, "Hola");
    await userEvent.click(screen.getByRole("button", { name: "Enviar" }));
    expect(box.value).toBe("Hola");
    expect(screen.getByText("Enviando tu mensaje…")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Enviar" })).toHaveProperty("disabled", true);
    await act(async () => accept(true));
    expect(box.value).toBe("");
  });
});
