import type { VaultCall } from "../../bridge/generated/VaultCall";
import type { VaultOutcome } from "../../bridge/generated/VaultOutcome";
import { agentFailureText } from "../../i18n/es/agents";

export type VaultPort = (call: VaultCall) => Promise<VaultOutcome>;

/** Only fixed refusal copy and numeric facts are shown; exceptions never carry content. */
export async function request(port: VaultPort | undefined, call: VaultCall): Promise<VaultOutcome> {
  if (port === undefined) throw new Error("Conecta Orbit con el hub para leer tus notas.");
  let result: VaultOutcome;
  try {
    result = await port(call);
  } catch {
    throw new Error("El hub no respondió a la petición de Notas. Vuelve a intentarlo.");
  }
  if (result.outcome === "refused") {
    const numbers = Object.entries(result.numbers)
      .map(([key, value]) => `${key}: ${value}`)
      .join(", ");
    throw new Error(`${result.message} (${result.code}${numbers === "" ? "" : `; ${numbers}`})`);
  }
  if (result.outcome === "failed") throw new Error(agentFailureText(result.refusal));
  return result;
}

export function message(error: unknown): string {
  return error instanceof Error ? error.message : "No se pudo leer Notas.";
}

export function readFilter(
  operation: import("../../bridge/generated/VaultRead").VaultRead["operation"],
  path = "",
  page = 1,
  query = "",
): import("../../bridge/generated/VaultRead").VaultRead {
  return {
    operation,
    path,
    page,
    query,
    content: true,
    page_size: 50,
    state: "all",
    owner: "",
    today: "",
  };
}
