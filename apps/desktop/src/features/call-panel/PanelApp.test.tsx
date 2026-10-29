import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, expect, it, vi } from "vitest";
import type { AppBootstrap, AppCommand } from "../../bridge/generated";
import type { DaemonPort } from "../../bridge/port";
import { secondsLeft } from "../../app/call-countdown";
import { TEST_NOW_MS } from "../../test-clock";
import { PanelApp } from "./PanelApp";

afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

/**
 * A port whose recording frame holds `meeting`, the assistant's `pending`,
 * and the hub's `settings` and open conversation (`chat`).
 */
function portWith(
  meeting: Record<string, unknown>,
  pending: unknown = null,
  settings: Record<string, unknown> = {},
  chat: Record<string, unknown> = {},
) {
  const sent: AppCommand[] = [];
  const opened: string[] = [];
  const features = [
    "status",
    "local",
    "permissions",
    "agenda",
    "notes",
    "note",
    "recording",
    "assistant",
    "control",
    "briefing",
  ];
  const port = {
    subscribeFrames: async () => () => undefined,
    snapshot: async () =>
      ({
        revision: { epoch: 1, counter: 1 },
        frames: features.map((feature) => ({
          feature,
          revision: { epoch: 1, counter: 1 },
          view:
            feature === "recording"
              ? { meeting }
              : feature === "status"
                ? { settings, notices: [], notice: null }
                : feature === "assistant"
                  ? { pending, ...chat }
                  : {},
        })),
      }) as unknown as AppBootstrap,
    report: async () => undefined,
    send: async (command: AppCommand) => {
      sent.push(command);
      return { state: "accepted" };
    },
    openUrl: async (url: string) => {
      opened.push(url);
    },
  } as unknown as DaemonPort;
  return { port, sent, opened };
}

const ask = {
  windows: [{ minutes: 2 }, { minutes: 5 }, { minutes: 10 }],
  default_minutes: 5,
  max_question_bytes: 64,
};
const ELAPSED = (8 * 60 + 12) * 1_000;
const recording = {
  phase: "recording",
  recording_id: "rec-1",
  title: "Repaso de la obra",
  started_at_ms: TEST_NOW_MS - ELAPSED,
  ask,
  voice_trigger: { state: "listening" },
};
const byCall = {
  ...recording,
  call_panel: {
    state: "recording",
    recording_id: "rec-1",
    application: "zoom.us",
    event_title: "Ribera Norte · seguimiento",
  },
};
const question = {
  id: "q-1",
  question: "¿Qué dijo Pablo del presupuesto?",
  minutes: 5,
  origin: "typed",
  sent: true,
};

it("a detected call offers to record it, and each answer names the call", async () => {
  const { port, sent } = portWith({
    phase: "idle",
    call_panel: {
      state: "offer",
      call_id: "call-1",
      application: "zoom.us",
      event_title: "Revisión semanal",
    },
  });
  render(<PanelApp port={port} />);
  const card = await screen.findByRole("region", { name: "Aviso de llamada" });
  expect(card.textContent).toContain("Llamada en zoom.us");
  expect(card.textContent).toContain("Revisión semanal");
  await userEvent.click(screen.getByRole("button", { name: "Grabar" }));
  await userEvent.click(screen.getByRole("button", { name: "Ahora no" }));
  expect(sent).toEqual([
    { command: "call_record", call_id: "call-1" },
    { command: "call_dismiss", call_id: "call-1" },
  ]);
});

it("recording a call: its time, its event, asking Deimos, stopping and the voice hint", async () => {
  const { port, sent } = portWith(byCall);
  render(<PanelApp port={port} />);
  expect(await screen.findByText("Grabando · 08:12")).toBeTruthy();
  expect(screen.getByText("Ribera Norte · seguimiento")).toBeTruthy();
  expect(screen.getByRole("button", { name: "Preguntar a Deimos" })).toBeTruthy();
  expect(screen.getByText("Di «Oye Deimos» y tu pregunta")).toBeTruthy();
  await userEvent.click(screen.getByRole("button", { name: "Parar" }));
  expect(sent).toEqual([{ command: "meeting_finish" }]);
});

