import { cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { AppCommand } from "../../bridge/generated";
import type { ChatMessage } from "../../bridge/generated/ChatMessage";
import type { ChatSessionSummary } from "../../bridge/generated/ChatSessionSummary";
import type { MeetingQuestionSnapshot } from "../../bridge/generated/MeetingQuestionSnapshot";
import type { MessagesPage } from "../../bridge/reads";
import { QuestionCard } from "../../app/meeting-question/QuestionCard";
import type { AppView as AppSnapshot } from "../../state/app-view";
import AssistantView from "./AssistantView";
import { conversationView } from "./conversation";
import { MessageList } from "./MessageList";
import { ShellSlots, useShellSlots } from "../../app/shell-slots";
import { AssistantSessions } from "./AssistantSessions";
import { sessionWhen } from "./session-title";
import { es } from "../../i18n/es";

/** The window's list column, where the conversations are drawn. */
function ColumnHost() {
  return <div ref={useShellSlots()?.setColumn} />;
}

// The conversations are the hub's. The screen paints the pages it
// reads and, after them, the turn in progress; Deimos's Markdown is drawn.

afterEach(cleanup);

/** The open conversation's row: marked current, not a button to press again. */
async function openRow(): Promise<HTMLElement> {
  return waitFor(() => {
    const row = document.querySelector<HTMLElement>('[aria-current="page"]');
    expect(row).not.toBeNull();
    return row as HTMLElement;
  });
}

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

describe("an earlier conversation opens with its messages", () => {
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

describe("the turn in flight and what the hub stores", () => {
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

describe("Deimos's Markdown is painted", () => {
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

describe("the list and the voice notes", () => {
  it("names each conversation by when it began and opens the one chosen", async () => {
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
    render(
      <ShellSlots>
        <ColumnHost />
        <AssistantSessions
          read={() =>
            Promise.resolve({
              state: "present",
              value: { items: sessions, next: null, total: 2, revision: 1 },
              as_of_ms: null,
            })
          }
          send={(command) => sent.push(command)}
          snapshot={snapshot({ session_id: "a" })}
        />
      </ShellSlots>,
    );
    const open = await openRow();
    expect(open.getAttribute("aria-label")).toContain(sessionWhen(1_789_000_000_000, Date.now()));
    const older = screen.getByRole("link", {
      name: new RegExp(sessionWhen(1_788_000_000_000, Date.now())),
    });
    expect(older.getAttribute("aria-current")).toBeNull();
    await userEvent.click(older);
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

  it("plays the audio the app received for that note when pressed, never by itself", async () => {
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
    expect(played).not.toHaveBeenCalled();
    await userEvent.click(screen.getByRole("button", { name: es.assistant.playVoiceNote }));
    expect(played).toHaveBeenCalledTimes(1);
    played.mockRestore();
  });
});

it("names a pending approval and a vendor retry as waits", async () => {
  const view = render(
    <AssistantView
      error={null}
      send={() => undefined}
      snapshot={snapshot({
        activity: "thinking",
        pending: { kind: "approval", id: "permit", text: "Permit?", choices: [] },
      })}
    />,
  );
  expect(await screen.findByText("Deimos espera tu permiso")).toBeTruthy();
  expect(screen.queryByText("Deimos está pensando…")).toBeNull();
  view.rerender(
    <AssistantView
      error={null}
      send={() => undefined}
      snapshot={snapshot({ activity: "retrying_openai" as AppSnapshot["chat"]["activity"] })}
    />,
  );
  expect(await screen.findByText("OpenAI tarda en contestar; reintentando")).toBeTruthy();
});

it("marks a conversation waiting for its owner while another is open", async () => {
  render(
    <ShellSlots>
      <ColumnHost />
      <AssistantSessions
        read={() =>
          Promise.resolve({
            state: "present",
            value: {
              items: [{ id: "left", created_at_ms: 1, last_at_ms: 1, message_count: 1 }],
              next: null,
              total: 1,
              revision: 1,
            },
            as_of_ms: null,
          })
        }
        send={() => undefined}
        snapshot={snapshot({ session_id: "other", waiting_sessions: { left: "approval" } })}
      />
    </ShellSlots>,
  );
  expect(await screen.findByText("espera tu permiso")).toBeTruthy();
});

it("refreshes history when a left turn finishes during the opening read", async () => {
  const read = vi
    .fn()
    .mockResolvedValueOnce(page([message("turn", "user", "Question")]))
    .mockResolvedValue(
      page([message("turn", "user", "Question"), message("answer", "assistant", "Finished away")]),
    );
  const view = render(
    <AssistantView
      error={null}
      send={() => undefined}
      readMessages={read}
      snapshot={snapshot({ session_id: "left", history_revision: 1 })}
    />,
  );
  expect(await screen.findByText("Question")).toBeTruthy();
  view.rerender(
    <AssistantView
      error={null}
      send={() => undefined}
      readMessages={read}
      snapshot={snapshot({ session_id: "left", history_revision: 2 })}
    />,
  );
  expect(await screen.findByText("Finished away")).toBeTruthy();
});

it("keeps a stored voice note's playback when its live answer is observed again", () => {
  const stored = {
    ...message("turn", "user", "Question"),
    voice: { duration_ms: 4000, transcript: "Question", audio_available: true },
  };
  const observed = { ...stored, voice: { ...stored.voice, audio_available: false } };
  expect(conversationView([stored], [observed], true)[0]?.voice?.audio_available).toBe(true);
});

it("names an unanswered clarification as a wait for the owner's answer", async () => {
  render(
    <AssistantView
      error={null}
      send={() => undefined}
      snapshot={snapshot({
        activity: "thinking",
        pending: {
          kind: "clarify",
          id: "question",
          text: "Which date?",
          choices: [],
          questions: [],
        },
      })}
    />,
  );
  expect(await screen.findByText("Deimos espera tu respuesta")).toBeTruthy();
  expect(screen.queryByText("Deimos está pensando…")).toBeNull();
});

it("refreshes the list when opening a running turn so the left permission remains listed", async () => {
  const read = vi
    .fn()
    .mockResolvedValueOnce({
      state: "present",
      value: { items: [], next: null, total: 0, revision: 1 },
      as_of_ms: null,
    })
    .mockResolvedValue({
      state: "present",
      value: {
        items: [{ id: "left", created_at_ms: 1, last_at_ms: 1, message_count: 1 }],
        next: null,
        total: 1,
        revision: 2,
      },
      as_of_ms: null,
    });
  const list = (chat: Partial<AppSnapshot["chat"]>) => (
    <ShellSlots>
      <ColumnHost />
      <AssistantSessions read={read} send={() => undefined} snapshot={snapshot(chat)} />
    </ShellSlots>
  );
  const view = render(list({ session_id: "left", activity: "thinking" }));
  await waitFor(() => expect(read).toHaveBeenCalledTimes(1));
  view.rerender(
    list({ session_id: "other", activity: "thinking", waiting_sessions: { left: "approval" } }),
  );
  expect(await screen.findByText("espera tu permiso")).toBeTruthy();
  expect(read).toHaveBeenCalledTimes(2);
});
