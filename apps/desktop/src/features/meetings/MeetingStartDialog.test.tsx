import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, expect, it, vi } from "vitest";
import type { AppCommand } from "../../bridge/generated";
import type { MeetingAudio } from "../../bridge/generated/MeetingAudio";
import type { AppView } from "../../state/app-view";
import { TEST_NOW_MS } from "../../test-clock";
import { MeetingStartDialog } from "./MeetingStartDialog";

afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

const MINUTE = 60_000;

/** The view the dialog reads, at the suite's 14:30 UTC. */
function view(change: Record<string, unknown> = {}, meeting: Record<string, unknown> = {}) {
  return {
    privacy_stopped: false,
    system_timezone: "UTC",
    permissions: { microphone: "granted", screen: "granted" },
    briefing: undefined,
    ...change,
    meeting: {
      phase: "idle",
      recording_id: null,
      limit_minutes: 240,
      applications: [
        { name: "Orbit Pruebas", pid: 4242 },
        { name: "Llamadas Norte", pid: 5151 },
      ],
      applications_state: "ready",
      ...meeting,
    },
  } as unknown as AppView;
}

function next(start: number, end: number, title = "Revisión del tablero de obras") {
  return { briefing: { next: { title, start_ms: start, end_ms: end } } };
}

function open(snapshot: AppView, error: string | null = null) {
  const sent: AppCommand[] = [];
  const close = vi.fn();
  const dismiss = vi.fn();
  const props = {
    onClose: close,
    onErrorDismiss: dismiss,
    send: (command: AppCommand) => sent.push(command),
  };
  const rendered = render(<MeetingStartDialog {...props} error={error} snapshot={snapshot} />);
  const rerender = (snapshot: AppView, error: string | null = null) =>
    rendered.rerender(<MeetingStartDialog {...props} error={error} snapshot={snapshot} />);
  return { sent, close, dismiss, rerender };
}

const startButton = () => screen.getByRole("button", { name: /Empezar a grabar/ });

const sources: [string, MeetingAudio][] = [
  ["Audio del sistema", "system"],
  ["Solo mi micrófono", "microphone_only"],
  ["Una app concreta", { application: { application_pid: 4242 } }],
];

it.each(sources)("recording «%s» sends orbit-core's audio selection", async (source, audio) => {
  const { sent } = open(view());
  expect(sent).toEqual([{ command: "audio_sources_refresh" }]);
  await userEvent.click(screen.getByRole("radio", { name: new RegExp(source) }));
  await userEvent.click(startButton());
  expect(sent).toContainEqual({ command: "meeting_start", title: "", audio });
});

it("is «Nueva grabación», a window that Esc closes", async () => {
  const { close } = open(view());
  const dialog = screen.getByRole("dialog", { name: "Nueva grabación" });
  expect(dialog.textContent).toContain("Orbit graba y, al terminar, escribe la nota por ti.");
  expect(dialog.textContent).toContain("esc para cerrar");
  await userEvent.keyboard("{Escape}");
  expect(close).toHaveBeenCalledOnce();
});

it("an untitled recording is «Reunión HH:MM» and the limit is said before it starts", () => {
  open(view());
  expect(screen.getByRole("textbox", { name: "Título" }).getAttribute("placeholder")).toBe(
    "Reunión 14:30",
  );
  expect(
    screen.getByText("Graba hasta 4 h, hasta las 18:30. Si cierras la ventana, sigue grabando."),
  ).toBeTruthy();
});

it("suggests the title of the event about to start, and uses it on request", async () => {
  const { sent } = open(view(next(TEST_NOW_MS + 20 * MINUTE, TEST_NOW_MS + 80 * MINUTE)));
  await userEvent.click(
    screen.getByRole("button", {
      name: "Usar «Revisión del tablero de obras», que empieza en 20 min",
    }),
  );
  expect(screen.getByRole<HTMLInputElement>("textbox", { name: "Título" }).value).toBe(
    "Revisión del tablero de obras",
  );
  expect(screen.getByText("Aparecerá en Reuniones con este nombre.")).toBeTruthy();
  await userEvent.click(startButton());
  expect(sent).toContainEqual({
    command: "meeting_start",
    title: "Revisión del tablero de obras",
    audio: "system",
  });
});

it("suggests the event in progress by when it started", () => {
  open(view(next(TEST_NOW_MS - 10 * MINUTE, TEST_NOW_MS + 20 * MINUTE)));
  expect(
    screen.getByRole("button", {
      name: "Usar «Revisión del tablero de obras», que empezó a las 14:20",
    }),
  ).toBeTruthy();
});

it("suggests nothing for an event hours away", () => {
  open(view(next(TEST_NOW_MS + 3 * 60 * MINUTE, TEST_NOW_MS + 4 * 60 * MINUTE)));
  expect(screen.queryByRole("button", { name: /^Usar/ })).toBeNull();
});

