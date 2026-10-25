import { describe, expect, it } from "vitest";
import { assistantsGearCopy as addAssistant } from "./assistantsGear";
import { agentRefusalText } from "./agents";

describe("assistants on a personal hub", () => {
  it("explains that Hermes comes from a hub profile", () => {
    expect(addAssistant.kinds.hermes?.text).toBe(
      "Se crea en tu hub a partir de un perfil de Hermes. Después aparece en Orbit.",
    );
    expect(addAssistant.askHub).toBe("Añádelo en tu hub");
    expect(addAssistant.askHubText).toBe(
      "Crea el asistente en tu hub a partir de un perfil de Hermes. No se añade desde Orbit; cuando lo añadas al hub, aparecerá aquí.",
    );
    expect(addAssistant.kinds.hermes?.text + addAssistant.askHubText).not.toContain("API Gateway");
  });

  it("keeps the exact missing-tool cause the hub supplied", () => {
    expect(
      agentRefusalText({
        code: "agent_adapter_missing",
        status: 409,
        numbers: { service: "codex", path: "/opt/example/bin/codex", cause: "no existe" },
      }),
    ).toBe(
      "Codex no está disponible en tu hub: /opt/example/bin/codex (no existe). Publica de nuevo el hub para instalarlo.",
    );
  });
});
