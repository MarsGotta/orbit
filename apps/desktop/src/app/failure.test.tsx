import { act, cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { AppCommand } from "../bridge/generated";
import type { DaemonPort } from "../bridge/port";
import type { Refusal } from "../bridge/refusal";
import { ViewError } from "../design/ViewError";
import { es } from "../i18n/es";
import { createOrbitStore } from "../state/orbit-store";
import { FailureActionsProvider } from "./FailureActions";
import { receive, type Message } from "./interceptor";
import { NoticeBanner } from "./NoticeBanner";

const copyText = vi.hoisted(() => vi.fn(() => Promise.resolve()));
vi.mock("../bridge/clipboard", () => ({ copyText }));

const OP = "7f3a2c9e-0000-4000-8000-000000000001";
const SAVE: AppCommand = { command: "note_edit", recording_id: "rec-1", sections: [] };

afterEach(() => {
  cleanup();
  copyText.mockClear();
  vi.useRealTimers();
});

function refused(refusal: Refusal, command: AppCommand = SAVE): Message {
  const { messages } = receive([], { kind: "refusal", op: OP, refusal }, { command });
  const [message] = messages;
  if (!message) throw new Error("the refusal must be shown");
  return message;
}

describe("the generic failure says what could not be done", () => {
  it("a refused order is named by what it was going to do", () => {
    const message = refused({ code: "internal", retry: { kind: "never" } });
    expect(message.text).toBe(
      "No se pudo guardar la nota. Vuelve a intentarlo. Si se repite, comparte el código 7F3A2C con quien gestiona tu hub.",
    );
    expect(message.failure).toMatchObject({ title: "No se pudo guardar la nota", code: "7F3A2C" });
  });

  it("a failure of the daemon is named by its code", () => {
    const { messages } = receive([], { kind: "notice", notice: { code: "helper_error", op: OP } });
    expect(messages[0]?.text).toBe(
      "No se pudo terminar una tarea en el Mac. Vuelve a intentarlo. Si se repite, comparte el código 7F3A2C con quien gestiona tu hub.",
    );
  });

  it("nothing anywhere asks for an administrator", () => {
    const words = JSON.stringify(es, (_key, value: unknown) =>
      typeof value === "function" ? String(value) : value,
    );
    expect(words).not.toMatch(/administrador/i);
  });
});

describe("«Reintentar» sends the failed order again, when repeating it is safe", () => {
  it("offers the order when the refusal says it can be repeated", () => {
    expect(
      refused({ code: "timeout", retry: { kind: "after_ms", ms: 500 } }).failure?.retry,
    ).toEqual(SAVE);
    expect(refused({ code: "expired", retry: { kind: "after_fix" } }).failure?.retry).toEqual(SAVE);
  });

  it("does not offer it when the order may already have been done, or never will be", () => {
    expect(
      refused({ code: "timeout", retry: { kind: "check_outcome" } }).failure?.retry,
    ).toBeUndefined();
    expect(refused({ code: "internal", retry: { kind: "never" } }).failure?.retry).toBeUndefined();
    const { messages } = receive([], { kind: "notice", notice: { code: "capture_cut", op: OP } });
    expect(messages[0]?.failure?.retry).toBeUndefined();
  });

  it("the store sends it again from where it was sent, in place of the old failure", async () => {
    const sent: [AppCommand, string][] = [];
    let refuse = true;
    const fake: DaemonPort = {
      snapshot: () => Promise.reject(new Error("sin arranque en esta prueba")),
      subscribeFrames: () => Promise.resolve(() => undefined),
      report: () => Promise.resolve(),
      send: (command, op) => {
        sent.push([command, op]);
        return refuse
          ? Promise.reject({ refusal: { code: "timeout", retry: { kind: "after_fix" } } })
          : Promise.resolve({
              revision: { epoch: 1, counter: 1 },
              delivery: { state: "reflected" },
            });
      },
    };
    const store = createOrbitStore(fake);
    await store.getState().send(SAVE, "notes");
    const [failed] = store.getState().messages;
    expect(failed?.place).toBe("notes");
    refuse = false;
    await store.getState().retry(failed?.op ?? "");
    expect(sent.map(([command]) => command)).toEqual([SAVE, SAVE]);
    expect(sent[1]?.[1]).not.toBe(sent[0]?.[1]);
    expect(store.getState().messages).toEqual([]);
  });
});

describe("the failure notice", () => {
  it("says what failed, what to do and the code, with «Reintentar» first", () => {
    const onRetry = vi.fn();
    const message = refused({ code: "timeout", retry: { kind: "after_fix" } });
    render(
      <NoticeBanner
        messages={[message]}
        onAction={vi.fn()}
        onDismiss={vi.fn()}
        onRetry={onRetry}
      />,
    );
    const notice = screen.getByRole("alert");
    expect(within(notice).getByText("No se pudo guardar la nota")).toBeTruthy();
    expect(notice.textContent).toContain(
      "Vuelve a intentarlo. Si se repite, comparte el código 7F3A2C con quien gestiona tu hub.",
    );
    const buttons = within(notice)
      .getAllByRole("button")
      .map((button) => button.textContent);
    expect(buttons.slice(0, 2)).toEqual([
      "Reintentar",
      expect.stringContaining("Copiar el código"),
    ]);
    fireEvent.click(within(notice).getByRole("button", { name: "Reintentar" }));
    expect(onRetry).toHaveBeenCalledWith(OP);
  });

  it("«Copiar el código» copies only the code and says «✓ Copiado»", async () => {
    const message = refused({ code: "internal", retry: { kind: "never" } });
    render(
      <NoticeBanner
        messages={[message]}
        onAction={vi.fn()}
        onDismiss={vi.fn()}
        onRetry={vi.fn()}
      />,
    );
    expect(screen.queryByRole("button", { name: "Reintentar" })).toBeNull();
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: /Copiar el código/ }));
    });
    expect(copyText).toHaveBeenCalledWith("7F3A2C");
    expect(screen.getByText("✓ Copiado").getAttribute("data-shown")).toBe("true");
  });

  it("inside the view that sent the order, the failure has the same two actions", () => {
    const onRetry = vi.fn();
    const message = refused({ code: "timeout", retry: { kind: "after_fix" } });
    render(
      <FailureActionsProvider messages={[message]} onRetry={onRetry}>
        <ViewError error={message.text} />
      </FailureActionsProvider>,
    );
    const failure = screen.getByRole("alert");
    fireEvent.click(within(failure).getByRole("button", { name: "Reintentar" }));
    expect(onRetry).toHaveBeenCalledWith(OP);
    expect(within(failure).getByRole("button", { name: /Copiar el código/ })).toBeTruthy();
  });
});

it.each(["audio", "voice_transcription"])(
  "the %s navigation wait has no code or retry banner",
  (operation) => {
    const message = refused({ code: "busy", operation, retry: { kind: "after_fix" } });
    render(
      <NoticeBanner
        messages={[message]}
        onAction={vi.fn()}
        onDismiss={vi.fn()}
        onRetry={vi.fn()}
      />,
    );
    expect(screen.getByText(/Espera a que termine/)).toBeTruthy();
    expect(screen.queryByText(/comparte el código/)).toBeNull();
    expect(screen.queryByRole("button", { name: /Copiar el código/ })).toBeNull();
    expect(screen.queryByRole("button", { name: "Reintentar" })).toBeNull();
  },
);
