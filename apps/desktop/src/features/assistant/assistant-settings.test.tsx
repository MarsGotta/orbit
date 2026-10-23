import { cleanup, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { AgentCall } from "../../bridge/generated/AgentCall";
import type { AgentConfig } from "../../bridge/generated/AgentConfig";
import type { AgentOutcome } from "../../bridge/generated/AgentOutcome";
import {
  ARES,
  assistantSnapshot,
  at,
  DEIMOS,
  NOW,
  PHOBOS,
  renderAssistant,
} from "./test-assistants";

// The gear (annex 37–39): an assistant's own settings page, with what
// Ajustes › Asistente showed for it, its connection and its plan's usage.

beforeEach(() => {
  vi.useFakeTimers({ now: NOW, toFake: ["Date"] });
});

afterEach(() => {
  cleanup();
  vi.useRealTimers();
  window.location.hash = "";
});

function config(agentId: string, editable: boolean): AgentConfig {
  return {
    agent_id: agentId,
    kind: editable ? "acp" : "hermes",
    personality: {
      text: "Explica cada cambio antes de hacerlo.",
      editable,
      max_bytes: 16_384,
      cause: editable ? null : "La personalidad es el SOUL.md del perfil de Hermes",
    },
    access: {
      groups: { screen: false, control: false, calendar: true, meetings: true, files: true },
      editable,
      cause: editable ? null : "Hermes decide sus herramientas",
    },
    skills: { editable, cause: null, max_skills: 128, max_skill_bytes: 65_536, items: [] },
  };
}

function hub(call: AgentCall): AgentOutcome {
  if (call.call === "config")
    return { outcome: "config", config: config(call.agent_id, call.agent_id !== DEIMOS.id) };
  return { outcome: "done" };
}

function openSettings(agent: typeof PHOBOS, agents = [DEIMOS, PHOBOS, ARES]) {
  window.location.hash = `#/assistant/${agent.id}/settings`;
  return renderAssistant(assistantSnapshot({ agent_id: agent.id }, { agents }), { agent: hub });
}

describe("an assistant's settings", () => {
  it("leads back to its conversation from the top bar", async () => {
    openSettings(PHOBOS);
    const back = within(screen.getByTestId("top-bar")).getByRole("link", {
      name: "Conversación con Phobos",
    });
    await userEvent.click(back);
    expect(window.location.hash).toBe(`#/assistant/${PHOBOS.id}`);
  });

  it("shows its connection, account and plan's usage, amber from 80 %", async () => {
    const usage = { used_percent: 84, window: "seven_day", resets_at_ms: at(26, 9, 0) };
    openSettings(PHOBOS, [DEIMOS, { ...PHOBOS, usage }, ARES]);
    expect(await screen.findByRole("heading", { name: "Ajustes de Phobos" })).toBeTruthy();
    const connection = screen.getByRole("region", { name: "Conexión" });
    expect(within(connection).getByText("Codex")).toBeTruthy();
    expect(
      within(connection).getByText("lucia.ferrer@nubaria.example · ChatGPT Plus"),
    ).toBeTruthy();
    expect(within(connection).getByText("Conectado")).toBeTruthy();
    const bar = within(connection).getByRole("progressbar", { name: "Uso de tu plan" });
    expect(bar.getAttribute("aria-valuenow")).toBe("84");
    expect(bar.getAttribute("data-tone")).toBe("warn");
    expect(within(connection).getByText("84 % del uso de esta semana")).toBeTruthy();
    expect(within(connection).getByText("Se renueva el lunes 26")).toBeTruthy();
    expect(within(connection).getByRole("button", { name: "Ver mi plan de ChatGPT" })).toBeTruthy();
  });

  it("draws no usage bar when the provider gave none", async () => {
    openSettings(PHOBOS);
    const connection = await screen.findByRole("region", { name: "Conexión" });
    expect(within(connection).queryByRole("progressbar")).toBeNull();
  });

  it("signs it out, and it stays among «Tus asistentes»", async () => {
    const { calls } = openSettings(PHOBOS);
    await userEvent.click(await screen.findByRole("button", { name: "Cerrar sesión" }));
    expect(calls).toContainEqual({ call: "sign_out", agent_id: PHOBOS.id });
    expect(
      await screen.findByText(
        "Has cerrado la sesión. Phobos sigue en «Tus asistentes» con sus conversaciones; vuelve a iniciarla para escribirle.",
      ),
    ).toBeTruthy();
  });

  it("gathers its personality, what it may use, its skills and «En Orbit»", async () => {
    openSettings(PHOBOS);
    expect(await screen.findByRole("region", { name: "Personalidad" })).toBeTruthy();
    expect(screen.getByRole("region", { name: "Qué puede usar" })).toBeTruthy();
    expect(screen.getByRole("region", { name: "Habilidades" })).toBeTruthy();
    expect(screen.getByText("Di «Oye Phobos, …» y te responde en el panel flotante.")).toBeTruthy();
    expect(
      screen.getByText("No tiene voz propia: Orbit lee sus respuestas con la voz de Deimos."),
    ).toBeTruthy();
  });

  it("removes one added from Orbit after asking in its row", async () => {
    const { calls } = openSettings(PHOBOS);
    await userEvent.click(await screen.findByRole("button", { name: "Quitar a Phobos de Orbit" }));
    expect(screen.getByText("¿Quitar a Phobos?")).toBeTruthy();
    expect(
      screen.getByText(
        "Sus conversaciones se quedan, pero ya no podrás escribirle. Tu suscripción no cambia.",
      ),
    ).toBeTruthy();
    expect(calls.some((call) => call.call === "remove")).toBe(false);
    await userEvent.click(screen.getByRole("button", { name: "Quitar" }));
    expect(calls).toContainEqual({ call: "remove", agent_id: PHOBOS.id });
    await waitFor(() => expect(window.location.hash).toBe("#/assistant"));
  });

  it("cannot remove Deimos, which the hub gives, and shows its settings read-only", async () => {
    openSettings(DEIMOS);
    expect(
      await screen.findByText("Deimos lo da tu hub: solo quien lo gestiona puede quitarlo."),
    ).toBeTruthy();
    expect(screen.queryByRole("button", { name: /Quitar a Deimos/ })).toBeNull();
    const access = await screen.findByRole("region", { name: "Qué puede usar" });
    for (const toggle of within(access).getAllByRole("switch"))
      expect((toggle as HTMLButtonElement).disabled).toBe(true);
  });

  it("asks to sign in again in its connection when the session expired", async () => {
    openSettings(ARES);
    const connection = await screen.findByRole("region", { name: "Conexión" });
    expect(within(connection).getByText("La sesión de Claude caducó")).toBeTruthy();
    expect(within(connection).getByText("Sesión caducada")).toBeTruthy();
    expect(
      within(connection).getByRole("button", { name: "Iniciar sesión en Claude" }),
    ).toBeTruthy();
  });
});
