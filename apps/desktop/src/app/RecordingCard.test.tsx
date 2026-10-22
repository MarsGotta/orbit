import { act, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { AppCommand } from "../bridge/generated";
import type { MeetingSnapshot } from "../bridge/generated/MeetingSnapshot";
import { RecordingCard } from "./RecordingCard";

// 11:45 in Madrid on 21 October 2026, an invented deadline.
const ENDS = Date.UTC(2026, 9, 21, 9, 45);

function meeting(change: Partial<MeetingSnapshot> = {}): MeetingSnapshot {
  return {
    phase: "recording",
    recording_id: "rec-1",
    title: "Revisión del presupuesto",
    started_at_ms: Date.now() - 62_000,
    ends_at_ms: ENDS,
    limit_minutes: 240,
    audio: "system",
    tracks: [],
    upload: { pending_chunks: 0, offline: false },
    call_panel: null,
    call_preference: null,
    ask: {
      windows: [{ minutes: 2 }, { minutes: 5 }, { minutes: 10 }],
      default_minutes: 2,
      max_question_bytes: 40,
    },
    voice_trigger: { state: "listening" },
    recent: [],
    applications: [],
    applications_state: "ready",
    ...change,
  };
}

function renderCard(value = meeting()) {
  const sent: AppCommand[] = [];
  const view = render(
    <RecordingCard
      meeting={value}
      send={(command) => sent.push(command)}
      timezone="Europe/Madrid"
    />,
  );
  const rerender = (next: MeetingSnapshot) =>
    view.rerender(
      <RecordingCard
        meeting={next}
        send={(command) => sent.push(command)}
        timezone="Europe/Madrid"
      />,
    );
  return { sent, rerender, view };
}

afterEach(() => {
  vi.useRealTimers();
});

describe("the recording card", () => {
  it("says what it records, until when, and whether «Oye Deimos» listens", () => {
    renderCard();
    const card = screen.getByRole("region", { name: "Grabación en curso" });
    expect(card.textContent).toContain("Grabando");
    expect(card.textContent).toContain("Revisión del presupuesto");
    expect(card.textContent).toContain("Audio del sistema · hasta las 11:45");
    expect(card.textContent).toContain("«Oye Deimos» te escucha");
    expect(within(card).getByRole("timer", { name: "Tiempo grabado" }).textContent).toBe("01:02");
  });

  it("keeps its controls in sight and sends each order", async () => {
    const { sent } = renderCard();
    await userEvent.click(screen.getByRole("button", { name: "Pausar" }));
    expect(sent).toEqual([{ command: "meeting_pause" }]);
    // Reachable from the keyboard, as the real test presses it.
    screen.getByRole("button", { name: "Terminar" }).focus();
    await userEvent.keyboard("{Enter}");
    expect(sent).toContainEqual({ command: "meeting_finish" });
  });

  it("in pause the clock stops, «Oye Deimos» rests and «Seguir» resumes", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const { sent, rerender } = renderCard();
    rerender(
      meeting({
        phase: "paused",
        voice_trigger: { state: "not_listening", why: { cause: "paused" } },
      }),
    );
    const card = screen.getByRole("region", { name: "Grabación en curso" });
    expect(card.textContent).toContain("En pausa");
    expect(card.textContent).toContain("«Oye Deimos» no escucha en pausa");
    const stopped = screen.getByRole("timer").textContent;
    act(() => vi.advanceTimersByTime(3_000));
    expect(screen.getByRole("timer").textContent).toBe(stopped);
    await userEvent.click(screen.getByRole("button", { name: "Seguir" }));
    expect(sent).toEqual([{ command: "meeting_resume" }]);
  });

  it("counts every second while it records", () => {
    vi.useFakeTimers();
    renderCard();
    expect(screen.getByRole("timer").textContent).toBe("01:02");
    act(() => vi.advanceTimersByTime(2_000));
    expect(screen.getByRole("timer").textContent).toBe("01:04");
  });

  it("says when a track arrives silent, and where to look", () => {
    renderCard(
      meeting({ tracks: [{ name: "mic_local", level: "silent", chunks_uploaded: 3, gaps: 0 }] }),
    );
    expect(screen.getByText("Micrófono en silencio")).toBeTruthy();
    expect(screen.getByRole("link", { name: "Revisar" }).getAttribute("href")).toBe(
      "#/settings/privacidad",
    );
  });

  it("after the call ends, counts down and offers to keep recording", async () => {
    const { sent } = renderCard(
      meeting({
        call_panel: {
          state: "ending",
          recording_id: "rec-1",
          application: "Videollamada",
          ends_at_ms: Date.now() + 13_000,
        },
      }),
    );
    expect(screen.getByText(/Dejo de grabar en 13 s/)).toBeTruthy();
    await userEvent.click(screen.getByRole("button", { name: "Seguir grabando" }));
    expect(sent).toEqual([{ command: "recording_keep", recording_id: "rec-1" }]);
  });

  it("is gone once the recording is over", () => {
    const { rerender } = renderCard();
    rerender(meeting({ phase: "idle" }));
    expect(screen.queryByRole("region", { name: "Grabación en curso" })).toBeNull();
  });
});

