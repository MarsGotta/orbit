import { act, cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { AppCommand } from "../../bridge/generated";
import type { Agent } from "../../bridge/generated/Agent";
import type { AgentAnswer } from "../../bridge/generated/AgentAnswer";
import type { AgentCall } from "../../bridge/generated/AgentCall";
import type { AgentOutcome } from "../../bridge/generated/AgentOutcome";
import type { PermissionSnapshot } from "../../bridge/generated/PermissionSnapshot";
import type { AppView } from "../../state/app-view";
import { OnboardingView } from "./OnboardingView";

// Step 4, «Tus asistentes», and step 5, «Todo listo». The company, people
// and assistants are invented.

afterEach(cleanup);

const DEIMOS: Agent = { id: "deimos", name: "Deimos", service: "hermes", color: "c1" };
const PHOBOS: Agent = {
  id: "codex-1",
  name: "Phobos",
  service: "codex",
  description: "Código y datos",
  color: "c2",
  account: "signed_in",
  account_email: "lucia.ferrer@nubaria.example",
  plan: "ChatGPT Plus",
};
const ARES: Agent = {
  id: "claude-1",
  name: "Ares",
  service: "claude-code",
  color: "c3",
  account: "expired",
};

const granted: PermissionSnapshot = {
  microphone: "granted",
  screen: "granted",
  accessibility: "granted",
  calendars: "denied",
};

function snapshot({
  paired = true,
  agents = [DEIMOS, PHOBOS, ARES],
  primary = "deimos",
  permissions = granted,
}: {
  paired?: boolean;
  agents?: Agent[];
  primary?: string;
  permissions?: PermissionSnapshot;
} = {}): AppView {
  return {
    settings: {
      hub: {
        url: paired ? "https://hub.nubaria.example" : null,
        paired,
        device_name: paired ? "Mac de Lucía" : null,
      },
      permissions,
      agents,
      default_agent_id: paired ? primary : null,
    },
  } as unknown as AppView;
}

function hub(answers: Partial<Record<AgentCall["call"], AgentOutcome>> = {}) {
  const calls: AgentCall[] = [];
  const port = vi.fn(async (call: AgentCall): Promise<AgentAnswer> => {
    calls.push(call);
    return { call, outcome: answers[call.call] ?? { outcome: "done" } };
  });
  return { calls, port };
}

function show(view: AppView, port = hub().port) {
  const sent: AppCommand[] = [];
  const send = vi.fn(async (command: AppCommand) => {
    sent.push(command);
  });
  const rendered = render(
    <OnboardingView agent={port} error={null} onComplete={vi.fn()} send={send} snapshot={view} />,
  );
  return { ...rendered, sent, send };
}

const press = (name: string) => fireEvent.click(screen.getByRole("button", { name }));
const progress = () => screen.getByText(/^Paso \d de 5 · /).textContent;

function toAssistants() {
  press("Empezar");
  press(screen.queryByRole("button", { name: "Continuar" }) ? "Continuar" : "Emparejar después");
  press("Continuar");
  expect(progress()).toBe("Paso 4 de 5 · Asistentes");
}

const row = (name: string) =>
  within(screen.getByRole("list", { name: "Tus asistentes" }))
    .getByText(name, { selector: "strong" })
    .closest("li") as HTMLElement;

describe("«Tus asistentes»", () => {
  it("says each one's service and state: ready with the hub, connected, or to sign in", () => {
    show(snapshot());
    toAssistants();
    expect(screen.getByRole("heading", { name: "Tus asistentes" })).toBeTruthy();
    expect(row("Deimos").textContent).toContain("Hermes, a través del API Gateway de tu hub");
    expect(within(row("Deimos")).getByText("✓ Listo con tu hub")).toBeTruthy();
    expect(row("Phobos").textContent).toContain("Codex, con tu suscripción de ChatGPT");
    expect(
      within(row("Phobos")).getByText("lucia.ferrer@nubaria.example · ChatGPT Plus"),
    ).toBeTruthy();
    expect(within(row("Phobos")).getByText("✓ Conectado")).toBeTruthy();
    expect(
      within(row("Ares")).getByRole("button", { name: "Iniciar sesión con Claude" }),
    ).toBeTruthy();
    expect(
      screen.getByText(
        "Puedes seguir sin alguno. Añades más o cambias sus cuentas cuando quieras, desde Asistente.",
      ),
    ).toBeTruthy();
  });

  it("without the hub, its assistant needs it and «Emparejar el hub» goes back to step 2", () => {
    show(snapshot({ paired: false, agents: [] }));
    toAssistants();
    expect(within(row("Deimos")).getByText("Necesita tu hub")).toBeTruthy();
    expect(screen.getByText("Conecta al menos uno para elegir el principal.")).toBeTruthy();
    expect(screen.queryByRole("radiogroup")).toBeNull();
    fireEvent.click(within(row("Deimos")).getByRole("button", { name: "Emparejar el hub" }));
    expect(progress()).toBe("Paso 2 de 5 · Tu hub");
  });

  it("signs a subscription in through the hub: the browser opens and the row waits for it", async () => {
    const { calls, port } = hub({
      sign_in: {
        outcome: "sign_in",
        attempt: {
          agent_id: ARES.id,
          state: "waiting",
          expects_code: true,
          url: "https://claude.example/oauth/authorize",
          expires_at_ms: 0,
        },
      },
    });
    const view = show(snapshot(), port);
    toAssistants();
    fireEvent.click(within(row("Ares")).getByRole("button", { name: "Iniciar sesión con Claude" }));
    await act(async () => undefined);
    expect(calls).toContainEqual({ call: "sign_in", agent_id: ARES.id });
    expect(view.sent).toContainEqual({
      command: "open_url",
      url: "https://claude.example/oauth/authorize",
    });
    expect(within(row("Ares")).getByText("Esperando al navegador…")).toBeTruthy();
    expect(
      within(row("Ares")).getByText(
        "Se ha abierto tu navegador. Inicia sesión con tu cuenta de Claude y vuelve aquí.",
      ),
    ).toBeTruthy();
    // The hub says the account is signed in: the row says so.
    view.rerender(
      <OnboardingView
        agent={port}
        error={null}
        onComplete={vi.fn()}
        send={view.send}
        snapshot={snapshot({ agents: [DEIMOS, PHOBOS, { ...ARES, account: "signed_in" }] })}
      />,
    );
    expect(within(row("Ares")).getByText("✓ Conectado")).toBeTruthy();
  });

  it("chooses the primary among those that are ready, and the hub keeps it", async () => {
    const { calls, port } = hub();
    show(snapshot(), port);
    toAssistants();
    const choice = screen.getByRole("radiogroup", { name: "Asistente principal" });
    expect(
      within(choice)
        .getAllByRole("radio")
        .map((radio) => [radio.textContent, radio.getAttribute("aria-checked")]),
    ).toEqual([
      ["DDeimos", "true"],
      ["PPhobos", "false"],
    ]);
    expect(
      screen.getByText(
        "Responde a «Oye…» sin nombre y en el panel flotante. A los demás les llamas por su nombre: «Oye Phobos, …».",
      ),
    ).toBeTruthy();
    fireEvent.click(within(choice).getByRole("radio", { name: "Phobos" }));
    await act(async () => undefined);
    expect(calls).toContainEqual({
      call: "profile",
      agent_id: PHOBOS.id,
      name: "Phobos",
      description: "Código y datos",
      color: "c2",
      primary: true,
    });
    expect(within(choice).getByRole("radio", { name: "Phobos" }).getAttribute("aria-checked")).toBe(
      "true",
    );
  });

  it("a primary the hub refuses says why and stays as it was", async () => {
    const { port } = hub({ profile: { outcome: "failed", refusal: { code: "not_paired" } } });
    show(snapshot(), port);
    toAssistants();
    fireEvent.click(screen.getByRole("radio", { name: "Phobos" }));
    await act(async () => undefined);
    expect(screen.getByRole("alert").textContent).toBe(
      "Este Mac no está emparejado con el hub. Emparéjalo en Ajustes.",
    );
    expect(screen.getByRole("radio", { name: "Deimos" }).getAttribute("aria-checked")).toBe("true");
  });
});

describe("«Todo listo» sums up", () => {
  function summary() {
    const list = screen.getByRole("list", { name: "Lo que has configurado" });
    return within(list)
      .getAllByRole("listitem")
      .map((item) => [item.textContent, item.getAttribute("data-tone")]);
  }

  it("what is configured, in green, and what is missing, in amber", () => {
    show(snapshot());
    toAssistants();
    press("Continuar");
    expect(summary()).toEqual([
      ["Tu hubhub.nubaria.example · este Mac se llama «Mac de Lucía»", "ok"],
      ["Permisos3 de 4 concedidos · falta Calendarios", "warn"],
      ["AsistentesDeimos (principal) y Phobos", "ok"],
    ]);
    expect(screen.getByText("Hablar con Deimos")).toBeTruthy();
  });

  it("a Mac with nothing set up yet says each thing it lacks", () => {
    show(
      snapshot({
        paired: false,
        agents: [],
        permissions: {
          microphone: "not_determined",
          screen: "not_determined",
          accessibility: "not_granted",
          calendars: "not_determined",
        },
      }),
    );
    toAssistants();
    press("Continuar");
    expect(summary()).toEqual([
      ["Tu hubSin emparejar. Hazlo cuando quieras en Ajustes › General.", "warn"],
      [
        "Permisos0 de 4 concedidos · falta Micrófono, Pantalla y audio del sistema, Accesibilidad y Calendarios",
        "warn",
      ],
      ["AsistentesNinguno conectado todavía", "warn"],
    ]);
  });

  it("calls the primary by its name in the shortcut", () => {
    show(snapshot({ primary: PHOBOS.id }));
    toAssistants();
    press("Continuar");
    expect(summary()[2]).toEqual(["AsistentesDeimos y Phobos (principal)", "ok"]);
    expect(screen.getByText("Hablar con Phobos")).toBeTruthy();
  });
});