it("«Terminar sin guardar» asks in the panel first and then names the recording", async () => {
  const { port, sent } = portWith({ ...recording, call_panel: null });
  render(<PanelApp port={port} />);
  await userEvent.click(await screen.findByRole("button", { name: "Terminar sin guardar" }));
  expect(sent).toEqual([]);
  const question = screen.getByRole("group", { name: "¿Terminar sin guardar?" });
  expect(question.textContent).toContain("Se borra lo grabado y no se escribe nota.");
  await userEvent.click(screen.getByRole("button", { name: "Seguir grabando" }));
  expect(screen.getByRole("button", { name: "Parar" })).toBeTruthy();
  await userEvent.click(screen.getByRole("button", { name: "Terminar sin guardar" }));
  await userEvent.click(screen.getByRole("button", { name: "Borrar y terminar" }));
  expect(sent).toEqual([{ command: "meeting_discard", recording_id: "rec-1" }]);
});

it("names the primary agent, not the agent of the conversation left open", async () => {
  const agents = [
    { id: "deimos", name: "Deimos" },
    { id: "codex", name: "Codex" },
  ];
  const { port } = portWith(
    { ...recording, call_panel: null },
    null,
    { agents, default_agent_id: "deimos" },
    { agent_id: "codex" },
  );
  render(<PanelApp port={port} />);
  expect(await screen.findByRole("button", { name: "Preguntar a Deimos" })).toBeTruthy();
  expect(screen.queryByRole("button", { name: "Preguntar a Codex" })).toBeNull();
});

it("offers «Preguntar a Deimos» during a recording started by hand, with no call", async () => {
  const { port } = portWith({ ...recording, call_panel: null });
  render(<PanelApp port={port} />);
  expect(await screen.findByRole("button", { name: "Preguntar a Deimos" })).toBeTruthy();
  expect(screen.getByText("Grabando · 08:12")).toBeTruthy();
  expect(screen.getByText("Repaso de la obra")).toBeTruthy();
  expect(screen.getByText("Di «Oye Deimos» y tu pregunta")).toBeTruthy();
});

it("says why «Oye Deimos» does not listen instead of the hint", async () => {
  const { port } = portWith({
    ...recording,
    voice_trigger: { state: "not_listening", why: { cause: "paused" } },
  });
  render(<PanelApp port={port} />);
  expect(
    await screen.findByText("«Oye Deimos» no escucha: la grabación está en pausa"),
  ).toBeTruthy();
  expect(screen.queryByText("Di «Oye Deimos» y tu pregunta")).toBeNull();
});

it("says why a phrase heard was left without a question instead of the hint", async () => {
  const { port } = portWith({
    ...recording,
    voice_trigger: { state: "abandoned", cause: "question_wait_expired" },
  });
  render(<PanelApp port={port} />);
  expect(
    await screen.findByText(
      "Oí «Oye Deimos», pero no llegó ninguna pregunta. Dila justo después del nombre.",
    ),
  ).toBeTruthy();
  expect(screen.queryByText("Di «Oye Deimos» y tu pregunta")).toBeNull();
});

it("a paused recording says so", async () => {
  const { port } = portWith({ ...recording, phase: "paused" });
  render(<PanelApp port={port} />);
  expect(await screen.findByText("En pausa · 08:12")).toBeTruthy();
});

it("counts down to the end the daemon published and keeps recording on request", async () => {
  vi.useFakeTimers({ shouldAdvanceTime: true });
  vi.setSystemTime(TEST_NOW_MS);
  const { port, sent } = portWith({
    ...byCall,
    call_panel: {
      state: "ending",
      recording_id: "rec-1",
      application: "zoom.us",
      ends_at_ms: TEST_NOW_MS + 13_000,
    },
  });
  render(<PanelApp port={port} />);
  expect(await screen.findByText("La llamada terminó")).toBeTruthy();
  expect(screen.getByText("Dejo de grabar en 13 s")).toBeTruthy();
  act(() => vi.advanceTimersByTime(1_000));
  expect(screen.getByText("Dejo de grabar en 12 s")).toBeTruthy();
  act(() => vi.advanceTimersByTime(20_000));
  expect(screen.getByText("Dejo de grabar en 0 s")).toBeTruthy();
  await act(async () => screen.getByRole("button", { name: "Seguir grabando" }).click());
  expect(sent).toEqual([{ command: "recording_keep", recording_id: "rec-1" }]);
});

