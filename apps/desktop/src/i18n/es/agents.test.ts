import { describe, expect, it } from "vitest";
import type { AgentRefusal } from "../../bridge/generated/AgentRefusal";
import { agentFailureText, agentRefusalText } from "./agents";

// Every refusal of an agent's configuration says its cause and its numbers
// (hub-v1 «Agents»). Names and values are invented.

function refusal(code: string, status: number, numbers: AgentRefusal["numbers"]): AgentRefusal {
  return { code, status, numbers };
}

describe("a hub refusal of an agent's configuration", () => {
  it.each([
    [
      refusal("agent_not_found", 404, { agent_id: "orion", agent_ids: "atlas,vega" }),
      "El hub no tiene el agente «orion»; tiene atlas,vega.",
    ],
    [
      // What the hub answers to «Borrar» on a Hermes agent's skill, while the
      // same screen edits, creates and switches that agent's skills.
      refusal("skill_delete_unsupported", 403, { agent_id: "atlas", kind: "hermes" }),
      "El panel de Hermes no puede borrar habilidades: desactívala aquí o bórrala en el servidor de Hermes.",
    ],
    [
      refusal("skill_invalid", 400, { name: "Resumen", rule: "name_lowercase" }),
      "La habilidad «Resumen» no es válida: no cumple la regla «name_lowercase».",
    ],
    [
      refusal("skill_not_found", 404, { name: "resumen-diario" }),
      "El hub no tiene la habilidad «resumen-diario».",
    ],
    [
      refusal("skill_too_large", 413, { name: "resumen-diario", bytes: 40000, max_bytes: 32768 }),
      "La habilidad «resumen-diario» ocupa 40.000 bytes; el máximo es 32.768.",
    ],
    [
      refusal("too_many_skills", 409, { count: 64, max: 64 }),
      "Este agente ya tiene 64 habilidades; el máximo es 64.",
    ],
    [
      refusal("personality_too_large", 413, { bytes: 70000, max_bytes: 65536 }),
      "La personalidad ocupa 70.000 bytes; el máximo es 65.536.",
    ],
    [
      refusal("assistant_admin_refused", 502, { route: "/api/profiles/atlas/soul", status: 403 }),
      "Hermes no aceptó el cambio: /api/profiles/atlas/soul respondió HTTP 403.",
    ],
  ])("says %o with its numbers", (given, text) => {
    expect(agentRefusalText(given)).toBe(text);
  });

  it("names an unknown code with its status and every number", () => {
    expect(agentRefusalText(refusal("agent_busy", 409, { retry_after_ms: 500, kind: "acp" }))).toBe(
      "El hub rechazó la orden (agent_busy, HTTP 409): kind: acp, retry_after_ms: 500.",
    );
  });

  it("does not invent a number the hub did not send", () => {
    expect(agentRefusalText(refusal("skill_too_large", 413, { name: "x" }))).toBe(
      "El hub rechazó la orden (skill_too_large, HTTP 413): name: x.",
    );
  });
});

describe("a call the Mac could not make", () => {
  it.each([
    [{ code: "not_paired" }, "Este Mac no está emparejado con el hub. Emparéjalo en Ajustes."],
    [
      { code: "privacy_stopped", since_ms: null },
      "Todo está parado. Reactiva Orbit para leer o cambiar los agentes.",
    ],
    [
      { code: "hub_unreachable", since_ms: null },
      "No hay conexión con el hub; los agentes se leen y se cambian en él.",
    ],
    [
      { code: "upstream_failed", service: "hub", status: 404 },
      "El hub respondió HTTP 404 sin código: puede que su versión todavía no tenga la configuración de agentes.",
    ],
    [{ code: "upstream_failed", service: "hub", status: 503 }, "El hub respondió HTTP 503."],
    [
      { code: "upstream_failed", service: "hub", status: null },
      "El hub no dio su respuesta: la conexión se cortó o no respondió a tiempo. Vuelve a leer para ver si el cambio se aplicó.",
    ],
    [
      { code: "hub_reply_invalid", field: "body", received_bytes: null },
      "La respuesta del hub no cumple el contrato (campo body).",
    ],
    [
      { code: "too_large", resource: "hub_response", bytes: 2000000, max_bytes: 1048576 },
      "La respuesta del hub ocupa 2000000 bytes; el máximo es 1048576.",
    ],
  ])("says %o", (given, text) => {
    expect(agentFailureText(given)).toBe(text);
  });
});
