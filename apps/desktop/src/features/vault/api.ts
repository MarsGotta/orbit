import { getCopy } from "../../i18n";
import type { VaultCall } from "../../bridge/generated/VaultCall";
import type { VaultOutcome } from "../../bridge/generated/VaultOutcome";

class VaultRequestError extends Error {
  constructor(render: () => string) {
    super();
    Object.defineProperty(this, "message", { get: render });
  }
}

export type VaultPort = (call: VaultCall) => Promise<VaultOutcome>;

/** Only fixed refusal copy and numeric facts are shown; exceptions never carry content. */
export async function request(port: VaultPort | undefined, call: VaultCall): Promise<VaultOutcome> {
  if (port === undefined) throw new VaultRequestError(() => getCopy().errors.disconnectedNotes);
  let result: VaultOutcome;
  try {
    result = await port(call);
  } catch {
    throw new VaultRequestError(() => getCopy().errors.notesNoResponse);
  }
  if (result.outcome === "refused") {
    const refusal = result;
    const numbers = Object.entries(refusal.numbers)
      .map(([key, value]) => `${key}: ${value}`)
      .join(", ");
    throw new VaultRequestError(
      () =>
        `${getCopy().extras.vaultRefusalText(refusal.message_code ?? refusal.code, refusal.numbers, refusal.message_fields ?? {})} (${refusal.code}${numbers === "" ? "" : `; ${numbers}`})`,
    );
  }
  if (result.outcome === "failed")
    throw new VaultRequestError(() => getCopy().extras.agentFailureText(result.refusal));
  return result;
}

export function message(error: unknown): string {
  return error instanceof VaultRequestError ? error.message : getCopy().errors.notesReadFailed;
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
