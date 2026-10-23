import { act, cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { Agent } from "../../bridge/generated/Agent";
import type { AgentAnswer } from "../../bridge/generated/AgentAnswer";
import type { AgentCall } from "../../bridge/generated/AgentCall";
import type { AgentOutcome } from "../../bridge/generated/AgentOutcome";
import type { AppView } from "../../state/app-view";
import SettingsView from "./SettingsView";

// Ajustes › Asistentes: a summary of the assistants, each one edited from
// its conversation. Names, accounts and folders are invented.

afterEach(() => {
  cleanup();
  window.location.hash = "";
});

const DEIMOS: Agent = { id: "deimos", name: "Deimos", service: "hermes", color: "c1" };
const PHOBOS: Agent = {
  id: "codex-1",
  name: "Phobos",
  service: "codex",
  description: "Código y datos",
  color: "c2",
  account: "signed_in",
  usage: { used_percent: 72, window: "seven_day" },
};
const ARES: Agent = {
  id: "claude-1",
  name: "Ares",
  service: "claude-code",
  color: "c3",
  account: "expired",
};

function snapshot(agents: Agent[] = [DEIMOS, PHOBOS, ARES]): AppView {
  return {
    connection: { daemon: "ready" },
    meeting: { call_preference: null },
    settings: {
      agents,
      default_agent_id: agents.length > 0 ? "deimos" : null,
      addable_agents: ["claude-code", "codex"],
      permissions: {},
      hub: { paired: true, url: "https://hub.example" },
      helper_connected: true,
      versions: { app: "test" },
      update: { state: "idle" },
      privacy_stopped: false,
    },
    computer_control: {
      engine: { state: "ready", tools: 3 },
      history: [],
      folders: { state: "known", max: 8, folders: [] },
    },
  } as unknown as AppView;
}

function hub(answer: AgentOutcome = { outcome: "done" }) {
  const calls: AgentCall[] = [];
  const port = vi.fn(async (call: AgentCall): Promise<AgentAnswer> => {
    calls.push(call);
    return { call, outcome: answer };
  });
  return { calls, port };
}

function show(view = snapshot(), port = hub().port) {
  render(
    <SettingsView
      agent={port}
      error={null}
      onReopenOnboarding={() => undefined}
      section="asistentes"
      send={vi.fn()}
      snapshot={view}
    />,
  );
}

const row = (name: string) =>
  within(screen.getByRole("list", { name: "Tus asistentes" }))
    .getByText(name, { selector: "strong" })
    .closest("li") as HTMLElement;

describe("Ajustes › Asistentes", () => {
  it("says each one is edited from its conversation, with no tab per assistant", () => {
    show();
    expect(screen.getByRole("heading", { level: 2, name: "Asistentes" })).toBeTruthy();
    expect(
      screen.getByText(
        "Cada asistente se edita desde su conversación: el lápiz cambia su nombre, descripción y color; el engranaje abre el resto de sus ajustes.",
      ),
    ).toBeTruthy();
    expect(screen.queryByRole("tablist")).toBeNull();
    expect(screen.queryByRole("region", { name: "Personalidad" })).toBeNull();
  });

  it("one row per assistant with its service and its state", () => {
    show();
    expect(row("Deimos").textContent).toContain(
      "Funciona con Hermes, a través del API Gateway de tu hub",
    );
    expect(within(row("Deimos")).getByText("Principal")).toBeTruthy();
    expect(row("Phobos").textContent).toContain(
      "Funciona con Codex, con tu suscripción de ChatGPT",
    );
    expect(within(row("Phobos")).getByText("72 % del uso de la semana")).toBeTruthy();
    expect(
      within(row("Ares"))
        .getByText("La sesión caducó")
        .closest("[data-tone]")
        ?.getAttribute("data-tone"),
    ).toBe("warn");
  });

  it("«Sus ajustes» opens that assistant's gear, and «Añadir asistente» the four steps", () => {
    show();
    fireEvent.click(within(row("Phobos")).getByRole("button", { name: "Sus ajustes" }));
    expect(window.location.hash).toBe("#/assistant/codex-1/settings");
    fireEvent.click(screen.getByRole("button", { name: "Añadir asistente" }));
    expect(window.location.hash).toBe("#/assistant/add");
  });

  it("the primary is a selector, and choosing another tells the hub", async () => {
    const { calls, port } = hub();
    show(snapshot(), port);
    const primary = screen.getByRole("region", { name: "Asistente principal" });
    expect(
      within(primary).getByText(
        "Responde en el panel flotante, en «Preguntar» de las notas y cuando dices «Oye…» sin nombre. A los demás les llamas por su nombre.",
      ),
    ).toBeTruthy();
    const select = within(primary).getByRole<HTMLSelectElement>("combobox", { name: "Principal" });
    expect(select.value).toBe("deimos");
    expect(Array.from(select.options).map((option) => option.textContent)).toEqual([
      "Deimos",
      "Phobos",
      "Ares",
    ]);
    fireEvent.change(select, { target: { value: "codex-1" } });
    await act(async () => undefined);
    expect(calls).toEqual([
      {
        call: "profile",
        agent_id: "codex-1",
        name: "Phobos",
        description: "Código y datos",
        color: "c2",
        primary: true,
      },
    ]);
  });

  it("a primary the hub refuses says why and the selector stays as it was", async () => {
    const { port } = hub({ outcome: "failed", refusal: { code: "not_paired" } });
    show(snapshot(), port);
    const select = screen.getByRole<HTMLSelectElement>("combobox", { name: "Principal" });
    fireEvent.change(select, { target: { value: "codex-1" } });
    await act(async () => undefined);
    expect(screen.getByRole("alert").textContent).toBe(
      "Este Mac no está emparejado con el hub. Emparéjalo en Ajustes.",
    );
    expect(select.value).toBe("deimos");
  });

  it("the folders are the same for all, and what they did says why it has no avatars", () => {
    show();
    const folders = screen.getByRole("region", { name: "Carpetas que pueden leer" });
    expect(
      within(folders).getByText(
        "Las mismas para todos tus asistentes. Pueden leer estos archivos, nunca cambiarlos.",
      ),
    ).toBeTruthy();
    const history = screen.getByRole("region", { name: "Lo que han hecho en este Mac" });
    expect(
      within(history).getByText(
        "Tu hub todavía no dice qué asistente hizo cada cosa: aquí está lo de todos.",
      ),
    ).toBeTruthy();
  });

  it("before the hub says who they are, says so instead of an empty list", () => {
    show(snapshot([]));
    expect(
      screen.getByText("Todavía no se conocen los agentes del hub: aparecen al conectar con él."),
    ).toBeTruthy();
    expect(screen.queryByRole("combobox")).toBeNull();
  });
});
