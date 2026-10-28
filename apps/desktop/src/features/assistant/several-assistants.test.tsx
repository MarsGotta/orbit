import { act, cleanup, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { assistantHash, parseAssistantRoute } from "./assistant-route";
import { rememberFirstLine } from "./first-lines";
import {
  ARES,
  assistantSnapshot,
  at,
  DEIMOS,
  NOW,
  PHOBOS,
  renderAssistant,
  summary,
} from "./test-assistants";

// The Assistant with several assistants (annex 1, changes 1 and 2): the
// column starts with «Tus asistentes», «+ Nueva» asks with whom, each
// assistant opens its own conversation, and the header lives in the top bar.

beforeEach(() => {
  vi.useFakeTimers({ now: NOW, toFake: ["Date"] });
  window.location.hash = "#/assistant";
});

afterEach(() => {
  cleanup();
  vi.useRealTimers();
  window.location.hash = "";
});

/** What a row says on screen, without the avatar's initial, which is decoration. */
function visibleText(element: HTMLElement): string {
  const copy = element.cloneNode(true) as HTMLElement;
  for (const hidden of copy.querySelectorAll("[aria-hidden=true]")) hidden.remove();
  return copy.textContent ?? "";
}

const SESSIONS = [
  summary("s-llamada", at(21, 9, 31), DEIMOS.id),
  summary("s-tarifas", at(21, 12, 10) - 60 * 60_000, PHOBOS.id),
  summary("s-contrato", at(20, 16, 0), ARES.id),
  summary("s-script", at(18, 18, 2), PHOBOS.id, 8),
];

describe("the routes of the Asistente", () => {
  it("names each screen of guide 6 and reads it back", () => {
    const routes = [
      { screen: "conversation" },
      { screen: "conversation", sessionId: "s-1" },
      { screen: "new" },
      { screen: "add" },
      { screen: "agent", agentId: PHOBOS.id },
      { screen: "settings", agentId: PHOBOS.id },
    ] as const;
    expect(routes.map(assistantHash)).toEqual([
      "#/assistant",
      "#/assistant/c/s-1",
      "#/assistant/new",
      "#/assistant/add",
      "#/assistant/codex-1a2b3c4d",
      "#/assistant/codex-1a2b3c4d/settings",
    ]);
    for (const route of routes) expect(parseAssistantRoute(assistantHash(route))).toEqual(route);
  });
});

describe("«Tus asistentes»", () => {
  it("lists each assistant with its service, and a warning only when something is wrong", async () => {
    renderAssistant(
      assistantSnapshot(
        {},
        { agents: [DEIMOS, { ...PHOBOS, limit: { until_ms: at(21, 18, 40) } }, ARES] },
      ),
    );
    const yours = await screen.findByRole("list", { name: "Tus asistentes" });
    const rows = within(yours).getAllByRole("link");
    expect(rows.map(visibleText)).toEqual([
      "DeimosHermes · API Gateway",
      "PhobosCodex · tu suscripciónHasta 18:40",
      "AresClaude · tu suscripciónInicia sesión",
    ]);
  });

  it("opens an assistant's own conversation, a new one with it", async () => {
    const { sent } = renderAssistant(assistantSnapshot({ session_id: "s-llamada" }));
    await userEvent.click(await screen.findByRole("link", { name: /^Phobos/ }));
    expect(window.location.hash).toBe("#/assistant/codex-1a2b3c4d");
    await waitFor(() =>
      expect(sent).toContainEqual({ command: "new_session", agent_id: PHOBOS.id }),
    );
  });

  it("opens «Añadir un asistente» from its «+»", async () => {
    renderAssistant(assistantSnapshot());
    await userEvent.click(await screen.findByRole("link", { name: "Añadir asistente" }));
    expect(window.location.hash).toBe("#/assistant/add");
  });
});

describe("«Conversaciones», by date", () => {
  it("groups them by day, each with its assistant, and keeps the start in its name", async () => {
    rememberFirstLine("s-llamada", "Prepárame la llamada con Ribera. Y lo pendiente.");
    renderAssistant(assistantSnapshot({ session_id: "s-llamada" }), { sessions: SESSIONS });
    const today = await screen.findByRole("list", { name: "Hoy" });
    const rows = within(today).getAllByRole("link");
    expect(rows[0]?.getAttribute("aria-label")).toBe(
      "Prepárame la llamada con Ribera. · Hoy, 09:31 · con Deimos",
    );
    expect(rows[0]?.getAttribute("aria-current")).toBe("page");
    expect(rows[1]?.textContent).toContain("Phobos");
    expect(screen.getByRole("list", { name: "Ayer" }).textContent).toContain("Ares");
    expect(screen.getByRole("list", { name: "18 oct" }).textContent).toContain("8 mensajes");
  });

  it("opens one with its assistant and names it in the route", async () => {
    const { sent } = renderAssistant(assistantSnapshot({ session_id: "s-llamada" }), {
      sessions: SESSIONS,
    });
    const yesterday = await screen.findByRole("list", { name: "Ayer" });
    await userEvent.click(within(yesterday).getByRole("link"));
    expect(sent).toContainEqual({ command: "session_open", id: "s-contrato", agent_id: ARES.id });
    expect(window.location.hash).toBe("#/assistant/c/s-contrato");
  });
});

describe("«+ Nueva»: ¿Con quién quieres hablar?", () => {
  it("from an open conversation, begins a new one and asks with whom", async () => {
    const { sent } = renderAssistant(assistantSnapshot({ session_id: "s-llamada" }));
    await userEvent.click(screen.getByRole("button", { name: "Nueva conversación" }));
    expect(window.location.hash).toBe("#/assistant/new");
    await waitFor(() => expect(sent).toContainEqual({ command: "new_session" }));
  });

  it("offers a card per assistant, the primary first and chosen, and changes until the first message", async () => {
    window.location.hash = "#/assistant/new";
    const { sent } = renderAssistant(assistantSnapshot());
    expect(await screen.findByRole("heading", { name: "¿Con quién quieres hablar?" })).toBeTruthy();
    const cards = screen.getByRole("radiogroup", { name: "Hablar con" });
    const options = within(cards).getAllByRole("radio");
    expect(options.map((option) => option.getAttribute("aria-checked"))).toEqual([
      "true",
      "false",
      "false",
    ]);
    expect(options[0]?.textContent).toContain("Principal");
    expect(options[1]?.textContent).toContain("Codex · tu suscripción");
    expect(options[2]?.textContent).toContain("La sesión de Claude caducó");
    await userEvent.click(options[1] as HTMLElement);
    expect(sent).toContainEqual({ command: "new_session", agent_id: PHOBOS.id });
  });

  it("with an assistant whose session expired, says so and keeps the text without sending it", async () => {
    window.location.hash = "#/assistant/new";
    renderAssistant(assistantSnapshot({ agent_id: ARES.id }));
    expect(await screen.findByText("Inicia sesión otra vez en Claude")).toBeTruthy();
    const box = screen.getByRole("textbox", { name: "Mensaje para Ares" });
    await userEvent.type(box, "Revisa el contrato");
    expect((box as HTMLTextAreaElement).value).toBe("Revisa el contrato");
    expect((screen.getByRole("button", { name: "Enviar" }) as HTMLButtonElement).disabled).toBe(
      true,
    );
    expect(
      screen.getByText(
        "Ares no puede responder hasta que inicies sesión en Claude. Lo que escribas se queda aquí.",
      ),
    ).toBeTruthy();
  });
});

describe("an assistant's own conversation", () => {
  it("shows who it is, «Para empezar» that fills the box and «Seguir donde lo dejaste»", async () => {
    rememberFirstLine("s-script", "Un script para exportar las tarifas.");
    window.location.hash = `#/assistant/${PHOBOS.id}`;
    const { sent } = renderAssistant(
      assistantSnapshot(
        { agent_id: PHOBOS.id },
        {
          agents: [DEIMOS, { ...PHOBOS, usage: { used_percent: 72, window: "seven_day" } }, ARES],
        },
      ),
      { sessions: SESSIONS },
    );
    expect(await screen.findByRole("heading", { level: 1, name: "Phobos" })).toBeTruthy();
    expect(screen.getByText("Código, scripts y datos")).toBeTruthy();
    expect(screen.getByText("Disponible · 72 % del uso de la semana")).toBeTruthy();
    const starters = screen.getByRole("group", { name: "Para empezar" });
    const [first] = within(starters).getAllByRole("button");
    await userEvent.click(first as HTMLElement);
    expect(
      (screen.getByRole("textbox", { name: "Mensaje para Phobos" }) as HTMLTextAreaElement).value,
    ).toBe(first?.textContent);
    const earlier = await screen.findByRole("list", { name: "Seguir donde lo dejaste" });
    const rows = within(earlier).getAllByRole("link");
    expect(rows).toHaveLength(2);
    expect(rows[1]?.textContent).toContain("Un script para exportar las tarifas.");
    await userEvent.click(rows[1] as HTMLElement);
    expect(sent).toContainEqual({ command: "session_open", id: "s-script", agent_id: PHOBOS.id });
  });

  it("takes the route of its conversation once it has one", async () => {
    window.location.hash = `#/assistant/${PHOBOS.id}`;
    const { rerender } = renderAssistant(assistantSnapshot({ agent_id: PHOBOS.id }));
    await screen.findByRole("heading", { level: 1, name: "Phobos" });
    act(() => rerender(assistantSnapshot({ agent_id: PHOBOS.id, session_id: "s-nueva" })));
    await waitFor(() => expect(window.location.hash).toBe("#/assistant/c/s-nueva"));
  });
});

describe("the header, in the window's top bar", () => {
  it("says with whom, its service and its state, with the pencil and the gear, in one bar", async () => {
    renderAssistant(
      assistantSnapshot({
        session_id: "s-tarifas",
        agent_id: PHOBOS.id,
        pending: { kind: "approval", id: "p1", text: "Ejecutar el script", choices: [] },
      }),
    );
    const bar = screen.getByTestId("top-bar");
    expect(within(bar).getByRole("heading", { name: "Phobos" })).toBeTruthy();
    expect(within(bar).getByText("Codex · tu suscripción")).toBeTruthy();
    expect(within(bar).getByText("espera tu permiso")).toBeTruthy();
    expect(
      within(bar).getByRole("button", { name: "Editar nombre y descripción de Phobos" }),
    ).toBeTruthy();
    // The view draws no second bar of its own: the name is a heading once, up there.
    expect(screen.getAllByRole("heading", { name: "Phobos" })).toHaveLength(1);
    await userEvent.click(within(bar).getByRole("button", { name: "Ajustes de Phobos" }));
    expect(window.location.hash).toBe("#/assistant/codex-1a2b3c4d/settings");
  });

  it("says the limit and its time in amber", () => {
    renderAssistant(
      assistantSnapshot(
        { session_id: "s-tarifas", agent_id: PHOBOS.id },
        { agents: [DEIMOS, { ...PHOBOS, limit: { until_ms: at(21, 18, 40) } }, ARES] },
      ),
    );
    expect(
      within(screen.getByTestId("top-bar")).getByText("límite alcanzado · hasta las 18:40"),
    ).toBeTruthy();
  });
});