it("asks without speaking: the question and the minutes of the call it listens to", async () => {
  const { port, sent } = portWith(byCall);
  render(<PanelApp port={port} />);
  await userEvent.click(await screen.findByRole("button", { name: "Preguntar a Deimos" }));
  const field = screen.getByRole("textbox", { name: "Pregunta sobre la llamada" });
  expect(screen.getByRole("radio", { name: "5 min" }).getAttribute("aria-checked")).toBe("true");
  await userEvent.click(screen.getByRole("radio", { name: "10 min" }));
  await userEvent.type(field, "¿Qué dijo Pablo?");
  await userEvent.click(screen.getByRole("button", { name: "Preguntar" }));
  expect(sent).toEqual([{ command: "meeting_ask", question: "¿Qué dijo Pablo?", minutes: 10 }]);
  expect(screen.queryByRole("textbox")).toBeNull();
  expect(screen.getByText("Grabando · 08:12")).toBeTruthy();
});

it("cancelling or Esc goes back to the recording, and a long question is refused", async () => {
  const { port, sent } = portWith({ ...recording, call_panel: null });
  render(<PanelApp port={port} />);
  await userEvent.click(await screen.findByRole("button", { name: "Preguntar a Deimos" }));
  expect(screen.getByRole("textbox", { name: "Pregunta sobre la reunión" })).toBeTruthy();
  await userEvent.click(screen.getByRole("button", { name: "Cancelar" }));
  expect(screen.queryByRole("textbox")).toBeNull();
  await userEvent.click(screen.getByRole("button", { name: "Preguntar a Deimos" }));
  await userEvent.type(screen.getByRole("textbox"), "x".repeat(65));
  expect(screen.getByRole("alert").textContent).toBe(
    "La pregunta ocupa 65 bytes y el límite es 64 bytes. Acórtala.",
  );
  expect(screen.getByRole("button", { name: "Preguntar" }).hasAttribute("disabled")).toBe(true);
  await userEvent.keyboard("{Escape}");
  expect(screen.queryByRole("textbox")).toBeNull();
  expect(sent).toEqual([]);
});

it("the answer: its question, its text and link, where it also stays, another question", async () => {
  const { port, sent, opened } = portWith({
    ...byCall,
    question: {
      ...question,
      stage: "answered",
      answer: "Pidió esperar. Detalle en [la nota del arranque](https://hub.example/notas/1).",
      shown_until_ms: TEST_NOW_MS + 45_000,
    },
  });
  render(<PanelApp port={port} />);
  const card = await screen.findByRole("region", { name: "Deimos en la reunión" });
  expect(card.textContent).toContain("¿Qué dijo Pablo del presupuesto?");
  expect(card.textContent).toContain("Pidió esperar.");
  expect(card.textContent).toContain("Queda también en tu conversación");
  await userEvent.click(screen.getByRole("link", { name: "la nota del arranque (hub.example)" }));
  expect(opened).toEqual(["https://hub.example/notas/1"]);
  await userEvent.click(screen.getByRole("button", { name: "Cerrar" }));
  expect(sent).toEqual([{ command: "meeting_question_close", id: "q-1" }]);
  await userEvent.click(screen.getByRole("button", { name: "Otra pregunta" }));
  expect(screen.getByRole("textbox", { name: "Pregunta sobre la llamada" })).toBeTruthy();
});

it("while Deimos thinks it may ask permission, answered right in the panel", async () => {
  const { port, sent } = portWith(
    { ...byCall, question: { ...question, stage: "thinking" } },
    {
      kind: "approval",
      id: "a-1",
      text: "Deimos quiere enviar un correo desde Mail",
      choices: ["once", "session", "deny"],
    },
  );
  render(<PanelApp port={port} />);
  expect(await screen.findByText("Deimos quiere enviar un correo desde Mail")).toBeTruthy();
  expect(screen.getByText("¿Qué dijo Pablo del presupuesto?")).toBeTruthy();
  await userEvent.click(screen.getByRole("button", { name: "En esta conversación" }));
  await userEvent.click(screen.getByRole("button", { name: "No" }));
  await userEvent.click(screen.getByRole("button", { name: "Permitir una vez" }));
  expect(sent).toEqual([
    { command: "approval", approved: true, choice: "session" },
    { command: "approval", approved: false, choice: "deny" },
    { command: "approval", approved: true, choice: "once" },
  ]);
});

