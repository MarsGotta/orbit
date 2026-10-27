import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import type { AgentPort } from "./agents/use-agent-config";
import { AISettings } from "./AISettings";
import { AIStep } from "../onboarding/OnboardingAIStep";

afterEach(cleanup);
const settings = {
  providers: [
    {
      id: "hub",
      name: "En tu hub",
      kind: "hub",
      base_url: null,
      key_configured: false,
      models: [{ id: "whisper", capabilities: ["transcription", "word_timestamps"] }],
    },
  ],
  functions: [
    {
      id: "meeting_transcription",
      provider_id: "hub",
      model: "whisper",
      capability: "word_timestamps",
      privacy: "El audio se queda en tu hub.",
    },
  ],
};
const port = () =>
  vi.fn(async (call) => ({ call, outcome: { outcome: "ai_settings", settings } })) as AgentPort;

it("shows function privacy and submits a provider without keeping its key", async () => {
  const agent = port();
  render(<AISettings agent={agent} />);
  expect(await screen.findByText("El audio se queda en tu hub.")).toBeTruthy();
  fireEvent.click(screen.getByRole("button", { name: "Añadir un proveedor" }));
  fireEvent.change(screen.getByLabelText("Nombre del proveedor"), { target: { value: "Cedro" } });
  fireEvent.change(screen.getByLabelText("URL base"), {
    target: { value: "http://127.0.0.1:8080/v1" },
  });
  fireEvent.change(screen.getByLabelText("Clave (opcional)"), {
    target: { value: "invented-key" },
  });
  fireEvent.click(screen.getByRole("button", { name: "Comprobar y guardar" }));
  expect(await screen.findByText("Guardado en el hub")).toBeTruthy();
  expect(agent).toHaveBeenLastCalledWith(
    expect.objectContaining({
      call: "ai_provider_save",
      provider: expect.objectContaining({ name: "Cedro", api_key: "invented-key" }),
    }),
  );
  expect(screen.queryByDisplayValue("invented-key")).toBeNull();
});

it("onboarding defaults to hub without adding anything and opens the same provider form", () => {
  const agent = port();
  render(<AIStep agent={agent} />);
  expect((screen.getByRole("radio", { name: "Todo en tu hub" }) as HTMLInputElement).checked).toBe(
    true,
  );
  expect(agent).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole("radio", { name: "Añadir un proveedor" }));
  expect(screen.getByLabelText("Nombre del proveedor")).toBeTruthy();
});

it("keeps a provider refusal at its form, clears the key and offers a retry", async () => {
  const agent = vi.fn(async (call) => ({
    call,
    outcome:
      call.call === "ai_settings"
        ? { outcome: "ai_settings", settings }
        : {
            outcome: "refused",
            refusal: {
              code: "ai_key_invalid",
              status: 502,
              message: "El proveedor rechazó la clave",
              numbers: { provider_status: 401 },
            },
          },
  })) as AgentPort;
  render(<AISettings agent={agent} />);
  await screen.findByText("El audio se queda en tu hub.");
  fireEvent.click(screen.getByRole("button", { name: "Añadir un proveedor" }));
  fireEvent.change(screen.getByLabelText("Nombre del proveedor"), { target: { value: "Cedro" } });
  fireEvent.change(screen.getByLabelText("URL base"), {
    target: { value: "http://127.0.0.1:8080/v1" },
  });
  fireEvent.change(screen.getByLabelText("Clave (opcional)"), {
    target: { value: "synthetic-bad-key" },
  });
  fireEvent.click(screen.getByRole("button", { name: "Comprobar y guardar" }));
  expect((await screen.findByRole("alert")).textContent).toContain("El proveedor rechazó la clave");
  expect(screen.queryByDisplayValue("synthetic-bad-key")).toBeNull();
  expect(screen.getByRole("button", { name: "Comprobar y guardar" })).toBeTruthy();
  expect(screen.queryByText("Guardado en el hub")).toBeNull();
});

it("shows the hub's off-hub privacy line in the function row", async () => {
  const privacy = "El audio sale a Microsoft a través de OpenRouter.";
  const agent = vi.fn(async (call) => ({
    call,
    outcome: {
      outcome: "ai_settings",
      settings: { ...settings, functions: [{ ...settings.functions[0], privacy }] },
    },
  })) as AgentPort;
  render(<AISettings agent={agent} />);
  expect(await screen.findByText(privacy)).toBeTruthy();
});
