import { cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { AppCommand } from "../../bridge/generated";
import type { ChatMessage } from "../../bridge/generated/ChatMessage";
import type { ChatSessionSummary } from "../../bridge/generated/ChatSessionSummary";
import type { MeetingQuestionSnapshot } from "../../bridge/generated/MeetingQuestionSnapshot";
import type { MessagesPage } from "../../bridge/reads";
import { contextForView } from "../../app/ContextPanel";
import { QuestionCard } from "../../app/meeting-question/QuestionCard";
import type { AppView as AppSnapshot } from "../../state/app-view";
import AssistantView from "./AssistantView";
import { conversationView } from "./conversation";
import { MessageList } from "./MessageList";
import { sessionTitle } from "./session-title";

// SIM-30: the conversations are the hub's. The screen paints the pages it
// reads and, after them, the turn in progress; Deimos's Markdown is drawn.

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

function snapshot(chat: Partial<AppSnapshot["chat"]>): AppSnapshot {
  return {
    connection: { daemon: "ready", hub: "paired" },
    status_availability: { state: "current" },
    call_availability: { state: "current" },
    call: null,
    settings: { hub: { paired: true, connection: "online" } },
    chat: {
      session_id: null,
      turn: [],
      pending: null,
      voice: { level: 0, state: "idle", availability: { state: "current" } },
      availability: { state: "current" },
      ...chat,
    },
  } as unknown as AppSnapshot;
}

describe("SIM-30 fallo 5: una conversación anterior se abre con sus mensajes", () => {
  it("reads the opened conversation from the hub and paints it", async () => {
    const read = vi.fn(() =>
      Promise.resolve(
        page([message("1", "user", "¿Qué hay hoy?"), message("2", "assistant", "Tres reuniones.")]),
      ),
    );
    render(
      <AssistantView
        error={null}
        readMessages={read}
        send={() => undefined}
        snapshot={snapshot({ session_id: "session-old" })}
      />,
    );
    expect(await screen.findByText("¿Qué hay hoy?")).toBeTruthy();
    expect(screen.getByText("Tres reuniones.")).toBeTruthy();
    expect(read).toHaveBeenCalledWith("session-old", null);
  });
});

describe("SIM-30 el turno en curso y lo que guarda el hub", () => {
  const question = message("turn-1", "user", "Pregunta");
  const answer = message("turn-1:assistant", "assistant", "Respuesta");
  const before = [message("1", "user", "Antes"), message("2", "assistant", "Contestado")];

  it("keeps a finished turn in view until the page holds what followed it", () => {
    expect(conversationView(before, [question, answer], false)).toEqual([
      ...before,
      question,
      answer,
    ]);
    // The hub stored the question but not yet the answer: the turn stays.
    expect(conversationView([...before, question], [question, answer], false)).toEqual([
      ...before,
      question,
      answer,
    ]);
    const stored = [...before, question, message("9", "assistant", "Respuesta")];
    expect(conversationView(stored, [question, answer], false)).toEqual(stored);
  });

  it("shows a turn in flight once, after what came before it", () => {
    const stored = [...before, question];
    expect(conversationView(stored, [question], true)).toEqual([...before, question]);
  });
});

describe("SIM-30 el Markdown de Deimos se pinta", () => {
  it("draws Deimos's bold in the chat, and leaves Alina's text as she wrote it", () => {
    render(
      <MessageList
        messages={[
          message("1", "user", "**mío**"),
          message("2", "assistant", "**Presupuesto** aprobado"),
        ]}
        send={() => undefined}
      />,
    );
    const bold = screen.getByText("Presupuesto");
    expect(bold.closest("strong")).not.toBeNull();
    expect(screen.queryByText(/\*\*Presupuesto\*\*/)).toBeNull();
    expect(screen.getByText("**mío**")).toBeTruthy();
  });

  it("draws Deimos's bold in the floating panel", () => {
    const question = {
      id: "turn-1",
      question: "¿Qué se decidió?",
      minutes: 2,
      origin: "voice",
      stage: "answered",
      answer: "**Presupuesto** aprobado",
      asked_at_ms: 1,
      sent: true,
    } as unknown as MeetingQuestionSnapshot;
    render(<QuestionCard question={question} send={() => undefined} />);
    const bold = screen.getByText("Presupuesto");
    expect(bold.closest("strong")).not.toBeNull();
    expect(screen.queryByText(/\*\*/)).toBeNull();
  });
});

describe("SIM-30 la lista y las notas de voz", () => {
  it("names each conversation by its date, as the list did", () => {
    expect(sessionTitle(Date.UTC(2026, 9, 17, 7, 53), "Europe/Madrid")).toBe(
      "17 de octubre de 2026, 09:53",
    );
    const sessions: ChatSessionSummary[] = [
      {
        id: "a",
        created_at_ms: 1_789_000_000_000,
        last_at_ms: 1_789_000_500_000,
        message_count: 4,
      },
      {
        id: "b",
        created_at_ms: 1_788_000_000_000,
        last_at_ms: 1_788_000_000_000,
        message_count: 0,
      },
    ];
    const sent: AppCommand[] = [];
    const data = contextForView(
      "assistant",
      snapshot({ session_id: "a" }),
      (command) => sent.push(command),
      sessions,
    );
    const rows = data?.groups.flatMap((group) => group.rows) ?? [];
    expect(rows[0]?.title).toBe(sessionTitle(1_789_000_000_000));
    expect(rows.at(-1)?.title).toBe(sessionTitle(1_788_000_000_000));
    rows.at(-1)?.onSelect?.();
    expect(sent).toEqual([{ command: "session_open", id: "b" }]);
  });

  it("shows a voice note from the hub with its length, and plays it only if this Mac has it", async () => {
    const voice = (id: string, available: boolean): ChatMessage => ({
      ...message(id, "user", "Lo que dije"),
      voice: { duration_ms: 4_200, transcript: "Lo que dije", audio_available: available },
    });
    const sent: AppCommand[] = [];
    render(
      <MessageList
        messages={[voice("held", true), voice("gone", false)]}
        send={(command) => sent.push(command)}
      />,
    );
    const buttons = screen.getAllByRole("button");
    expect(buttons.map((button) => (button as HTMLButtonElement).disabled)).toEqual([false, true]);
    expect(screen.getAllByText("0:04")).toHaveLength(2);
    await userEvent.click(buttons[0]!);
    expect(sent).toEqual([{ command: "voice_note_audio", message_id: "held" }]);
  });

  it("plays the audio the app received for that note", async () => {
    // jsdom has no media playback.
    const played = vi.spyOn(HTMLMediaElement.prototype, "play").mockResolvedValue(undefined);
    const note: ChatMessage = {
      ...message("held", "user", "Lo que dije"),
      voice: { duration_ms: 1_000, transcript: "Lo que dije", audio_available: true },
    };
    render(
      <MessageList
        messages={[note]}
        send={() => undefined}
        voiceAudio={{ message_id: "held", format: "wav", bytes_base64: "UklGRg==" }}
      />,
    );
    await waitFor(() =>
      expect(screen.getByTestId("voice-audio-held").getAttribute("src")).toBe(
        "data:audio/wav;base64,UklGRg==",
      ),
    );
    expect(played).toHaveBeenCalled();
    played.mockRestore();
  });
});