describe("«Preguntar a Deimos» from the card", () => {
  it("asks with the windows app-core offers and shows Deimos thinking", async () => {
    const { sent } = renderCard();
    await userEvent.click(screen.getByRole("button", { name: "Preguntar a Deimos" }));
    const menu = screen.getByRole("dialog", { name: "Pregunta sobre esta reunión" });
    const windows = within(menu).getAllByRole("radio");
    expect(windows.map((window) => window.textContent)).toEqual(["2 min", "5 min", "10 min"]);
    expect(windows[0]?.getAttribute("aria-checked")).toBe("true");
    expect(menu.textContent).toContain("O di «Oye Deimos» en voz alta");
    await userEvent.click(within(menu).getByRole("radio", { name: "5 min" }));
    await userEvent.type(
      within(menu).getByRole("textbox", { name: "Tu pregunta" }),
      "¿Y el plazo?",
    );
    await userEvent.click(within(menu).getByRole("button", { name: "Preguntar" }));
    expect(sent).toEqual([{ command: "meeting_ask", question: "¿Y el plazo?", minutes: 5 }]);
    expect(menu.textContent).toContain("Deimos está pensando…");
    expect(menu.textContent).toContain("¿Y el plazo?");
  });

  it("shows the answer that arrives, and «Otra pregunta» asks again", async () => {
    const { rerender } = renderCard();
    await userEvent.click(screen.getByRole("button", { name: "Preguntar a Deimos" }));
    await userEvent.type(
      screen.getByRole("textbox", { name: "Tu pregunta" }),
      "¿Y el plazo?{Enter}",
    );
    rerender(
      meeting({
        question: {
          id: "turn-9",
          question: "¿Y el plazo?",
          minutes: 2,
          origin: "typed",
          stage: "answered",
          answer: "El viernes 30.",
          sent: true,
        },
      }),
    );
    const menu = screen.getByRole("dialog", { name: "Pregunta sobre esta reunión" });
    expect(menu.textContent).toContain("El viernes 30.");
    expect(menu.textContent).toContain("Queda también en tu conversación con Deimos");
    // The panel lets the answer go after a while; the menu keeps it.
    rerender(meeting());
    expect(menu.textContent).toContain("El viernes 30.");
    await userEvent.click(within(menu).getByRole("button", { name: "Otra pregunta" }));
    expect(
      (within(menu).getByRole("textbox", { name: "Tu pregunta" }) as HTMLInputElement).value,
    ).toBe("");
  });

  it("says why a question got no answer, without codes", async () => {
    const { rerender } = renderCard();
    await userEvent.click(screen.getByRole("button", { name: "Preguntar a Deimos" }));
    await userEvent.type(
      screen.getByRole("textbox", { name: "Tu pregunta" }),
      "¿Qué falta?{Enter}",
    );
    rerender(
      meeting({
        question: {
          id: "turn-10",
          question: "¿Qué falta?",
          minutes: 2,
          origin: "typed",
          stage: "failed",
          cause: { cause: "hub_refused", code: "send_failed", message: "canal cerrado" },
          sent: true,
        },
      }),
    );
    expect(screen.getByRole("alert").textContent).toBe("Vuelve a preguntar en un momento.");
    expect(screen.getByRole("dialog").textContent).not.toMatch(/send_failed|canal cerrado/);
  });

  it("refuses a question too long for Deimos, and Escape closes the menu", async () => {
    const { sent } = renderCard();
    await userEvent.click(screen.getByRole("button", { name: "Preguntar a Deimos" }));
    await userEvent.type(screen.getByRole("textbox", { name: "Tu pregunta" }), "ñ".repeat(21));
    expect(screen.getByRole("alert").textContent).toBe("La pregunta es demasiado larga: acórtala.");
    await userEvent.keyboard("{Enter}");
    expect(sent).toEqual([]);
    await userEvent.keyboard("{Escape}");
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("is not offered while app-core does not offer it", () => {
    renderCard(meeting({ ask: undefined }));
    expect(screen.queryByRole("button", { name: "Preguntar a Deimos" })).toBeNull();
  });
});
