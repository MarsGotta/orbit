import { act, cleanup, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Agent } from "../../bridge/generated/Agent";
import type { AgentCall } from "../../bridge/generated/AgentCall";
import type { AgentOutcome } from "../../bridge/generated/AgentOutcome";
import { assistantSnapshot, NOW, renderAssistant, THREE } from "./test-assistants";

// «Añadir un asistente» (annex 40, 40a–40c): what it works with, signing
// in, its name, colour and what it may use, and «… está listo».

beforeEach(() => {
  vi.useFakeTimers({ now: NOW, toFake: ["Date"] });
  window.location.hash = "#/assistant/add";
});

afterEach(() => {
  cleanup();
  vi.useRealTimers();
  window.location.hash = "";
});

const NEW_ID = "claude-0a0b0c0d";

function hub(call: AgentCall): AgentOutcome {
  if (call.call === "add") return { outcome: "added", agent_id: NEW_ID };
  if (call.call === "sign_in")
    return {
      outcome: "sign_in",
      attempt: {
        agent_id: NEW_ID,
        state: "waiting",
        expects_code: true,
        url: "https://claude.example/oauth/authorize",
      },
    };
  return { outcome: "done" };
}

const added: Agent = {
  id: NEW_ID,
  name: "Claude",
  service: "claude-code",
  color: "c4",
  removable: true,
  account: "signed_in",
  account_email: "lucia.ferrer@nubaria.example",
  plan: "Claude Pro",
};

function dialog() {
  return screen.getByRole("dialog", { name: /Añadir un asistente/ });
}

describe("«Añadir un asistente»", () => {
  it("walks the four steps: kind, sign in, name and ready", async () => {
    const { calls, sent, rerender } = renderAssistant(assistantSnapshot(), { agent: hub });
    expect(within(dialog()).getByText("Añadir un asistente · paso 1 de 4")).toBeTruthy();
    const kinds = within(dialog()).getByRole("radiogroup", { name: "Servicio" });
    expect(
      within(kinds)
        .getByRole("radio", { name: /Tu suscripción de Claude/ })
        .getAttribute("aria-checked"),
    ).toBe("true");
    await userEvent.click(within(dialog()).getByRole("button", { name: "Continuar" }));

    expect(within(dialog()).getByText("Conecta tu cuenta de Claude")).toBeTruthy();
    expect(
      within(dialog()).getByText(
        "Se abre tu navegador. Orbit no ve tu contraseña: solo recibe permiso para usar tu plan.",
      ),
    ).toBeTruthy();
    expect(
      (within(dialog()).getByRole("button", { name: "Continuar" }) as HTMLButtonElement).disabled,
    ).toBe(true);
    await userEvent.click(
      within(dialog()).getByRole("button", { name: "Iniciar sesión en Claude" }),
    );
    // The hub makes the assistant, signed out, and signs it in.
    expect(calls[0]).toMatchObject({ call: "add", service: "claude-code", color: "c4" });
    await waitFor(() => expect(calls).toContainEqual({ call: "sign_in", agent_id: NEW_ID }));
    expect(sent).toContainEqual({
      command: "open_url",
      url: "https://claude.example/oauth/authorize",
    });
    act(() => rerender(assistantSnapshot({}, { agents: [...THREE, added] })));
    expect(await within(dialog()).findByText("✓ Conectado")).toBeTruthy();
    expect(
      within(dialog()).getByText("Como lucia.ferrer@nubaria.example · Claude Pro"),
    ).toBeTruthy();
    await userEvent.click(within(dialog()).getByRole("button", { name: "Continuar" }));

    expect(within(dialog()).getByText("Ponle nombre")).toBeTruthy();
    const name = within(dialog()).getByRole("textbox", { name: "Nombre" });
    await userEvent.clear(name);
    await userEvent.type(name, "Eris");
    expect(
      within(dialog()).getByText("Así le llamarás en Orbit y en las reuniones («Oye Eris, …»)."),
    ).toBeTruthy();
    await userEvent.click(within(dialog()).getByRole("switch", { name: "Pantalla" }));
    await userEvent.click(within(dialog()).getByRole("button", { name: "Crear asistente" }));
    expect(calls).toContainEqual({
      call: "profile",
      agent_id: NEW_ID,
      name: "Eris",
      description: "",
      color: "c4",
      primary: false,
    });
    expect(calls).toContainEqual({
      call: "access",
      agent_id: NEW_ID,
      groups: { screen: true, control: false, calendar: true, meetings: true, files: true },
    });

    expect(await within(dialog()).findByText("Eris está listo")).toBeTruthy();
    await userEvent.click(within(dialog()).getByRole("button", { name: "Hablar con Eris" }));
    expect(window.location.hash).toBe(`#/assistant/${NEW_ID}`);
  });

  it("asks the hub's manager for a Hermes assistant", async () => {
    renderAssistant(assistantSnapshot(), { agent: hub });
    await userEvent.click(within(dialog()).getByRole("radio", { name: /Hermes en tu hub/ }));
    await userEvent.click(within(dialog()).getByRole("button", { name: "Continuar" }));
    expect(within(dialog()).getByText("Pídeselo a quien gestiona tu hub")).toBeTruthy();
    expect(
      (within(dialog()).getByRole("button", { name: "Continuar" }) as HTMLButtonElement).disabled,
    ).toBe(true);
  });

  it("says when the hub has no adapter for a service", async () => {
    renderAssistant(assistantSnapshot({}, { addable: ["codex"] }), { agent: hub });
    expect(
      within(dialog()).getByText(
        "Tu hub no tiene instalado este servicio. Pídeselo a quien gestiona tu hub.",
      ),
    ).toBeTruthy();
    expect(
      (within(dialog()).getByRole("button", { name: "Continuar" }) as HTMLButtonElement).disabled,
    ).toBe(true);
  });

  it("cancelled half way, removes the assistant it began and closes", async () => {
    const { calls } = renderAssistant(assistantSnapshot(), { agent: hub });
    await userEvent.click(within(dialog()).getByRole("button", { name: "Continuar" }));
    await userEvent.click(
      within(dialog()).getByRole("button", { name: "Iniciar sesión en Claude" }),
    );
    await waitFor(() => expect(calls).toContainEqual({ call: "sign_in", agent_id: NEW_ID }));
    await userEvent.click(
      within(dialog()).getAllByRole("button", { name: "Cancelar" }).at(-1) as HTMLElement,
    );
    await waitFor(() => expect(calls).toContainEqual({ call: "remove", agent_id: NEW_ID }));
    expect(calls).toContainEqual({ call: "sign_in_cancel", agent_id: NEW_ID });
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(window.location.hash).toBe("#/assistant");
  });
});
