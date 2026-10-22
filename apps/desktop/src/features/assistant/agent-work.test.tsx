import { act, cleanup, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { AppBootstrap, AppCommand, FeatureFrame } from "../../bridge/generated";
import type { AgentWork } from "../../bridge/generated/AgentWork";
import type { ChatMessage } from "../../bridge/generated/ChatMessage";
import type { ChatPending } from "../../bridge/generated/ChatPending";
import type { WorkItem } from "../../bridge/generated/WorkItem";
import type { DaemonPort } from "../../bridge/port";
import { PanelApp } from "../call-panel/PanelApp";
import { AgentWorkView, workSummary } from "./AgentWork";
import { MessageList } from "./MessageList";
import { PendingCard } from "./PendingCard";

// What the agent does in a turn (hub-v1 chat.item), and its questions with
// the choices it offered.

afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

function tool(id: string, phase: WorkItem["phase"], label: string, subject: string): WorkItem {
  return { id, kind: "tool", phase, label, subject, steps: [] };
}

function work(partial: Partial<AgentWork>): AgentWork {
  return {
    turn_id: "turno-1",
    started_at_ms: 1_000,
    items: [],
    tools: 0,
    subjects: [],
    ...partial,
  };
}

const question: ChatMessage = {
  id: "turno-1",
  role: "user",
  text: "¿Qué tengo mañana?",
  at_ms: 1_000,
  streaming: false,
  voice: null,
  receipts: [],
};

describe("the agent's work in the chat", () => {
  it("shows a line per running tool, the plan with each step's state and the time so far", () => {
    vi.useFakeTimers({ now: 41_000 });
    render(
      <AgentWorkView
        inFlight
        work={work({
          items: [
            tool("a", "completed", "Mirando la pantalla", "la pantalla"),
            tool("b", "started", "Leyendo el calendario", "el calendario"),
            {
              id: "plan",
              kind: "plan",
              phase: "progress",
              label: "Plan · 1 de 2 pasos",
              steps: [
                { text: "Mirar la pantalla", status: "completed" },
                { text: "Leer el calendario", status: "in_progress" },
              ],
            },
          ],
          tools: 1,
          subjects: ["la pantalla", "el calendario"],
        })}
      />,
    );
    expect(screen.getByText("Trabajando 40 s")).toBeTruthy();
    expect(screen.getByText("Leyendo el calendario")).toBeTruthy();
    expect(screen.queryByText("Mirando la pantalla")).toBeNull();
    const plan = screen.getByRole("list", { name: "Plan · 1 de 2 pasos" });
    expect(plan.textContent).toContain("Mirar la pantalla");
    expect(screen.getByText("Hecho")).toBeTruthy();
    expect(screen.getByText("En curso")).toBeTruthy();
  });

  it("folds into one summary when the turn ends, with the reasoning folded too", () => {
    const ended = work({
      ended_at_ms: 73_000,
      items: [
        { id: "r", kind: "reasoning", phase: "completed", label: "Comparo dos fechas", steps: [] },
        tool("a", "completed", "Mirando la pantalla", "la pantalla"),
        tool("b", "completed", "Leyendo el calendario", "el calendario"),
        {
          ...tool("c", "failed", "Leyendo el calendario", "el calendario"),
          cause: "Sin permiso de calendario",
        },
      ],
      tools: 3,
      subjects: ["la pantalla", "el calendario"],
    });
    expect(workSummary(ended)).toBe(
      "Usó la pantalla y el calendario · 3 pasos · Trabajó 1 min 12 s",
    );
    const { container } = render(<AgentWorkView inFlight={false} work={ended} />);
    const summary = screen.getByText(
      "Usó la pantalla y el calendario · 3 pasos · Trabajó 1 min 12 s",
    );
    expect(summary.closest("details")?.open).toBe(false);
    expect(screen.getByText("Razonamiento").closest("details")?.open).toBe(false);
    expect(container.textContent).toContain("Sin permiso de calendario");
  });

  it("says only how long a turn without steps worked", () => {
    expect(workSummary(work({ ended_at_ms: 6_000 }))).toBe("Trabajó 5 s");
    expect(workSummary(work({ ended_at_ms: 121_000, tools: 1, subjects: ["web_search"] }))).toBe(
      "Usó web_search · 1 paso · Trabajó 2 min",
    );
  });

  it("sits after the question of its own turn in the thread", () => {
    render(
      <MessageList
        messages={[question]}
        send={() => undefined}
        work={work({ ended_at_ms: 6_000 })}
        workInFlight={false}
      />,
    );
    const said = screen.getByText("¿Qué tengo mañana?").closest("article");
    const worked = screen.getByText("Trabajó 5 s").closest("section");
    expect(said?.compareDocumentPosition(worked as Node)).toBe(Node.DOCUMENT_POSITION_FOLLOWING);
  });
});

describe("questions and approvals with the agent's real choices", () => {
  const approval: ChatPending = {
    kind: "approval",
    id: "permiso-1",
    text: "Borrar la carpeta de pruebas",
    choices: ["once", "deny", "session", "always"],
  };

  it("offers one button per choice and answers the one chosen", async () => {
    const send = vi.fn<(command: AppCommand) => void>();
    render(<PendingCard pending={approval} send={send} />);
    const buttons = screen.getAllByRole("button").map((button) => button.textContent);
    expect(buttons).toEqual([
      "Permitir una vez",
      "Rechazar",
      "Permitir en esta conversación",
      "Permitir siempre",
    ]);
    await userEvent.click(screen.getByRole("button", { name: "Permitir en esta conversación" }));
    await userEvent.click(screen.getByRole("button", { name: "Rechazar" }));
    expect(send.mock.calls).toEqual([
      [{ command: "approval", approved: true, choice: "session" }],
      [{ command: "approval", approved: false, choice: "deny" }],
    ]);
  });

  it("asks one question at a time by its id, with its choices and free text", async () => {
    const send = vi.fn<(command: AppCommand) => void>();
    const clarify: ChatPending = {
      kind: "clarify",
      id: "aclarar",
      text: "Dos preguntas",
      choices: [],
      questions: [
        { qid: "sala", question: "¿Qué sala?", choices: ["Norte", "Sur"], multi_select: false },
        { qid: "hora", question: "¿A qué hora?", choices: [], multi_select: false },
      ],
    };
    render(<PendingCard pending={clarify} send={send} />);
    expect(screen.getByText("¿Qué sala?")).toBeTruthy();
    expect(screen.queryByText("¿A qué hora?")).toBeNull();
    expect(screen.getByText("Pregunta 1 de 2")).toBeTruthy();
    await userEvent.click(screen.getByRole("button", { name: "Norte" }));
    await userEvent.type(screen.getByRole("textbox", { name: "Respuesta" }), "La grande");
    await userEvent.click(screen.getByRole("button", { name: "Responder" }));
    expect(send.mock.calls).toEqual([
      [{ command: "clarify", answer: "Norte", question_id: "sala" }],
      [{ command: "clarify", answer: "La grande", question_id: "sala" }],
    ]);
  });

  it("is answered from the floating panel while a meeting question waits for it", async () => {
    const sent: AppCommand[] = [];
    const frame = (feature: string, view: unknown) =>
      ({ feature, revision: { epoch: 1, counter: 1 }, view }) as unknown as FeatureFrame;
    const port = {
      subscribeFrames: async () => () => undefined,
      snapshot: async () =>
        ({
          revision: { epoch: 1, counter: 1 },
          frames: [
            frame("status", { settings: {}, notices: [], notice: null }),
            ...["local", "permissions", "agenda", "notes", "note", "control", "briefing"].map(
              (feature) => frame(feature, {}),
            ),
            frame("recording", {
              meeting: {
                question: {
                  id: "q",
                  question: "¿Quién lleva el acta?",
                  minutes: 5,
                  origin: "panel",
                  stage: "thinking",
                  sent: true,
                },
              },
            }),
            frame("assistant", {
              session_id: "s",
              turn: [],
              pending: approval,
              voice: { level: 0, state: "idle" },
              activity: "thinking",
            }),
          ],
        }) as unknown as AppBootstrap,
      report: async () => undefined,
      send: async (command: AppCommand) => {
        sent.push(command);
        return { state: "accepted" };
      },
    } as unknown as DaemonPort;
    render(<PanelApp port={port} />);
    const allow = await screen.findByRole("button", { name: "Permitir una vez" });
    await act(async () => userEvent.click(allow));
    expect(sent).toContainEqual({ command: "approval", approved: true, choice: "once" });
  });
});
