import { cleanup, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ChatMessage } from "../../bridge/generated/ChatMessage";
import type { Notice } from "../../bridge/generated/Notice";
import type { MessagesPage } from "../../bridge/reads";
import { useHandovers } from "./handover";
import {
  ARES,
  assistantSnapshot,
  at,
  DEIMOS,
  NOW,
  PHOBOS,
  renderAssistant,
} from "./test-assistants";

// The plan's limit (annex 41 and 41a, proposal 30): the question is not
// lost, «Seguir con Deimos» hands the conversation over, «Esperar» sends it
// when the plan comes back, and another assistant asks for its sign-in first.

beforeEach(() => {
  vi.useFakeTimers({ now: NOW, toFake: ["Date"] });
  window.location.hash = "#/assistant/c/s-tarifas";
});

afterEach(() => {
  cleanup();
  useHandovers.setState({ pending: null, from: {}, waiting: {} });
  vi.useRealTimers();
  window.location.hash = "";
});

const RESETS = at(21, 18, 40);
const question: ChatMessage = {
  id: "q1",
  role: "user",
  text: "Compáralas con las de septiembre.",
  at_ms: at(21, 11, 58),
  streaming: false,
  voice: null,
  receipts: [],
};
const limited = { ...PHOBOS, limit: { until_ms: RESETS } };
const notice = {
  code: "assistant_failed",
  op: "op-chat",
  sequence: 4,
  data: { code: "agent_limit_reached", resets_at_ms: String(RESETS) },
} as unknown as Notice;

function threadWith(items: ChatMessage[]) {
  return (): Promise<MessagesPage> =>
    Promise.resolve({
      state: "present",
      value: { items, next: null, total: items.length, revision: 1 },
      as_of_ms: null,
    });
}

function atLimit(agents = [DEIMOS, limited, ARES]) {
  return renderAssistant(
    assistantSnapshot(
      { session_id: "s-tarifas", agent_id: PHOBOS.id, turn: [question] },
      { agents, notice },
    ),
    { messages: threadWith([question]) },
  );
}

describe("an assistant at its plan's limit", () => {
  it.each(["the same Mac next day", "another Mac"])(
    "reads the handover when reopened on %s",
    async () => {
      vi.setSystemTime(NOW + 86_400_000);
      // Neither Mac remembers a handover; only the hub's message read carries it.
      const read = vi.fn(() =>
        Promise.resolve({
          state: "present",
          value: {
            items: [question],
            next: null,
            total: 1,
            revision: 1,
            handed_over_from: { session_id: "s-tarifas", agent_id: PHOBOS.id },
          },
          as_of_ms: null,
        } as MessagesPage),
      );
      renderAssistant(assistantSnapshot({ session_id: "s-nueva", agent_id: DEIMOS.id }), {
        messages: read,
      });
      expect(
        await screen.findByText("Sigues con Deimos · le hemos pasado esta conversación"),
      ).toBeTruthy();
      expect(within(screen.getByTestId("top-bar")).getByText("sigue desde Phobos")).toBeTruthy();
      expect(read).toHaveBeenCalledWith("s-nueva", null);
    },
  );

  it("says so with its time, keeps the question and offers the others", async () => {
    atLimit();
    const card = (
      await screen.findByText("Phobos ha llegado al límite de tu plan de ChatGPT")
    ).closest("[role=alert]") as HTMLElement;
    expect(
      within(card).getByText(
        "Vuelve a estar disponible hoy a las 18:40. Tu pregunta no se ha perdido: puedes seguir con otro asistente, que leerá lo que habéis hablado.",
      ),
    ).toBeTruthy();
    expect(within(card).getByRole("button", { name: "Seguir con Deimos" })).toBeTruthy();
    expect(within(card).getByRole("link", { name: "Ares · inicia sesión primero" })).toBeTruthy();
    expect(within(card).getByRole("button", { name: "Esperar a las 18:40" })).toBeTruthy();
    expect(within(card).getByText("Te proponemos a Deimos porque es tu principal.")).toBeTruthy();
    expect(within(screen.getByRole("log")).getByText(question.text)).toBeTruthy();
    expect(
      screen.getByText("Phobos no puede responder hasta las 18:40. Lo que escribas se queda aquí."),
    ).toBeTruthy();
  });

  it("hands the conversation and its question over to the primary, and a line says so", async () => {
    const { sent, rerender } = atLimit();
    await userEvent.click(await screen.findByRole("button", { name: "Seguir con Deimos" }));
    expect(sent).toContainEqual({
      command: "handover",
      session_id: "s-tarifas",
      agent_id: DEIMOS.id,
      text: "",
    });
    rerender(
      assistantSnapshot(
        { session_id: "s-nueva", agent_id: DEIMOS.id, turn: [question] },
        { agents: [DEIMOS, limited, ARES] },
      ),
    );
    expect(
      await screen.findByText("Sigues con Deimos · le hemos pasado esta conversación"),
    ).toBeTruthy();
    expect(within(screen.getByTestId("top-bar")).getByText("sigue desde Phobos")).toBeTruthy();
  });

  it("waits for the plan and sends the question then", async () => {
    const { sent, rerender } = atLimit();
    await userEvent.click(await screen.findByRole("button", { name: "Esperar a las 18:40" }));
    expect(
      screen.getByText(
        "Te avisamos cuando Phobos vuelva a estar disponible (18:40). Mandaremos tu pregunta entonces.",
      ),
    ).toBeTruthy();
    expect(sent.filter((command) => command.command === "chat")).toEqual([]);
    vi.setSystemTime(RESETS + 1_000);
    // The hub lifts the limit when the plan comes back.
    rerender(
      assistantSnapshot(
        { session_id: "s-tarifas", agent_id: PHOBOS.id, turn: [question] },
        { agents: [DEIMOS, PHOBOS, ARES], notice },
      ),
    );
    await waitFor(() => expect(sent).toContainEqual({ command: "chat", text: question.text }));
  });

  it("goes to the other assistant to sign in first", async () => {
    atLimit();
    await userEvent.click(
      await screen.findByRole("link", { name: "Ares · inicia sesión primero" }),
    );
    expect(window.location.hash).toBe(`#/assistant/${ARES.id}`);
  });

  it("does not offer to hand over on a hub that cannot", async () => {
    renderAssistant(
      assistantSnapshot(
        { session_id: "s-tarifas", agent_id: PHOBOS.id, turn: [question] },
        { agents: [DEIMOS, limited, ARES], notice, addable: null },
      ),
      { messages: threadWith([question]) },
    );
    await screen.findByText("Phobos ha llegado al límite de tu plan de ChatGPT");
    expect(screen.queryByRole("button", { name: "Seguir con Deimos" })).toBeNull();
    expect(screen.getByRole("button", { name: "Esperar a las 18:40" })).toBeTruthy();
  });
});