it("the chosen application is one of the chips of what plays now", async () => {
  const { sent } = open(view());
  await userEvent.click(screen.getByRole("radio", { name: /Una app concreta/ }));
  expect(screen.getByText("Suenan ahora:")).toBeTruthy();
  expect(screen.getByRole("button", { name: "Orbit Pruebas", pressed: true })).toBeTruthy();
  await userEvent.click(screen.getByRole("button", { name: "Llamadas Norte", pressed: false }));
  expect(
    screen.getByText("Se graba la app entera, no una sola pestaña del navegador."),
  ).toBeTruthy();
  await userEvent.click(startButton());
  expect(sent).toContainEqual({
    command: "meeting_start",
    title: "",
    audio: { application: { application_pid: 5151 } },
  });
});

it("says it is looking for applications while the list arrives, and why it has none after", () => {
  vi.useFakeTimers();
  const { sent } = open(view({}, { applications: [], applications_state: "unavailable" }));
  const option = screen.getByRole("radio", { name: /Una app concreta/ });
  expect(option.closest("label")?.textContent).toContain("Buscando apps con sonido…");
  fireEvent.click(option);
  expect(startButton().hasAttribute("disabled")).toBe(true);
  const why = document.getElementById(startButton().getAttribute("aria-describedby") ?? "");
  expect(why?.textContent).toBe("Buscando apps con sonido…");
  act(() => vi.advanceTimersByTime(10_000));
  expect(screen.getByText("No se pudo saber qué apps suenan ahora.")).toBeTruthy();
  fireEvent.click(screen.getByRole("button", { name: "Volver a buscar" }));
  expect(sent.filter((command) => command.command === "audio_sources_refresh")).toHaveLength(2);
  expect(screen.getByRole("status").textContent).toBe("Buscando apps con sonido…");
});

it("says when no application plays audio", async () => {
  open(view({}, { applications: [], applications_state: "empty" }));
  await userEvent.click(screen.getByRole("radio", { name: /Una app concreta/ }));
  expect(
    screen.getByText("Ninguna app suena ahora. Pon el sonido en la app y vuelve a buscar."),
  ).toBeTruthy();
  expect(startButton().hasAttribute("disabled")).toBe(true);
});

it("a missing microphone is one row inside the window, and recording waits for it", async () => {
  const { sent } = open(view({ permissions: { microphone: "denied", screen: "granted" } }));
  expect(screen.getAllByText("Falta el permiso de micrófono")).toHaveLength(1);
  expect(screen.getByText("En cuanto lo des, podrás grabar.")).toBeTruthy();
  expect(startButton().hasAttribute("disabled")).toBe(true);
  expect(startButton().getAttribute("aria-describedby")).toBeTruthy();
  await userEvent.click(screen.getByRole("button", { name: "Abrir Ajustes del Sistema" }));
  expect(sent).toContainEqual({ command: "open_system_privacy", pane: "Microphone" });
});

it("the missing screen permission stops every source but the microphone", async () => {
  open(view({ permissions: { microphone: "granted", screen: "denied" } }));
  expect(screen.getByText("Falta el permiso de pantalla y audio del sistema")).toBeTruthy();
  expect(startButton().hasAttribute("disabled")).toBe(true);
  await userEvent.click(screen.getByRole("radio", { name: /Solo mi micrófono/ }));
  expect(screen.queryByText("Falta el permiso de pantalla y audio del sistema")).toBeNull();
  expect(startButton().hasAttribute("disabled")).toBe(false);
});

it("with everything stopped, the row reactivates Orbit", async () => {
  const { sent } = open(view({ privacy_stopped: true }));
  expect(screen.getByText("Todo está parado")).toBeTruthy();
  expect(startButton().hasAttribute("disabled")).toBe(true);
  await userEvent.click(screen.getByRole("button", { name: "Reactivar Orbit" }));
  expect(sent).toContainEqual({ command: "rearm" });
});

it("says it is starting until the recording begins, then closes", async () => {
  const { close, rerender } = open(view());
  await userEvent.click(startButton());
  expect(screen.getByRole("button", { name: "Empezando…" }).getAttribute("aria-busy")).toBe("true");
  expect(close).not.toHaveBeenCalled();
  rerender(view({}, { phase: "starting", recording_id: "rec-new" }));
  expect(close).toHaveBeenCalledOnce();
});

it("a start refused after the press is said inside the window, and the button is back", async () => {
  const { close, rerender } = open(view(), "Una negativa anterior.");
  expect(screen.queryByText("Una negativa anterior.")).toBeNull();
  await userEvent.click(startButton());
  rerender(view(), "No se pudo empezar a grabar. Vuelve a intentarlo.");
  expect(screen.getByRole("alert").textContent).toBe(
    "No se pudo empezar a grabar. Vuelve a intentarlo.",
  );
  expect(startButton().getAttribute("aria-busy")).toBeNull();
  expect(close).not.toHaveBeenCalled();
});

it("pressing again clears the refusal shown, so the same refusal is said again", async () => {
  const refused = "No se pudo empezar a grabar. Vuelve a intentarlo.";
  const { dismiss, rerender } = open(view());
  await userEvent.click(startButton());
  rerender(view(), refused);
  await userEvent.click(startButton());
  expect(dismiss).toHaveBeenCalledOnce();
  expect(screen.queryByRole("alert")).toBeNull();
  rerender(view(), null);
  rerender(view(), refused);
  expect(screen.getByRole("alert").textContent).toBe(refused);
});
