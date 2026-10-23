import { act, cleanup, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { AgentCall } from "../../bridge/generated/AgentCall";
import type { AgentOutcome } from "../../bridge/generated/AgentOutcome";
import type { SignInAttempt } from "../../bridge/generated/SignInAttempt";
import { ARES, assistantSnapshot, DEIMOS, NOW, PHOBOS, renderAssistant } from "./test-assistants";

// An expired session (annex 36, 36a, 36b) and signing in from Orbit: the hub
// runs the vendor's tool and hands the Mac the address to open, and Claude's
// code to paste back. Its failures are said with the hub's cause.

beforeEach(() => {
  vi.useFakeTimers({ now: NOW, toFake: ["Date"] });
  window.location.hash = `#/assistant/${ARES.id}`;
});

afterEach(() => {
  cleanup();
  vi.useRealTimers();
  window.location.hash = "";
});

function attempt(change: Partial<SignInAttempt> = {}): SignInAttempt {
  return {
    agent_id: ARES.id,
    state: "waiting",
    expects_code: true,
    url: "https://claude.example/oauth/authorize",
    expires_at_ms: NOW + 15 * 60_000,
    ...change,
  };
}

function hubThatSignsIn(answers: Partial<Record<AgentCall["call"], () => AgentOutcome>>) {
  return (call: AgentCall): AgentOutcome =>
    answers[call.call]?.() ?? { outcome: "sign_in", attempt: attempt() };
}

describe("an expired session", () => {
  it("says so in the conversation, and «Iniciar sesión en Claude» opens the browser and waits", async () => {
    const { calls, sent } = renderAssistant(assistantSnapshot({ agent_id: ARES.id }), {
      agent: hubThatSignsIn({}),
    });
    expect(await screen.findByText("Inicia sesión otra vez en Claude")).toBeTruthy();
    expect(
      screen.getByText(
        "Tus conversaciones con Ares siguen aquí; solo falta tu sesión para que pueda responder.",
      ),
    ).toBeTruthy();
    await userEvent.click(screen.getByRole("button", { name: "Iniciar sesión en Claude" }));
    expect(calls[0]).toEqual({ call: "sign_in", agent_id: ARES.id });
    await waitFor(() =>
      expect(sent).toContainEqual({
        command: "open_url",
        url: "https://claude.example/oauth/authorize",
      }),
    );
    expect(await screen.findByText("Esperando a que inicies sesión en el navegador…")).toBeTruthy();
    expect(
      screen.getByText(
        "Se ha abierto tu navegador. Inicia sesión con tu cuenta de Claude y vuelve aquí.",
      ),
    ).toBeTruthy();
  });

  it("takes Claude's code back from the page, and is done once the hub says it", async () => {
    const { calls, rerender } = renderAssistant(assistantSnapshot({ agent_id: ARES.id }), {
      agent: hubThatSignsIn({}),
    });
    await userEvent.click(await screen.findByRole("button", { name: "Iniciar sesión en Claude" }));
    const field = await screen.findByRole("textbox", { name: "Código de la página" });
    await userEvent.type(field, "abc123#estado9");
    await userEvent.click(screen.getByRole("button", { name: "Enviar código" }));
    expect(calls).toContainEqual({
      call: "sign_in_code",
      agent_id: ARES.id,
      code: "abc123#estado9",
    });
    act(() =>
      rerender(
        assistantSnapshot(
          { agent_id: ARES.id },
          { agents: [DEIMOS, PHOBOS, { ...ARES, account: "signed_in" }] },
        ),
      ),
    );
    await waitFor(() => expect(screen.queryByText(/Inicia sesión otra vez/)).toBeNull());
    expect(screen.queryByText(/no puede responder hasta que inicies sesión/)).toBeNull();
  });

  it("shows Codex's one-time code to type on its page", async () => {
    window.location.hash = `#/assistant/${PHOBOS.id}`;
    renderAssistant(
      assistantSnapshot(
        { agent_id: PHOBOS.id },
        { agents: [DEIMOS, { ...PHOBOS, account: "signed_out" }, ARES] },
      ),
      {
        agent: () => ({
          outcome: "sign_in",
          attempt: attempt({
            agent_id: PHOBOS.id,
            expects_code: false,
            url: "https://auth.example/codex/device",
            code: "ABCD-1234",
          }),
        }),
      },
    );
    await userEvent.click(await screen.findByRole("button", { name: "Iniciar sesión en ChatGPT" }));
    expect(await screen.findByText("Escribe este código en la página: ABCD-1234")).toBeTruthy();
    expect(screen.queryByRole("textbox", { name: "Código de la página" })).toBeNull();
  });

  it("cancels half way and leaves the assistant as it was", async () => {
    const { calls } = renderAssistant(assistantSnapshot({ agent_id: ARES.id }), {
      agent: hubThatSignsIn({
        sign_in_cancel: () => ({ outcome: "sign_in", attempt: attempt({ state: "cancelled" }) }),
      }),
    });
    await userEvent.click(await screen.findByRole("button", { name: "Iniciar sesión en Claude" }));
    const notice = (
      await screen.findByText("Esperando a que inicies sesión en el navegador…")
    ).closest("[role=status]") as HTMLElement;
    await userEvent.click(within(notice).getByRole("button", { name: "Cancelar" }));
    expect(calls).toContainEqual({ call: "sign_in_cancel", agent_id: ARES.id });
    expect(
      await screen.findByText("Cancelaste el inicio de sesión. Ares sigue como estaba."),
    ).toBeTruthy();
    expect(screen.getByRole("button", { name: "Iniciar sesión en Claude" })).toBeTruthy();
  });
});

describe("a sign-in that fails", () => {
  it("says the vendor's cause the hub names", async () => {
    window.location.hash = `#/assistant/${PHOBOS.id}`;
    renderAssistant(
      assistantSnapshot(
        { agent_id: PHOBOS.id },
        { agents: [DEIMOS, { ...PHOBOS, account: "signed_out" }, ARES] },
      ),
      {
        agent: () => ({
          outcome: "sign_in",
          attempt: attempt({ agent_id: PHOBOS.id, state: "failed", cause: "device_code_disabled" }),
        }),
      },
    );
    await userEvent.click(await screen.findByRole("button", { name: "Iniciar sesión en ChatGPT" }));
    expect(
      await screen.findByText(
        "Tu cuenta de ChatGPT no permite iniciar sesión con un código. Actívalo en la seguridad de tu cuenta y vuelve a intentarlo.",
      ),
    ).toBeTruthy();
    expect(screen.getByRole("button", { name: "Volver a intentarlo" })).toBeTruthy();
  });

  it("says when there is no connection with the hub", async () => {
    renderAssistant(assistantSnapshot({ agent_id: ARES.id }), {
      agent: () => ({ outcome: "failed", refusal: { code: "hub_unreachable" } }),
    });
    await userEvent.click(await screen.findByRole("button", { name: "Iniciar sesión en Claude" }));
    expect(
      await screen.findByText(
        "No hay conexión con el hub; los agentes se leen y se cambian en él.",
      ),
    ).toBeTruthy();
  });

  it("warns when the account is already another assistant's", async () => {
    const { rerender } = renderAssistant(assistantSnapshot({ agent_id: ARES.id }), {
      agent: hubThatSignsIn({}),
    });
    await userEvent.click(await screen.findByRole("button", { name: "Iniciar sesión en Claude" }));
    const other = { ...PHOBOS, account_email: "otra@nubaria.example" };
    act(() =>
      rerender(
        assistantSnapshot(
          { agent_id: ARES.id },
          {
            agents: [
              DEIMOS,
              {
                ...other,
                id: "claude-99999999",
                name: "Eris",
                service: "claude-code",
                account_email: "lucia.ferrer@nubaria.example",
              },
              { ...ARES, account: "signed_in" },
            ],
          },
        ),
      ),
    );
    expect(
      await screen.findByText(
        "Esta cuenta ya la usa Eris. Puedes seguir: los dos gastan del mismo plan.",
      ),
    ).toBeTruthy();
  });
});