it("a question without answer says why", async () => {
  const { port } = portWith({
    ...byCall,
    question: { ...question, stage: "failed", cause: { cause: "hub_unreachable" } },
  });
  render(<PanelApp port={port} />);
  expect(await screen.findByText("Deimos no respondió")).toBeTruthy();
  expect(screen.getByRole("alert").textContent).toBe(
    "Sin conexión con el hub. Pregunta de nuevo cuando vuelva.",
  );
});

it("offers nothing to ask outside a recording", async () => {
  const { port } = portWith({ phase: "idle", call_panel: null });
  const { container } = render(<PanelApp port={port} />);
  await waitFor(() => expect(container.querySelector("[data-panel-root]")).not.toBeNull());
  expect(screen.queryByRole("button", { name: "Preguntar a Deimos" })).toBeNull();
});

it("rounds the seconds left up and never below zero", () => {
  expect(secondsLeft(10_000, 0)).toBe(10);
  expect(secondsLeft(10_000, 1)).toBe(10);
  expect(secondsLeft(10_000, 999)).toBe(10);
  expect(secondsLeft(10_000, 1_000)).toBe(9);
  expect(secondsLeft(10_000, 10_000)).toBe(0);
  expect(secondsLeft(10_000, 50_000)).toBe(0);
});

it("«Silenciar» mutes only her microphone, and says it once the Mac confirmed it", async () => {
  const { port, sent } = portWith(byCall);
  render(<PanelApp port={port} />);
  await userEvent.click(await screen.findByRole("button", { name: "Silenciar" }));
  expect(sent).toEqual([{ command: "microphone_mute", muted: true }]);
  expect(screen.queryByText("Micrófono silenciado")).toBeNull();
  cleanup();
  const muted = portWith({ ...byCall, microphone_muted: true });
  render(<PanelApp port={muted.port} />);
  expect(await screen.findByText("Micrófono silenciado")).toBeTruthy();
  await userEvent.click(screen.getByRole("button", { name: "Reactivar micrófono" }));
  expect(muted.sent).toEqual([{ command: "microphone_mute", muted: false }]);
});

it("keeps the question form until accepted, preserving refused text", async () => {
  const { port } = portWith(byCall);
  let accept: () => void = () => undefined;
  port.send = vi
    .fn()
    .mockRejectedValueOnce({
      refusal: { code: "hub_unreachable", since_ms: null, retry: { kind: "after_fix" } },
    })
    .mockImplementation(
      () =>
        new Promise<void>((resolve) => {
          accept = resolve;
        }),
    );
  render(<PanelApp port={port} />);
  await userEvent.click(await screen.findByRole("button", { name: "Preguntar a Deimos" }));
  await userEvent.type(screen.getByRole("textbox"), "What changed?");
  await userEvent.click(screen.getByRole("button", { name: "Preguntar" }));
  expect(screen.getByRole("textbox")).toHaveProperty("value", "What changed?");
  expect(await screen.findByText(/No se pudo mandar la pregunta/)).toBeTruthy();
  await userEvent.click(screen.getByRole("button", { name: "Preguntar" }));
  expect(screen.getByRole("textbox")).toHaveProperty("value", "What changed?");
  expect(screen.getByRole("button", { name: "Preguntar" })).toHaveProperty("disabled", true);
  await act(async () => accept());
  expect(screen.queryByRole("textbox")).toBeNull();
});

it("selects and focuses context radios with arrows and wraps", async () => {
  const { port, sent } = portWith(byCall);
  render(<PanelApp port={port} />);
  await userEvent.click(await screen.findByRole("button", { name: "Preguntar a Deimos" }));
  const five = screen.getByRole("radio", { name: "5 min" });
  expect(five.tabIndex).toBe(0);
  five.focus();
  await userEvent.keyboard("{ArrowRight}");
  expect(screen.getByRole("radio", { name: "10 min" })).toBe(document.activeElement);
  await userEvent.keyboard("{ArrowDown}");
  const two = screen.getByRole("radio", { name: "2 min" });
  expect(two).toBe(document.activeElement);
  expect(two.getAttribute("aria-checked")).toBe("true");
  expect(five.tabIndex).toBe(-1);
  fireEvent.change(screen.getByRole("textbox"), { target: { value: "What changed?" } });
  await userEvent.click(screen.getByRole("button", { name: "Preguntar" }));
  expect(sent).toEqual([{ command: "meeting_ask", question: "What changed?", minutes: 2 }]);
});
