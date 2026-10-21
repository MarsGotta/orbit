import type { AppCommand } from "./generated";

import type { RefusalCode } from "./generated/RefusalCode";
export type { RefusalCode } from "./generated/RefusalCode";

export interface Refusal {
  code: RefusalCode;
  retry: unknown;
  [field: string]: unknown;
}

export interface CommandError {
  command: AppCommand;
  refusal: Refusal;
}

export function commandError(error: unknown, command: AppCommand): CommandError {
  return { command, refusal: refusalOf(error) };
}

/** The typed refusal an error carries, or its transport failure. */
export function refusalOf(error: unknown): Refusal {
  if (typeof error === "object" && error !== null && "refusal" in error) {
    const refusal = error.refusal;
    if (
      typeof refusal === "object" &&
      refusal !== null &&
      "code" in refusal &&
      typeof refusal.code === "string"
    ) {
      return refusal as Refusal;
    }
  }
  const text = error instanceof Error ? error.message : String(error);
  return {
    code: "internal",
    cause: { kind: "app_operation", code: "transport", error: text },
    retry: { kind: "never" },
  };
}
